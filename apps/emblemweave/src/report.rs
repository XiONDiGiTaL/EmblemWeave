//! One report a player can attach to an issue: what the launcher sees, the
//! settings that change behaviour, and the logs that explain a failure.
//!
//! The archive is written by the launcher itself rather than by `tar.exe`:
//! GNU tar accepts `-a` but still writes a tar archive, and a file named
//! `.zip` that is not a zip cannot be attached to an issue. If the archive
//! cannot be written, the folder is still useful and is returned.

use crate::config::Config;
use crate::widgets::Tone;
use crate::{fmt_count, fmt_size, App};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_COPY: u64 = 8 * 1024 * 1024;

fn stamp() -> String {
    // Milliseconds, not seconds: two reports made in the same second must not
    // share a folder and mix their files.
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{ms}")
}

fn printable_time() -> String {
    // UTC without a date crate: enough to tell two reports apart and match a
    // log's timestamps.
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let day = secs / 86_400;
    let s = secs % 86_400;
    format!("day {day}, {:02}:{:02}:{:02} UTC", s / 3600, s % 3600 / 60, s % 60)
}

/// Add a folder and everything under it to a real zip archive. Names use `/`
/// as the zip format requires, and an empty folder still gets its entry.
fn add_tree(zw: &mut zip::ZipWriter<std::fs::File>, dir: &Path, rel: &Path) -> std::io::Result<()> {
    let file_opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let dir_opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let child = rel.join(&name);
        let zip_name = child.to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            zw.add_directory(zip_name, dir_opts)?;
            add_tree(zw, &path, &child)?;
        } else if path.is_file() {
            zw.start_file(zip_name, file_opts)?;
            let mut file = std::fs::File::open(&path)?;
            std::io::copy(&mut file, zw)?;
        }
    }
    Ok(())
}

fn write_zip(root: &Path, zip_path: &Path, top: &str) -> Result<(), String> {
    let file = std::fs::File::create(zip_path).map_err(|e| format!("creating {}: {e}", zip_path.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    if let Err(e) = add_tree(&mut zw, root, Path::new(top)) {
        let _ = zw.finish();
        return Err(format!("writing {}: {e}", zip_path.display()));
    }
    zw.finish().map(|_| ()).map_err(|e| format!("finishing {}: {e}", zip_path.display()))
}

fn copy_file(src: &Path, dst: &Path) {
    let Ok(meta) = std::fs::metadata(src) else { return };
    if !meta.is_file() {
        return;
    }
    let _ = std::fs::create_dir_all(dst.parent().unwrap_or(dst));
    if meta.len() <= MAX_LOG_COPY {
        let _ = std::fs::copy(src, dst);
        return;
    }
    // A huge log is still useful at its end. Take the last 8 MiB so the bundle
    // stays attachable instead of silently leaving the failure out.
    let Ok(mut file) = std::fs::File::open(src) else { return };
    use std::io::{Read, Seek, SeekFrom, Write};
    if file.seek(SeekFrom::End(-(MAX_LOG_COPY as i64))).is_err() {
        return;
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return;
    }
    if let Ok(mut out) = std::fs::File::create(dst) {
        let _ = out.write_all(b"[report: this log was longer than 8 MiB; only its end is included]\n");
        let _ = out.write_all(&buf);
    }
}

fn newest_files(dir: &Path, ext: Option<&str>, limit: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.is_file() {
                return None;
            }
            if let Some(ext) = ext {
                if !p.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext)) {
                    return None;
                }
            }
            let t = e.metadata().ok().and_then(|m| m.modified().ok()).unwrap_or(UNIX_EPOCH);
            Some((t, p))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().take(limit).map(|(_, p)| p).collect()
}

fn game_summary(app: &App) -> String {
    let mut s = String::new();
    s.push_str(&format!("game data verdict: {:?}\n", app.game.worst()));
    for check in &app.game.checks {
        s.push_str(&format!("  {:?} {}: {}\n", check.severity, check.label, check.detail));
    }
    s.push_str(&format!(
        "launcher ready to play: {}\n",
        if crate::engine::ready() { "yes" } else { "no" }
    ));
    if let Some((built, now)) = crate::engine::stale_build() {
        s.push_str(&format!("build folder: made by {}, launcher is {}\n", built.unwrap_or_else(|| "an older release".into()), now));
    }
    s
}

