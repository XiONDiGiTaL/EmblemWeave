//! The dependency registry: one definition per thing the launcher needs.
//!
//! Every place that talks about a dependency reads it here: the Requirements
//! page, the Overview line, the installer script, the diagnostics report, and the
//! table in `README.md` and `docs/TROUBLESHOOTING.md` (a test keeps those two in
//! step with this list). A user is therefore never told to install something the
//! release already carries: the shader translator is checked first, and when the
//! zip carries it, the .NET SDK is not needed and says so.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a dependency is needed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Reading the dump: the ZBIC bridge.
    Decode,
    /// The launcher's own recompiler (lifted C compiled by a C compiler).
    Recompile,
    /// Building the game through the engine.
    Build,
    /// Building or running the shader translator.
    Translate,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Decode => "Decode",
            Stage::Recompile => "Recompilation",
            Stage::Build => "Build the game",
            Stage::Translate => "Shaders",
        }
    }
}

/// How a dependency is found. The matching code in `detect` is the only place
/// that knows a concrete path or command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detect {
    VsCpp,
    WindowsSdk,
    Clang,
    Cmake,
    Ninja,
    Nasm,
    Git,
    Pwsh,
    Uv,
    DotnetSdk,
    DotnetRuntime,
    Translator,
    Python,
    Zbic,
    CCompiler,
}

/// How to install a dependency, when the launcher can do more than describe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// A `winget` line, safe to run in the installer script.
    Winget(&'static str),
    /// A command that installs it inside something already present (pip).
    Command(&'static str),
    /// Wording only: the fix needs a person in a GUI.
    Manual(&'static str),
}

impl Install {
    pub fn line(self) -> &'static str {
        match self {
            Install::Winget(l) | Install::Command(l) | Install::Manual(l) => l,
        }
    }
    /// Whether the installer script may run it unattended.
    pub fn runnable(self) -> bool {
        matches!(self, Install::Winget(_) | Install::Command(_))
    }
}

/// Whether the stage needs it at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Every build needs it.
    Always,
    /// Only when the release does not carry the shader translator.
    OnlyWhenUnbundled,
    /// Nice to have: reported, but never blocks a build.
    Optional,
}

/// One dependency.
#[derive(Debug, Clone, Copy)]
pub struct Dep {
    /// Stable identifier, used by the CLI and by tests.
    pub key: &'static str,
    pub name: &'static str,
    /// The minimum version that works, as text for people; `min_parts` is the
    /// same requirement for the comparison.
    pub min: Option<&'static str>,
    pub min_parts: &'static [u32],
    pub detect: Detect,
    pub install: Option<Install>,
    pub need: Need,
    pub stages: &'static [Stage],
    /// One line: what it is for, in the words the interface shows.
    pub note: &'static str,
}

