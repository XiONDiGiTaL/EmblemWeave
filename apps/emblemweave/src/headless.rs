//! `EmblemWeave.exe --headless <command>`: the launcher's main jobs without the window, for a front end that drives it
//! from outside (a Steam Deck / Decky plugin, a Steam shortcut, a script). Every command reads the same settings the
//! window does and runs the same code; nothing here is a second implementation of a build or of Play.
//!
//! Commands (add `--json` for one JSON object on stdout instead of the text lines):
//!
//! - `status`        what is installed and whether it can be played: launcher version, engine folder, game data, built game,
//!                   whether the build is stale, whether the game is running.
//! - `check-update`  asks GitHub for the newest launcher release (the Updates page's check).
//! - `update`        downloads and installs that release, then exits; the new launcher is not opened.
//! - `rebuild`       Set up's build of the game, with its log on stdout (exit 0 when it finished).
//! - `play`          starts the game with the saved settings and waits until it exits, so a Steam shortcut tracks it.
//!
//! Exit codes: 0 done, 1 failed, 2 nothing to do (`check-update`: no newer release; `update`: already current),
//! 64 unknown command.

use std::io::Write;
use std::time::Duration;

use crate::config::Config;
use crate::jobs::{self, JobKind, JobResult};

pub const USAGE: &str = "usage: EmblemWeave.exe --headless <status|check-update|update|rebuild|play> [--json]";

/// The value of `--headless`, when it is there.
pub fn command(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--headless")?;
    Some(args.get(i + 1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_default())
}

/// Run one command and return the exit code.
pub fn run(cmd: &str, json: bool) -> i32 {
    attach_console();
    let (config, warning) = Config::load();
    if let Some(w) = warning.filter(|_| !json) {
        eprintln!("settings: {w}");
    }
    let out = Out { json };
    match cmd {
        "status" => status(&config, &out),
        "check-update" => check_update(&out),
        "update" => update(&out),
        "rebuild" => rebuild(&config, &out),
        "play" => play(&config, &out),
        _ => {
            eprintln!("{USAGE}");
            64
        }
    }
}

/// A launcher built for the window has no console of its own; when it was started from one, print there.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

struct Out {
    json: bool,
}

impl Out {
    /// A progress line: plain text, or nothing in JSON mode (stdout then holds only the result).
    fn line(&self, text: &str) {
        if !self.json {
            println!("{text}");
            let _ = std::io::stdout().flush();
        }
    }
    /// The result: `fields` as one JSON object, or as `key: value` lines.
    fn result(&self, fields: &[(&str, Value)]) {
        if self.json {
            let body: Vec<String> = fields.iter().map(|(k, v)| format!("{}:{}", quote(k), v.json())).collect();
            println!("{{{}}}", body.join(","));
        } else {
            for (k, v) in fields {
                println!("{k}: {}", v.text());
            }
        }
        let _ = std::io::stdout().flush();
    }
}

enum Value {
    Str(String),
    Bool(bool),
    None,
}

impl Value {
    fn opt(v: Option<String>) -> Value {
        v.map(Value::Str).unwrap_or(Value::None)
    }
    fn json(&self) -> String {
        match self {
            Value::Str(s) => quote(s),
            Value::Bool(b) => b.to_string(),
            Value::None => "null".into(),
        }
    }
    fn text(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Bool(b) => if *b { "yes" } else { "no" }.into(),
            Value::None => "-".into(),
        }
    }
}

fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn status(config: &Config, out: &Out) -> i32 {
    let root = crate::engine::root();
    let built = root.as_deref().map(crate::engine::game_exe).filter(|p| p.is_file());
    let ini = crate::engine::run_dir().map(|d| d.join("fefw.ini")).filter(|p| p.is_file());
    let stale = if built.is_some() { crate::engine::stale_build().map(|(_, why)| why.to_string()) } else { None };
    let game = config.paths.game_root.trim();
    out.result(&[
        ("launcher", Value::Str(env!("CARGO_PKG_VERSION").into())),
        ("engine", Value::opt(root.map(|p| p.display().to_string()))),
        ("game_data", Value::opt((!game.is_empty()).then(|| game.to_string()))),
        ("built", Value::opt(built.as_ref().map(|p| p.display().to_string()))),
        ("ready", Value::Bool(ini.is_some() && built.is_some())),
        ("stale", Value::opt(stale)),
        ("running", Value::Bool(crate::platform::game_running())),
        ("updates_managed_elsewhere", Value::Bool(crate::updater::managed_by_recomplauncher())),
    ]);
    0
}

fn newer_release(out: &Out) -> Result<Option<crate::updater::Release>, String> {
    if crate::updater::managed_by_recomplauncher() {
        return Err("updates are managed by RecompLauncher for this install".into());
    }
    out.line("asking GitHub for the newest release...");
    let rel = crate::updater::latest()?;
    Ok(crate::updater::is_newer(&rel.tag, crate::updater::version_now()).then_some(rel))
}