fn runtime_summary(app: &App) -> String {
    let mut s = String::new();
    if let Some((r, ms)) = &app.last_boot {
        s.push_str(&format!("\n== runtime ({} ms) ==\n{}\n", ms, r.headline()));
        for t in &r.thread_states {
            s.push_str(&format!("{t}\n"));
        }
        for l in &r.host_log {
            s.push_str(&format!("host: {l}\n"));
        }
        for e in &r.recent {
            s.push_str(&format!("event {e}\n"));
        }
    } else {
        s.push_str("\n== runtime ==\nNo boot test this session.\n");
    }
    if let Some(r) = app.last_pipeline.as_ref().and_then(|p| p.report.as_ref()) {
        s.push_str(&format!(
            "\n== recompiler ==\n{} functions ({} complete), {:.2}% decode coverage, {} blocks lifted\n",
            fmt_count(r.lifted_functions as u64),
            fmt_count(r.complete_functions as u64),
            r.decode_coverage * 100.0,
            fmt_count(r.blocks_lifted as u64)
        ));
    }
    s
}

/// Build the report bundle. Returns the `.zip` when the archive was written,
/// otherwise the folder beside it.
pub fn bundle(app: &App, note: &str) -> Result<PathBuf, String> {
    let root = Config::dir().ok_or(crate::platform::NO_DATA_DIR)?.join("reports");
    std::fs::create_dir_all(&root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let stamp = stamp();
    let name = format!("emblemweave-report-{stamp}");
    let work = root.join(&name);
    std::fs::create_dir_all(&work).map_err(|e| format!("creating {}: {e}", work.display()))?;

    let mut text = String::new();
    text.push_str("EmblemWeave diagnostic report\n");
    text.push_str("============================\n\n");
    text.push_str(&format!("generated: {}\n", printable_time()));
    text.push_str(&format!("version: {} ({})\n", env!("CARGO_PKG_VERSION"), env!("EW_GIT_REV")));
    text.push_str(&format!("platform: {} {}\n", std::env::consts::OS, std::env::consts::ARCH));
    text.push_str(&format!(
        "data folder: {}\n",
        Config::dir().map(|p| p.display().to_string()).unwrap_or_else(|| "(not set)".into())
    ));
    text.push_str(&format!(
        "portable: {}\n",
        crate::platform::exe_dir()
            .and_then(|d| crate::platform::portable_root(&d))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "no".into())
    ));
    text.push_str(&format!(
        "logs: {}\n",
        crate::jobs::log_dir().map(|p| p.display().to_string()).unwrap_or_else(|| "(not set)".into())
    ));
    if let Some(dir) = crate::engine::run_dir() {
        text.push_str(&format!("run folder: {}\n", dir.display()));
    }
    text.push_str("\n== what the player wrote ==\n");
    text.push_str(if note.trim().is_empty() { "(no note)\n" } else { note.trim() });
    text.push_str("\n\n== game ==\n");
    text.push_str(&game_summary(app));
    text.push_str(&runtime_summary(app));

    text.push_str("\n== launcher activity ==\n");
    for a in &app.activity {
        text.push_str(&format!("{} {}\n", a.at, a.text));
    }
    text.push_str("\n== save directory reconciliation (issue #79) ==\n");
    if let Some(configured) = Config::save_dir() {
        text.push_str(&format!("configured: {}\n", configured.display()));
    } else {
        text.push_str("configured: (not set)\n");
    }
    if let Some(effective) = crate::engine::effective_save_dir() {
        text.push_str(&format!("effective (fefw.ini): {}\n", effective.display()));
        if let Some(configured) = Config::save_dir() {
            if effective == configured {
                text.push_str("status: in sync\n");
            } else {
                text.push_str("status: MISMATCH (game will use effective path)\n");
            }
        }
    } else {
        text.push_str("effective (fefw.ini): (not found)\n");
    }
    text.push_str("\n== recent settings ==\n");
    text.push_str(&format!("{:#?}\n", app.config));

    std::fs::write(work.join("report.txt"), &text).map_err(|e| format!("writing report: {e}"))?;

    if let Some(dir) = Config::dir() {
        copy_file(&dir.join("config.json"), &work.join("config.json"));
    }
    if let Some(dir) = crate::jobs::log_dir() {
        let dst = work.join("logs");
        for file in newest_files(&dir, None, 12) {
            if let Some(name) = file.file_name() {
                copy_file(&file, &dst.join(name));
            }
        }
    }
    if let Some(dir) = crate::engine::run_dir() {
        let dst = work.join("game");
        for name in ["fefw.log", "fefw.ini", "controls.ini", "fefw_freeze.txt", "fefw_fatal.txt", "fefw_crash.txt"] {
            copy_file(&dir.join(name), &dst.join(name));
        }
    }
    if crate::crash::reports_dir().is_some() {
        let dst = work.join("crash");
        for report in crate::crash::reports().into_iter().take(3) {
            let Some(name) = report.file_name() else { continue };
            let target = dst.join(name);
            let _ = std::fs::create_dir_all(&target);
            for entry in std::fs::read_dir(&report).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(file) = path.file_name() {
                        copy_file(&path, &target.join(file));
                    }
                }
            }
        }
    }
    // `diagnostics_text` carries the environment, the build-tool state and the
    // tail of the newest logs even when the files above could not be copied.
    std::fs::write(work.join("environment.txt"), crate::crash::diagnostics_text())
        .map_err(|e| format!("writing environment: {e}"))?;

    let zip = root.join(format!("{name}.zip"));
    match write_zip(&work, &zip, &name) {
        Ok(()) if zip.is_file() => {
            let _ = std::fs::remove_dir_all(&work);
            Ok(zip)
        }
        // A folder is still a usable report, and Explorer can zip it later.
        _ => {
            let _ = std::fs::remove_file(&zip);
            Ok(work)
        }
    }
}