/// The registry. Order is the order the Requirements page and the docs show:
/// the toolchain first, then the build's own helpers, then what decoding and
/// the translator need.
pub static DEPS: &[Dep] = &[
    Dep {
        key: "vs",
        name: "Visual Studio C++ tools",
        min: Some("2022 (17) or newer"),
        min_parts: &[17],
        detect: Detect::VsCpp,
        install: Some(Install::Winget(crate::engine::VS_INSTALL)),
        need: Need::Always,
        stages: &[Stage::Build],
        note: "The MSVC standard library and libraries that clang-cl compiles against. Build Tools is enough; no IDE is needed.",
    },
    Dep {
        key: "sdk",
        name: "Windows 10/11 SDK",
        min: Some("10.0"),
        min_parts: &[10],
        detect: Detect::WindowsSdk,
        install: Some(Install::Manual("Add Windows 11 SDK under Individual components in the Visual Studio Installer, or install the standalone SDK, then press Check again.")),
        need: Need::Always,
        stages: &[Stage::Build],
        note: "The Windows headers and import libraries. Comes with the Visual Studio line above; an SDK on another drive is found through Visual Studio.",
    },
    Dep {
        key: "clang",
        name: "LLVM (clang-cl and lld-link)",
        min: Some("23"),
        min_parts: &[23],
        detect: Detect::Clang,
        install: Some(Install::Winget("winget install --id LLVM.LLVM --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Always,
        stages: &[Stage::Build],
        note: "Compiles the engine with Microsoft's headers; lld-link is its linker.",
    },
    Dep {
        key: "cmake",
        name: "CMake",
        min: Some("3.25"),
        min_parts: &[3, 25],
        detect: Detect::Cmake,
        install: Some(Install::Winget("winget install --id Kitware.CMake --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Always,
        stages: &[Stage::Build],
        note: "Generates the build files the engine compiles from.",
    },
    Dep {
        key: "ninja",
        name: "Ninja",
        min: None,
        min_parts: &[],
        detect: Detect::Ninja,
        install: Some(Install::Winget("winget install --id Ninja-build.Ninja --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Always,
        stages: &[Stage::Build],
        note: "Runs one compiler per core over the generated build files.",
    },
    Dep {
        key: "nasm",
        name: "NASM",
        min: Some("2.14"),
        min_parts: &[2, 14],
        detect: Detect::Nasm,
        install: Some(Install::Winget("winget install --id NASM.NASM --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Optional,
        stages: &[Stage::Build],
        note: "Assembles the AV1 decoder's x86-64 code, which cutscene movies lean on. Without it the engine builds the decoder in C only: it still decodes, 1440p playback drops frames.",
    },
    Dep {
        key: "git",
        name: "Git",
        min: None,
        min_parts: &[],
        detect: Detect::Git,
        install: Some(Install::Winget("winget install --id Git.Git --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Optional,
        stages: &[Stage::Build],
        note: "Not needed: the build downloads its pinned sources as archives (tools\\textures\\scripts\\fetch_third_party.ps1), so nothing calls Git.",
    },
    Dep {
        key: "pwsh",
        name: "PowerShell 7",
        min: Some("7"),
        min_parts: &[7],
        detect: Detect::Pwsh,
        install: Some(Install::Winget("winget install --id Microsoft.PowerShell --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Optional,
        stages: &[Stage::Build],
        note: "Runs the engine's scripts. Without it the Windows PowerShell that ships with Windows runs them; none of them needs syntax only 7 understands.",
    },
    Dep {
        key: "uv",
        name: "uv",
        min: None,
        min_parts: &[],
        detect: Detect::Uv,
        install: Some(Install::Winget("winget install --id astral-sh.uv --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Optional,
        stages: &[Stage::Build],
        note: "Not needed: without it the Python environment is made with Python and pip. Installing it only makes the first build faster.",
    },
    Dep {
        key: "dotnet-sdk",
        name: ".NET 10 SDK",
        min: Some("10"),
        min_parts: &[10],
        detect: Detect::DotnetSdk,
        install: Some(Install::Winget("winget install --id Microsoft.DotNet.SDK.10 --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::OnlyWhenUnbundled,
        stages: &[Stage::Build],
        note: "Builds the shader translator from source. Not needed when the release carries the translator, which has its own runtime.",
    },
    Dep {
        key: "dotnet-runtime",
        name: ".NET runtime",
        min: Some("10"),
        min_parts: &[10],
        detect: Detect::DotnetRuntime,
        // Nothing is ever asked for here: the bundled translator carries its
        // own runtime, and a translator built from source is published
        // self-contained. The row exists so the page can say which case applies.
        install: None,
        need: Need::Optional,
        stages: &[Stage::Translate],
        note: "Only runs a translator that was built here without being published as one executable. The bundled one carries its own runtime.",
    },
    Dep {
        key: "translator",
        name: "Shader translator",
        min: None,
        min_parts: &[],
        detect: Detect::Translator,
        install: None,
        need: Need::Always,
        stages: &[Stage::Translate],
        note: "Turns the game's shaders into SPIR-V. The release zip carries it, published with its own .NET runtime.",
    },
    Dep {
        key: "python",
        name: "Python",
        min: Some("3.9"),
        min_parts: &[3, 9],
        detect: Detect::Python,
        install: Some(Install::Winget("winget install --id Python.Python.3.12 --exact --accept-package-agreements --accept-source-agreements")),
        need: Need::Always,
        stages: &[Stage::Decode],
        note: "Runs the ZBIC bridge that decodes the game's compressed main module.",
    },
    Dep {
        key: "zbic",
        name: "zbic (Python package)",
        min: None,
        min_parts: &[],
        detect: Detect::Zbic,
        install: Some(Install::Command("py -m pip install zbic")),
        need: Need::Always,
        stages: &[Stage::Decode],
        note: "Decompresses ZBIC segments. Installed into whichever Python the bridge uses.",
    },
    Dep {
        key: "cc",
        name: "C compiler",
        min: None,
        min_parts: &[],
        detect: Detect::CCompiler,
        install: Some(Install::Command("py -m pip install ziglang")),
        need: Need::Always,
        stages: &[Stage::Recompile],
        note: "Compiles the C the launcher's own recompiler emits. zig through the ziglang wheel, zig on PATH, or LLVM's clang.",
    },
];

/// What was found for one dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Ready { version: String, path: Option<PathBuf> },
    /// Found, but older than the minimum: named with both numbers.
    TooOld { found: String, path: Option<PathBuf> },
    Missing,
    /// Not needed in this configuration, with the reason (the bundled translator).
    NotNeeded { why: &'static str },
}

/// Whether a detected path is one of the release's own copies, in `engine/fefw-recomp/tools\bin`.
fn carried(path: &Path) -> bool {
    crate::portable::bin_dir().is_some_and(|bin| path.starts_with(&bin))
}

impl Status {
    pub fn ok(&self) -> bool {
        matches!(self, Status::Ready { .. } | Status::NotNeeded { .. })
    }
    pub fn label(&self) -> &'static str {
        match self {
            // A tool the release carries itself is not "installed": it travels with the zip, its version is pinned by the
            // release, and it is found before anything on the PC (`toolchain::tool_path` puts `tools\bin` first).
            Status::Ready { path: Some(p), .. } if carried(p) => "Carried",
            Status::Ready { .. } => "Installed",
            Status::TooOld { .. } => "Too old",
            Status::Missing => "Missing",
            Status::NotNeeded { .. } => "Not needed",
        }
    }
    /// The detected version, or why there is none.
    pub fn detail(&self) -> String {
        match self {
            Status::Ready { version, path } => match path {
                Some(p) if carried(p) => format!("{version} carried in tools\\bin ({})", p.display()),
                Some(p) => format!("{version} at {}", p.display()),
                None => version.clone(),
            },
            Status::TooOld { found, path } => match path {
                Some(p) => format!("{found} at {}", p.display()),
                None => found.clone(),
            },
            Status::Missing => "not found".into(),
            Status::NotNeeded { why } => (*why).to_string(),
        }
    }
}

/// One row of the report.
#[derive(Debug, Clone)]
pub struct Row {
    pub dep: &'static Dep,
    pub status: Status,
    /// Whether this stage cannot run without it in the current configuration.
    pub required: bool,
}

impl Row {
    /// Whether this row stops a build: required and not usable.
    pub fn blocking(&self) -> bool {
        self.required && !self.status.ok()
    }
}

/// The sentence the Overview shows, e.g. "2 build dependencies are missing:
/// LLVM (clang-cl and lld-link), NASM."
pub fn summary(rows: &[Row]) -> String {
    let bad: Vec<&str> = rows.iter().filter(|r| r.blocking()).map(|r| r.dep.name).collect();
    match bad.len() {
        0 => "Every dependency the current configuration needs was found.".into(),
        1 => format!("{} needs attention before it can run.", bad[0]),
        n => format!("{n} dependencies need attention: {}.", bad.join(", ")),
    }
}

/// The rows that stop their stage.
pub fn blocking(rows: &[Row]) -> Vec<&Row> {
    rows.iter().filter(|r| r.blocking()).collect()
}

pub fn row<'a>(rows: &'a [Row], key: &str) -> Option<&'a Row> {
    rows.iter().find(|r| r.dep.key == key)
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// The PATH the build itself uses: the environment `vcvars64.bat` sets, with the
/// launcher's tool folders in front. A check made against the launcher's own
/// PATH would find compilers the build will not use.
fn search_path() -> String {
    // Exactly the PATH `toolchain::resolve` hands the build: the launcher's own
    // tool folders, then what `vcvars64.bat` set. A check made against the
    // launcher's inherited PATH would find tools the build never sees, and miss
    // the ones it does.
    let vcvars = crate::toolchain::dev_env()
        .ok()
        .and_then(|dev| dev.env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone()))
        .unwrap_or_default();
    format!("{};{vcvars}", crate::toolchain::tool_path())
}

/// PATH-style lookup. `Command` resolves `cmake` to `cmake.exe` on its own;
/// `Path::is_file` does not, so the extensions are tried here.
fn which(exe: &str) -> Option<PathBuf> {
    let mut names = vec![exe.to_string()];
    if Path::new(exe).extension().is_none() {
        for ext in ["exe", "cmd", "bat"] {
            names.push(format!("{exe}.{ext}"));
        }
    }
    search_path()
        .split(';')
        .filter(|d| !d.trim().is_empty())
        .flat_map(|d| names.iter().map(move |n| Path::new(d).join(n)))
        .find(|p| p.is_file())
}

fn run(program: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("PATH", search_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The first version-looking token in a `--version` output (`cmake version
/// 4.4.4`, `ninja 1.13.2`, `nasm 2.16`), so one parser covers those tools.
pub fn first_version(text: &str) -> Option<String> {
    text.lines()
        .flat_map(|l| l.split_whitespace())
        .filter_map(|t| {
            // `git version 2.47.1.windows.1` and `NASM version 2.16.01 compiled
            // on ...`: the leading numeric part is the version; a build suffix
            // after it is the distributor's, not comparable across installs.
            let t = t.trim_end_matches(',').trim_start_matches('v');
            let end = t.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(t.len());
            let head = t[..end].trim_end_matches('.');
            crate::toolchain::version_parts(head).map(|_| head.to_string())
        })
        .next()
}

/// Compare what was found with a minimum. An empty minimum always passes.
pub fn compare(found: &str, min: &[u32]) -> bool {
    if min.is_empty() {
        return true;
    }
    let Some(got) = crate::toolchain::version_parts(found) else { return false };
    let n = got.len().max(min.len());
    for i in 0..n {
        let a = got.get(i).copied().unwrap_or(0);
        let b = min.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    true
}

/// The .NET SDK majors in `dotnet --list-sdks`.
pub fn parse_dotnet_sdks(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|v| crate::toolchain::version_parts(v).is_some())
        .max_by(|a, b| crate::toolchain::version_parts(a).cmp(&crate::toolchain::version_parts(b)))
        .map(str::to_string)
}

/// The newest `Microsoft.NETCore.App` version in `dotnet --list-runtimes`.
pub fn parse_dotnet_runtimes(text: &str) -> Option<String> {
    text.lines()
        .filter(|l| l.starts_with("Microsoft.NETCore.App "))
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|v| crate::toolchain::version_parts(v).is_some())
        .max_by(|a, b| crate::toolchain::version_parts(a).cmp(&crate::toolchain::version_parts(b)))
        .map(str::to_string)
}

/// A Python that can run the bridge: the engine's own environment first, then
/// `python`, then `py -3`, then `python3`. `python` and `py` commonly resolve to
/// different installs, and only one of them may have `zbic`, so every candidate
/// is asked rather than the first name that answers.
#[derive(Clone)]
pub struct Py {
    pub program: PathBuf,
    pub leading: Vec<&'static str>,
    pub version: String,
}

pub fn pythons() -> Vec<Py> {
    let mut out: Vec<Py> = Vec::new();
    let mut candidates: Vec<(PathBuf, Vec<&'static str>)> = Vec::new();
    if let Some(root) = crate::engine::root() {
        // The interpreter the release carries, before any the PC has: it is the one the translator runs.
        let carried = root.join("tools").join("bin").join("python.exe");
        if carried.is_file() {
            candidates.push((carried, Vec::new()));
        }
        let venv = root.join("tools").join(".venv").join("Scripts").join("python.exe");
        if venv.is_file() {
            candidates.push((venv, Vec::new()));
        }
    }
    candidates.push((PathBuf::from("python"), Vec::new()));
    candidates.push((PathBuf::from("py"), vec!["-3"]));
    candidates.push((PathBuf::from("python3"), Vec::new()));
    for (program, leading) in candidates {
        let mut args = leading.clone();
        args.extend(["-c", "import sys;print('%d.%d.%d' % sys.version_info[:3])"]);
        let Some(text) = run(&program, &args) else { continue };
        let Some(version) = first_version(&text) else { continue };
        if out.iter().any(|p| p.version == version && p.program == program) {
            continue;
        }
        out.push(Py { program, leading, version });
    }
    out
}

fn status_for(found: Option<(String, Option<PathBuf>)>, min: &[u32]) -> Status {
    match found {
        Some((version, path)) if compare(&version, min) => Status::Ready { version, path },
        Some((version, path)) => Status::TooOld { found: version, path },
        None => Status::Missing,
    }
}

fn versioned(exe: &str, args: &[&str]) -> Option<(String, Option<PathBuf>)> {
    let path = which(exe)?;
    let text = run(&path, args)?;
    let version = first_version(&text)?;
    Some((version, Some(path)))
}

fn detect(dep: &'static Dep, bundled: Option<&Path>) -> Status {
    match dep.detect {
        Detect::VsCpp => {
            let tc = crate::toolchain::probe();
            match (&tc.vs, &tc.msvc) {
                (Some(vs), Some(msvc)) if msvc.has_bit() => Status::Ready {
                    version: format!("{} (MSVC {})", vs.version, msvc.version),
                    path: Some(vs.path.clone()),
                },
                (Some(vs), Some(msvc)) => Status::TooOld {
                    found: format!("{} with MSVC {} (no `bit` header)", vs.version, msvc.version),
                    path: Some(vs.path.clone()),
                },
                _ => match tc.vs_all.first() {
                    Some(old) => Status::TooOld { found: old.version.clone(), path: Some(old.path.clone()) },
                    None => Status::Missing,
                },
            }
        }
        Detect::WindowsSdk => {
            // The SDK the build gets is the one `vcvars` selects, which may sit
            // on another drive; the folder scan is only the fallback.
            let from_vars = crate::toolchain::dev_env()
                .ok()
                .and_then(|dev| crate::toolchain::sdk_from_vars(&dev.vars));
            match from_vars.or_else(crate::toolchain::sdk_selected) {
                Some(sdk) => Status::Ready { version: sdk.version, path: Some(sdk.include) },
                None => Status::Missing,
            }
        }
        Detect::Clang => {
            let tc = crate::toolchain::probe();
            match &tc.clang {
                Some(c) if c.major < crate::toolchain::MIN_CLANG_MAJOR => {
                    Status::TooOld { found: c.version.clone(), path: Some(c.exe.clone()) }
                }
                Some(c) if c.lld_link().is_none() => Status::TooOld {
                    found: format!("{} (no lld-link)", c.version),
                    path: Some(c.exe.clone()),
                },
                Some(c) => Status::Ready { version: c.version.clone(), path: Some(c.exe.clone()) },
                None => Status::Missing,
            }
        }
        Detect::Cmake => status_for(versioned("cmake", &["--version"]), dep.min_parts),
        Detect::Ninja => status_for(versioned("ninja", &["--version"]), dep.min_parts),
        Detect::Nasm => status_for(versioned("nasm", &["-v"]), dep.min_parts),
        Detect::Git => status_for(versioned("git", &["--version"]), dep.min_parts),
        Detect::Pwsh => status_for(versioned("pwsh", &["-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"]), dep.min_parts),
        Detect::Uv => status_for(versioned("uv", &["--version"]), dep.min_parts),
        Detect::DotnetSdk => {
            let Some(dotnet) = which("dotnet.exe") else { return Status::Missing };
            let Some(text) = run(&dotnet, &["--list-sdks"]) else {
                return Status::TooOld { found: "dotnet is present but --list-sdks failed (a runtime without the CLI)".into(), path: Some(dotnet) };
            };
            match parse_dotnet_sdks(&text) {
                Some(v) => status_for(Some((v, Some(dotnet))), dep.min_parts),
                None => Status::Missing,
            }
        }
        Detect::DotnetRuntime => {
            let Some(dotnet) = which("dotnet.exe") else { return Status::Missing };
            let text = run(&dotnet, &["--list-runtimes"]).unwrap_or_default();
            status_for(parse_dotnet_runtimes(&text).map(|v| (v, Some(dotnet))), dep.min_parts)
        }
        Detect::Translator => match bundled {
            Some(exe) => Status::Ready {
                version: "bundled with this release (carries its own .NET runtime)".into(),
                path: Some(exe.to_path_buf()),
            },
            None => match crate::engine::root() {
                Some(root) => {
                    let dll = root.join("tools").join("shaders").join("build").join("bin").join("Release").join("net10.0").join("spirv_harness.dll");
                    if dll.is_file() {
                        Status::Ready { version: "built here; runs through dotnet".into(), path: Some(dll) }
                    } else {
                        Status::Missing
                    }
                }
                None => Status::Missing,
            },
        },
        Detect::Python => {
            let all = pythons();
            // The interpreter the release carries wins over a newer one installed on the PC: the translator's packages were
            // pinned against it, and a 3.14 from the Store is not what the build was tested with.
            let best = all.iter().find(|p| carried(&p.program)).cloned().or_else(|| {
                all.into_iter().max_by(|a, b| {
                    crate::toolchain::version_parts(&a.version).cmp(&crate::toolchain::version_parts(&b.version))
                })
            });
            match best {
                Some(p) => {
                    let name = p.program.to_string_lossy().into_owned();
                    let path = which(&name).unwrap_or(p.program);
                    status_for(Some((p.version, Some(path))), dep.min_parts)
                }
                None => Status::Missing,
            }
        }
        Detect::Zbic => {
            for py in pythons() {
                let mut args = py.leading.clone();
                args.extend(["-c", "import zbic;print(getattr(zbic, '__version__', 'installed'))"]);
                if let Some(text) = run(&py.program, &args) {
                    let version = text.trim().to_string();
                    return Status::Ready {
                        version: if version.is_empty() { "installed".into() } else { version },
                        path: Some(py.program),
                    };
                }
            }
            Status::Missing
        }
        Detect::CCompiler => match crate::jobs::cc_command() {
            Some(cmd) => Status::Ready { version: cmd.join(" "), path: None },
            None => Status::Missing,
        },
    }
}

/// Whether this dependency is needed in the current configuration. A bundled
/// translator is the case that matters: it removes the .NET SDK requirement.
pub fn needed(dep: &Dep, bundled: bool) -> bool {
    match dep.need {
        Need::Always => true,
        Need::OnlyWhenUnbundled => !bundled,
        Need::Optional => false,
    }
}

/// Ask every dependency. The translator is resolved first, so the rows that
/// depend on it (the .NET SDK, the runtime) are judged with it in hand.
pub fn check() -> Vec<Row> {
    let bundled = crate::engine::bundled_harness();
    DEPS.iter()
        .map(|dep| {
            let required = needed(dep, bundled.is_some());
            let status = if dep.detect == Detect::DotnetRuntime && !required {
                Status::NotNeeded { why: "the bundled translator carries its own runtime" }
            } else if dep.detect == Detect::DotnetSdk && bundled.is_some() {
                Status::NotNeeded { why: "the release carries the shader translator, so nothing is built with .NET here" }
            } else {
                detect(dep, bundled.as_deref())
            };
            Row { dep, status, required }
        })
        .collect()
}

/// The last report, if one has been computed. The page draws from this so a
/// frame never waits for `vswhere`, `dotnet` and a `vcvars` run.
pub fn cached() -> Option<Vec<Row>> {
    CACHE.lock().ok().and_then(|c| c.clone())
}

/// Compute the report again. This is what "Check again" calls, and it is the
/// only way the page changes its mind: nothing needs a launcher restart, because
/// every detector looks in the folders a fresh install lands in.
pub fn refresh() -> Vec<Row> {
    let rows = check();
    if let Ok(mut c) = CACHE.lock() {
        *c = Some(rows.clone());
    }
    rows
}

/// Compute the report once, for the background thread that warms it at start-up.
pub fn warm() {
    let _ = refresh();
}

static CACHE: std::sync::Mutex<Option<Vec<Row>>> = std::sync::Mutex::new(None);

/// The rows that stop a particular stage.
pub fn blocking_for(rows: &[Row], stage: Stage) -> Vec<&Row> {
    rows.iter().filter(|r| r.blocking() && r.dep.stages.contains(&stage)).collect()
}

/// The install lines for everything that blocks, in registry order. Winget lines
/// and pip commands are returned; wording-only fixes are not, and the caller
/// shows them as text.
pub fn install_lines(rows: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for row in blocking(rows) {
        if let Some(install) = row.dep.install {
            let line = install.line().to_string();
            if install.runnable() && !out.iter().any(|l| *l == line) {
                out.push(line);
            }
        }
    }
    out
}

/// The changes a person has to make by hand, for the rows that block and have no
/// runnable install line.
pub fn manual_fixes(rows: &[Row]) -> Vec<String> {
    blocking(rows)
        .into_iter()
        .filter_map(|r| match r.dep.install {
            Some(Install::Manual(text)) => Some(text.to_string()),
            None => Some(format!("{}: {}", r.dep.name, r.dep.note)),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Documentation, from the same registry
// ---------------------------------------------------------------------------

/// The markdown table in `README.md` and `docs/TROUBLESHOOTING.md`. A test
/// compares those files with this text, so the docs cannot describe a different
/// set of requirements than the launcher checks.
pub fn markdown() -> String {
    let mut s = String::from("| Dependency | Needed for | Version | Missing it |\n| --- | --- | --- | --- |\n");
    for dep in DEPS {
        let needed_for = if dep.need == Need::Optional {
            format!("{} (optional)", dep.stages.iter().map(|s| s.label()).collect::<Vec<_>>().join(", "))
        } else if dep.need == Need::OnlyWhenUnbundled {
            format!("{} (only without the bundled translator)", dep.stages.iter().map(|s| s.label()).collect::<Vec<_>>().join(", "))
        } else {
            dep.stages.iter().map(|s| s.label()).collect::<Vec<_>>().join(", ")
        };
        let version = dep.min.unwrap_or("any");
        let missing = match dep.install {
            Some(i) => format!("`{}`", i.line()),
            None => dep.note.to_string(),
        };
        s.push_str(&format!("| {} | {needed_for} | {version} | {missing} |\n", dep.name));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(key: &str) -> &'static Dep {
        DEPS.iter().find(|d| d.key == key).unwrap()
    }

    /// The requirements doc and the troubleshooting guide carry this table verbatim, so a
    /// registry change that is not written into both is a failing test rather
    /// than a release whose docs describe something else.
    #[test]
    fn the_docs_tables_match_the_registry() {
        let want = markdown();
        let start = "<!-- requirements:start -->";
        let end = "<!-- requirements:end -->";
        for rel in ["../../docs/REQUIREMENTS.md", "../../docs/TROUBLESHOOTING.md"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let open = text.find(start).unwrap_or_else(|| panic!("{rel} has no {start}"));
            let close = text[open..].find(end).expect("unterminated requirements block") + open;
            // A Windows checkout with `core.autocrlf` gives this file CRLF line endings; the table generated from the registry
            // never has them, and the comparison is about content, not endings.
            let block = text[open + start.len()..close].replace("\r\n", "\n");
            let have = block.trim_start_matches(['\r', '\n']).trim_end();
            assert_eq!(
                have,
                want.trim_end(),
                "{rel} lists different requirements than the registry; run `EmblemWeave.exe --print-requirements` and put its output between the markers"
            );
        }
    }

    #[test]
    fn a_bundled_translator_removes_the_dotnet_requirement() {
        assert!(needed(dep("translator"), true), "the translator itself is always needed");
        assert!(!needed(dep("dotnet-sdk"), true), "a bundled translator must not require the SDK");
        assert!(needed(dep("dotnet-sdk"), false), "without one, the SDK builds it");
        assert!(!needed(dep("dotnet-runtime"), true), "the bundled one carries its runtime");
        assert!(!needed(dep("dotnet-runtime"), false), "a published build does not need it either");
        assert!(needed(dep("python"), true), "decoding needs Python either way");
        // Nothing in the registry can install the translator on a player's behalf,
        // so no install line may ask for it.
        assert_eq!(dep("translator").install, None);
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(compare("23.1.5", &[23]));
        assert!(!compare("22.1.8", &[23]));
        assert!(compare("10.0.28000.0", &[10]));
        assert!(!compare("3.20.5", &[3, 25]));
        assert!(compare("3.25.0", &[3, 25]));
        assert!(compare("4.4.4", &[3, 25]));
        assert!(!compare("1.9.9", &[2, 14]), "NASM 1 is older than 2.14");
        assert!(compare("anything", &[]), "no minimum means any version");
        assert!(!compare("not a version", &[1]));
    }

    #[test]
    fn the_first_version_is_read_from_the_usual_outputs() {
        assert_eq!(first_version("cmake version 4.4.4").as_deref(), Some("4.4.4"));
        assert_eq!(first_version("1.13.2").as_deref(), Some("1.13.2"));
        assert_eq!(first_version("NASM version 2.16.01 compiled on Apr 17 2023").as_deref(), Some("2.16.01"));
        assert_eq!(first_version("git version 2.47.1.windows.1").as_deref(), Some("2.47.1"));
        assert_eq!(first_version("no digits here"), None);
    }

    #[test]
    fn dotnet_sdk_and_runtime_lists_are_read_separately() {
        let sdks = "\
8.0.404 [C:\\Program Files\\dotnet\\sdk]
10.0.100 [C:\\Program Files\\dotnet\\sdk]
9.0.203 [C:\\Program Files\\dotnet\\sdk]
";
        assert_eq!(parse_dotnet_sdks(sdks).as_deref(), Some("10.0.100"));
        // A runtime-only install lists no SDKs, which is what made the old check take `dotnet.exe` for an SDK.
        assert_eq!(parse_dotnet_sdks(""), None);
        let runtimes = "\
Microsoft.AspNetCore.App 10.0.0 [C:\\Program Files\\dotnet\\shared\\Microsoft.AspNetCore.App]
Microsoft.NETCore.App 8.0.11 [C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App]
Microsoft.NETCore.App 10.0.0 [C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App]
";
        assert_eq!(parse_dotnet_runtimes(runtimes).as_deref(), Some("10.0.0"));
        assert_eq!(parse_dotnet_runtimes("Microsoft.NETCore.App 8.0.11 [x]").as_deref(), Some("8.0.11"));
    }

    #[test]
    fn a_status_says_what_is_wrong_and_what_is_installed() {
        let ready = Status::Ready { version: "4.4.4".into(), path: Some(PathBuf::from("C:\\CMake\\bin\\cmake.exe")) };
        assert!(ready.ok());
        assert_eq!(ready.label(), "Installed");
        assert!(ready.detail().contains("4.4.4"));
        let old = Status::TooOld { found: "3.20.5".into(), path: None };
        assert!(!old.ok());
        assert_eq!(old.label(), "Too old");
        let missing = Status::Missing;
        assert_eq!(missing.detail(), "not found");
        let skipped = Status::NotNeeded { why: "the release carries the shader translator" };
        assert!(skipped.ok(), "not needed must not block anything");
        assert_eq!(skipped.label(), "Not needed");
    }

    #[test]
    fn the_summary_names_the_rows_that_block_and_nothing_else() {
        let rows = vec![
            Row { dep: dep("clang"), status: Status::Missing, required: true },
            Row { dep: dep("dotnet-sdk"), status: Status::Missing, required: false },
            Row { dep: dep("cmake"), status: Status::Ready { version: "4.4.4".into(), path: None }, required: true },
        ];
        assert_eq!(blocking(&rows).len(), 1, "only the required missing row blocks");
        let text = summary(&rows);
        assert!(text.contains("LLVM (clang-cl and lld-link)"), "{text}");
        assert!(!text.contains("CMake"), "{text}");
        assert!(!text.contains(".NET"), "an unbundled row must not be reported: {text}");
    }

    #[test]
    fn the_docs_table_lists_every_dependency_once() {
        let table = markdown();
        for dep in DEPS {
            assert_eq!(table.matches(&format!("| {} |", dep.name)).count(), 1, "{} appears once", dep.name);
        }
        assert!(table.contains("only without the bundled translator"), "the .NET SDK's condition is in the table");
        assert!(table.contains("`winget install --id LLVM.LLVM"));
    }

    #[test]
    fn install_lines_are_runnable_and_deduplicated() {
        let rows = vec![
            Row { dep: dep("clang"), status: Status::Missing, required: true },
            Row { dep: dep("cmake"), status: Status::Missing, required: true },
            Row { dep: dep("zbic"), status: Status::Missing, required: true },
            Row { dep: dep("sdk"), status: Status::Missing, required: true },
            Row { dep: dep("dotnet-sdk"), status: Status::Missing, required: false },
        ];
        let lines = install_lines(&rows);
        assert!(lines.iter().any(|l| l.contains("LLVM.LLVM")), "{lines:?}");
        assert!(lines.iter().any(|l| l == "py -m pip install zbic"), "a pip command is runnable too: {lines:?}");
        assert!(!lines.iter().any(|l| l.contains("DotNet.SDK")), "an unrequired row is not installed: {lines:?}");
        // The SDK is installed through the Visual Studio Installer, so its fix is
        // wording rather than a command.
        assert!(manual_fixes(&rows).iter().any(|m| m.contains("SDK")), "{:?}", manual_fixes(&rows));
    }
}