fn check_update(out: &Out) -> i32 {
    match newer_release(out) {
        Ok(rel) => {
            let newer = rel.is_some();
            out.result(&[
                ("current", Value::Str(env!("CARGO_PKG_VERSION").into())),
                ("available", Value::opt(rel.as_ref().map(|r| r.tag.clone()))),
                ("page", Value::opt(rel.as_ref().map(|r| r.page.clone()))),
                ("installable", Value::Bool(rel.as_ref().is_some_and(|r| r.asset.is_some()))),
            ]);
            if newer { 0 } else { 2 }
        }
        Err(e) => fail(out, &e),
    }
}

fn update(out: &Out) -> i32 {
    let rel = match newer_release(out) {
        Ok(Some(rel)) => rel,
        Ok(None) => {
            out.result(&[("current", Value::Str(env!("CARGO_PKG_VERSION").into())), ("updated", Value::Bool(false))]);
            return 2;
        }
        Err(e) => return fail(out, &e),
    };
    if crate::platform::game_running() {
        return fail(out, "the game is running; close it before updating");
    }
    out.line(&format!("downloading EmblemWeave {}...", rel.tag));
    let dir = std::env::temp_dir().join("emblemweave-update");
    let r = crate::updater::download(&rel, &dir)
        .and_then(|exe| crate::updater::install_without_restart(&exe, crate::engine::root().as_deref(), true, &rel.tag));
    match r {
        Ok(()) => {
            // The swap waits for this process to exit; it then moves the new launcher into place.
            out.result(&[("updated", Value::Bool(true)), ("version", Value::Str(rel.tag.clone()))]);
            0
        }
        Err(e) => fail(out, &e),
    }
}

fn fail(out: &Out, why: &str) -> i32 {
    if out.json {
        out.result(&[("error", Value::Str(why.into()))]);
    } else {
        eprintln!("error: {why}");
    }
    1
}

/// Run a job to its end, printing its log as it grows. The window shows the same lines in its activity panel.
fn wait_job(job: jobs::Job, out: &Out) -> Result<Option<JobResult>, String> {
    let mut shown = 0;
    loop {
        let (finished, lines) = job.snapshot(|p| (p.finished, p.log[shown.min(p.log.len())..].to_vec()));
        shown += lines.len();
        for l in &lines {
            out.line(l);
        }
        if finished {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    job.snapshot(|p| match &p.failed {
        Some(e) => Err(match &p.log_file {
            Some(f) => format!("{e} (log: {})", f.display()),
            None => e.clone(),
        }),
        None => Ok(p.result.clone()),
    })
}

fn detect(config: &Config, out: &Out) -> crate::gamedata::GameData {
    out.line("locating the game data...");
    crate::gamedata::detect_with(&config.paths, &mut |_, _| {}, &|| false)
}

fn rebuild(config: &Config, out: &Out) -> i32 {
    if crate::platform::game_running() {
        return fail(out, "the game is running; close it before building");
    }
    let game = detect(config, out);
    let (Some(exefs), Some(romfs)) = (game.exefs.clone(), game.nup.clone().or_else(|| game.romfs.clone())) else {
        return fail(out, "no game data: choose it on Game Setup in the launcher first");
    };
    let dlc = game.dlc.clone();
    let job = jobs::spawn(JobKind::Engine, &crate::engine::STAGES, move |ctx| crate::engine::build(ctx, exefs, romfs, dlc));
    match wait_job(job, out) {
        Ok(_) => {
            out.result(&[("built", Value::Bool(true))]);
            0
        }
        Err(e) => fail(out, &e),
    }
}

fn play(config: &Config, out: &Out) -> i32 {
    if crate::platform::game_running() {
        return fail(out, "the game is already running");
    }
    // A package that keeps some files packed is prepared first, as Play in the window does (issue #24).
    if let Some(romfs) = crate::engine::packed_package_without_overlay() {
        out.line("writing the files this package keeps packed...");
        let job = jobs::spawn(JobKind::Repair, crate::engine::REPAIR_STAGES, move |ctx| crate::engine::repair(ctx, romfs));
        if let Err(e) = wait_job(job, out) {
            return fail(out, &e);
        }
    }
    if let Err(e) = crate::engine::play(config, false) {
        return fail(out, &e);
    }
    out.line("the game is starting; waiting for it to exit");
    // The game takes a moment to appear in the process list.
    let mut seen = false;
    for _ in 0..60 {
        if crate::platform::game_running() {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    while seen && crate::platform::game_running() {
        std::thread::sleep(Duration::from_secs(1));
    }
    out.result(&[("played", Value::Bool(seen))]);
    if seen { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_follows_the_switch() {
        assert_eq!(command(&args(&["EmblemWeave.exe", "--headless", "status", "--json"])), Some("status".into()));
        assert_eq!(command(&args(&["EmblemWeave.exe", "--headless", "--json"])), Some(String::new()));
        assert_eq!(command(&args(&["EmblemWeave.exe", "--page", "Overview"])), None);
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(quote(r#"C:\Games\"FE"	x"#), r#""C:\\Games\\\"FE\"\tx""#);
        assert_eq!(Value::None.json(), "null");
        assert_eq!(Value::Bool(true).text(), "yes");
    }
}