fn encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push_str("%20"),
            b'\n' => out.push_str("%0A"),
            b'\r' => out.push_str("%0D"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// A ready-to-open GitHub issue. The body names the bundle but never puts a
/// local path in the URL; the player attaches the file from the page.
pub fn issue_url(app: &App, note: &str, report: Option<&Path>) -> String {
    let first = note.lines().find(|l| !l.trim().is_empty()).unwrap_or("EmblemWeave issue");
    let title: String = first.chars().take(90).collect();
    let mut body = String::new();
    body.push_str("### What happened\n\n");
    body.push_str(if note.trim().is_empty() { "<!-- describe what you saw, and what you expected -->" } else { note.trim() });
    body.push_str("\n\n### Build\n\n");
    body.push_str(&format!("- EmblemWeave {} ({})\n", env!("CARGO_PKG_VERSION"), env!("EW_GIT_REV")));
    body.push_str(&format!("- Windows {}", std::env::consts::ARCH));
    body.push_str(&format!(
        "- Game data: {:?}\n- Built game: {}\n",
        app.game.worst(),
        if crate::engine::ready() { "yes" } else { "no" }
    ));
    if let Some(path) = report {
        body.push_str(&format!(
            "- Diagnostic report: `{}` (created by Troubleshooter; attach the file to this issue)\n",
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "emblemweave-report".into())
        ));
    }
    body.push_str("\n### What I tried\n\n- [ ] Read Troubleshooting\n- [ ] Rebuilt the game\n- [ ] Checked the toolchain\n");
    format!(
        "https://github.com/XiONDiGiTaL/EmblemWeave/issues/new?title={}&body={}",
        encode(&format!("[Bug] {title}")),
        encode(&body)
    )
}

/// A one-line answer for the page's status area.
pub fn describe(path: &Path) -> String {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        format!("{} ({}), attach this file to the issue", path.display(), fmt_size(size))
    } else {
        format!("{} (folder; zip it before attaching)", path.display())
    }
}

/// The colour a report path uses: a zip is ready, a folder needs one more step.
pub fn tone(path: &Path) -> Tone {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        Tone::Ok
    } else {
        Tone::Warn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_archive_is_a_real_zip() {
        let root = std::env::temp_dir().join(format!("ew-report-{}-{}", std::process::id(), stamp()));
        let work = root.join("emblemweave-report-test");
        std::fs::create_dir_all(work.join("logs")).unwrap();
        std::fs::write(work.join("report.txt"), "report\n").unwrap();
        std::fs::write(work.join("logs").join("job.log"), "log\n").unwrap();
        let zip = root.join("report.zip");
        write_zip(&work, &zip, "emblemweave-report-test").unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
        assert!(archive.by_name("emblemweave-report-test/report.txt").is_ok());
        assert!(archive.by_name("emblemweave-report-test/logs/job.log").is_ok());
        drop(archive);
        let _ = std::fs::remove_dir_all(&root);
    }
}
