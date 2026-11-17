//! Toolchain discovery, validation and environment for the engine build.
//!
//! The engine compiles with `clang-cl` against Microsoft's headers, libraries
//! and Windows SDK, so the pieces that have to agree are found separately:
//! Visual Studio's C++ workload (through `vswhere`), its MSVC toolset and
//! standard library, the Windows SDK, LLVM's `clang-cl` and `lld-link`, CMake
//! and Ninja. Each is located, versioned, and then checked against the others
//! by compiling and linking one small file with exactly the environment the
//! build will use. A missing `<bit>` or an `INCLUDE` left by another install
//! therefore names the toolchain, not a header the user never wrote.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The major versions the engine needs. `cxx.h` includes `<bit>`, which 2019
/// (16) does not have, and the lifted code relies on clang 23's codegen.
pub const MIN_VS_MAJOR: u32 = 17;
pub const MIN_CLANG_MAJOR: u32 = 23;

/// Visual Studio's C++ tools, one install.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VsInstall {
    pub path: PathBuf,
    pub version: String,
    pub major: u32,
}

impl VsInstall {
    pub fn vcvars(&self) -> PathBuf {
        self.path.join("VC").join("Auxiliary").join("Build").join("vcvars64.bat")
    }
}

/// `VC\Tools\MSVC\<version>`, the headers and libraries clang-cl is pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msvc {
    pub version: String,
    pub dir: PathBuf,
}

impl Msvc {
    pub fn include(&self) -> PathBuf {
        self.dir.join("include")
    }
    /// The C++ standard library header whose absence means a C++ workload from
    /// before 2022, or one whose libraries did not install.
    pub fn has_bit(&self) -> bool {
        self.include().join("bit").is_file()
    }
}

/// One Windows 10/11 SDK, with both the headers and the x64 libraries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sdk {
    pub version: String,
    pub include: PathBuf,
    pub lib: PathBuf,
}

impl Sdk {
    pub fn has_window_header(&self) -> bool {
        self.include.join("um").join("windows.h").is_file()
    }
    pub fn has_kernel32(&self) -> bool {
        self.lib.join("um").join("x64").join("kernel32.lib").is_file()
    }
}

/// LLVM's `clang-cl` and the linker CMake picks up beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clang {
    pub exe: PathBuf,
    pub version: String,
    pub major: u32,
}

impl Clang {
    pub fn lld_link(&self) -> Option<PathBuf> {
        self.exe.parent().map(|d| d.join("lld-link.exe")).filter(|p| p.is_file())
    }
}

/// Everything the build was found to have, and everything wrong with it.
#[derive(Debug, Default, Clone)]
pub struct Toolchain {
    pub vs: Option<VsInstall>,
    pub vs_all: Vec<VsInstall>,
    pub msvc: Option<Msvc>,
    pub sdk: Option<Sdk>,
    pub clang: Option<Clang>,
    pub cmake: Option<String>,
    pub ninja: Option<String>,
    /// The environment `vcvars64.bat` produced, when it could be read. This is
    /// what the compilers will actually see, and the only trustworthy source for
    /// where the SDK and MSVC toolset are: they are not always in the folder
    /// everyone assumes, and a SDK installed on another drive is still the one
    /// the build uses.
    pub dev_env: Option<Vec<(String, String)>>,
    /// Problems that stop the build, each one an action the user can take.
    pub problems: Vec<String>,
}

impl Toolchain {
    /// One line naming every piece, for the log and for a diagnostics export.
    pub fn summary(&self) -> String {
        let vs = match &self.vs {
            Some(v) => format!("Visual Studio {} at {}", v.version, v.path.display()),
            None => "no Visual Studio C++ tools".into(),
        };
        let msvc = self.msvc.as_ref().map(|m| m.version.clone()).unwrap_or_else(|| "none".into());
        let sdk = self.sdk.as_ref().map(|s| s.version.clone()).unwrap_or_else(|| "none".into());
        let clang = match &self.clang {
            Some(c) => format!("{} ({})", c.version, c.exe.display()),
            None => "no clang-cl".into(),
        };
        let lld = self.clang.as_ref().and_then(Clang::lld_link).is_some();
        format!(
            "{vs}; MSVC toolset {msvc}; Windows SDK {sdk}; clang-cl {clang}; lld-link {}; CMake {}; Ninja {}",
            if lld { "present" } else { "missing" },
            self.cmake.as_deref().unwrap_or("missing"),
            self.ninja.as_deref().unwrap_or("missing"),
        )
    }

    /// The identity of the toolchain, so a cache made by another one can be
    /// told apart from a reusable one.
    pub fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.vs.as_ref().map(|v| v.path.display().to_string()).unwrap_or_default(),
            self.msvc.as_ref().map(|m| m.version.clone()).unwrap_or_default(),
            self.sdk.as_ref().map(|s| s.version.clone()).unwrap_or_default(),
            self.clang.as_ref().map(|c| c.exe.display().to_string()).unwrap_or_default(),
            self.clang.as_ref().map(|c| c.version.clone()).unwrap_or_default(),
        )
    }

    /// What the build needs from the environment: a paths list with the tool
    /// folders first, and `NX_*` taken from what was actually found rather than
    /// from whatever the user's shell happens to export.
    pub fn vars(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(vs) = &self.vs {
            out.push(("NX_VCVARS".to_string(), vs.vcvars().display().to_string()));
        }
        if let Some(clang) = &self.clang {
            if let Some(bin) = clang.exe.parent() {
                out.push(("NX_LLVM".to_string(), bin.display().to_string()));
                out.push(("NX_LLVM_BIN".to_string(), bin.display().to_string()));
            }
        }
        out
    }
}

/// The environment variable names whose inherited values can silently send the
/// build to a different toolkit or Python environment than the validated one.
pub const STALE: [&str; 18] = [
    "CC",
    "CXX",
    "CL",
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "VCINSTALLDIR",
    "VCToolsInstallDir",
    "WindowsSdkDir",
    "WindowsSDKVersion",
    "CMAKE_C_COMPILER",
    "CMAKE_CXX_COMPILER",
    "CMAKE_GENERATOR",
    "NX_TOOLCHAIN",
    "NX_LLVM",
    "NX_VCVARS",
    // A Python path from another install puts its extension modules (a 3.11
    // `_socket.pyd`) ahead of the engine's own and makes the build's Python
    // load `python311.dll` beside 3.13/3.14 (issue 43).
    "PYTHONHOME",
    "PYTHONPATH",
];

/// Which of those this process inherited, and their values, so the log can say
/// what was thrown away instead of silently ignoring it.
pub fn stale_env() -> Vec<(String, String)> {
    STALE
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|v| ((*name).to_string(), v)))
        .collect()
}

/// Remove the inherited overrides from a child process.
pub fn scrub(cmd: &mut Command) {
    for name in STALE {
        cmd.env_remove(name);
    }
}

pub(crate) fn hidden(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// The tools folder the launcher puts first on PATH, as the engine scripts
/// expect it.
pub fn tool_path() -> String {
    // Every version probe asks for this, and each call would run `clang-cl --version` again through `find_clang`: ten probes cost
    // seconds of a page that has to answer at once, so the answer is kept.
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE.get_or_init(build_tool_path).clone()
}

fn build_tool_path() -> String {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let mut dirs: Vec<String> = Vec::new();
    // Tools the release carries itself, first. A zip that ships clang-cl, lld-link,
    // CMake and Ninja in `tools\bin` needs none of them installed, and this is the
    // only place that has to know: every probe and every build step reads this PATH.
    if let Some(root) = crate::engine::root() {
        let carried = root.join("tools").join("bin");
        if carried.is_dir() {
            dirs.push(carried.display().to_string());
        }
    }
    if let Some(clang) = find_clang() {
        if let Some(bin) = clang.exe.parent() {
            dirs.push(bin.display().to_string());
        }
    }
    dirs.push(r"C:\Program Files\LLVM\bin".to_string());
    dirs.push(r"C:\Program Files\CMake\bin".to_string());
    // CMake bundled with Visual Studio lives here and reaches PATH only after `vcvars` runs. The dependency page runs it and the
    // build probe did not, so the page said "installed" while the build said "CMake was not found on PATH" (issue 30).
    for vs in vs_installs() {
        dirs.push(vs.path.join(r"Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin").display().to_string());
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        dirs.push(format!(r"{pf86}\CMake\bin"));
    }
    if let Ok(program_data) = std::env::var("ProgramData") {
        dirs.push(format!(r"{program_data}\chocolatey\bin"));
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        dirs.push(format!(r"{home}\scoop\shims"));
    }
    dirs.push(r"C:\Program Files\dotnet".to_string());
    dirs.push(format!(r"{local}\Microsoft\WinGet\Links"));
    dirs.push(format!(r"{local}\bin\NASM"));
    if let Ok(home) = std::env::var("USERPROFILE") {
        // `uv` and `uvx` install here, and nothing puts it on PATH for the
        // launcher's own process.
        dirs.push(format!(r"{home}\.local\bin"));
    }
    let mut path: Vec<String> = dirs.into_iter().filter(|d| Path::new(d).is_dir()).collect();
    path.push(std::env::var("PATH").unwrap_or_default());
    path.join(";")
}

fn program_files_x86() -> PathBuf {
    std::env::var("ProgramFiles(x86)").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(r"C:\Program Files (x86)"))
}

/// `vswhere.exe`, the only supported way to ask where Visual Studio is.
pub fn vswhere() -> PathBuf {
    program_files_x86().join("Microsoft Visual Studio").join("Installer").join("vswhere.exe")
}

const VC_COMPONENT: &str = "Microsoft.VisualStudio.Component.VC.Tools.x86.x64";

fn run(args: &[&str]) -> Option<(String, String)> {
    let exe = vswhere();
    if !exe.is_file() {
        return None;
    }
    let mut cmd = Command::new(&exe);
    cmd.args(args);
    let out = hidden(&mut cmd).output().ok()?;
    Some((String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// Every install that has the x64 C++ tools, newest first. `vswhere -all`
/// returns one path per line with `-format value`, which keeps this free of a
/// JSON dependency and of positional guessing between two property queries.
pub fn vs_installs() -> Vec<VsInstall> {
    let Some((stdout, _)) = run(&[
        "-products",
        "*",
        "-requires",
        VC_COMPONENT,
        "-property",
        "installationPath",
        "-format",
        "value",
        "-all",
    ]) else {
        return Vec::new();
    };
    let mut installs: Vec<VsInstall> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|path| {
            let version = run(&["-path", path, "-property", "installationVersion"])
                .map(|(v, _)| v.lines().next().unwrap_or_default().trim().to_string())
                .unwrap_or_default();
            let major = vs_major(&version).unwrap_or(0);
            let install = VsInstall { path: PathBuf::from(path), version, major };
            install.vcvars().is_file().then_some(install)
        })
        .collect();
    installs.sort_by(|a, b| (&b.version, &b.path).cmp(&(&a.version, &a.path)));
    installs
}

/// The install the build will use: the newest whose C++ tools are new enough.
pub fn vs_selected(installs: &[VsInstall]) -> Option<VsInstall> {
    installs.iter().find(|i| i.major >= MIN_VS_MAJOR).cloned()
}

/// The major version out of an `installationVersion` such as `17.9.34728.123`.
pub fn vs_major(version: &str) -> Option<u32> {
    version.split('.').next()?.trim().parse().ok()
}

/// The newest MSVC toolset inside an install. Versions sort numerically, not
/// lexicographically: `14.9` is older than `14.44`.
pub fn msvc_toolset(vs: &Path) -> Option<Msvc> {
    let root = vs.join("VC").join("Tools").join("MSVC");
    let mut found: Vec<(Vec<u32>, String)> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let parts = version_parts(&name)?;
            Some((parts, name))
        })
        .collect();
    found.sort_by(|a, b| (&b.0, &b.1).cmp(&(&a.0, &a.1)));
    found.first().map(|(_, name)| Msvc { version: name.clone(), dir: root.join(name) })
}

/// `14.44.35207` as numbers, for comparisons.
pub fn version_parts(v: &str) -> Option<Vec<u32>> {
    let parts: Vec<u32> = v.split('.').map(|p| p.parse::<u32>().ok()).collect::<Option<Vec<u32>>>()?;
    (!parts.is_empty()).then_some(parts)
}

/// The Windows SDK folders, under the standard relative paths.
pub fn sdk_candidates() -> Vec<Sdk> {
    let root = program_files_x86().join("Windows Kits").join("10");
    let includes = root.join("Include");
    let libs = root.join("Lib");
    let mut found: Vec<(Vec<u32>, String)> = std::fs::read_dir(&includes)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("10.") {
                return None;
            }
            Some((version_parts(&name)?, name))
        })
        .collect();
    found.sort_by(|a, b| (&b.0, &b.1).cmp(&(&a.0, &a.1)));
    found
        .into_iter()
        .map(|(_, version)| Sdk {
            include: includes.join(&version),
            lib: libs.join(&version),
            version,
        })
        .filter(|s| s.has_window_header() && s.has_kernel32())
        .collect()
}

/// The SDK the build will use: the newest with both headers and x64 libraries.
pub fn sdk_selected() -> Option<Sdk> {
    sdk_candidates().into_iter().next()
}

/// The MSVC toolset `vcvars` selected, out of `VCToolsInstallDir`.
pub fn msvc_from_vars(vars: &BTreeMap<String, String>) -> Option<Msvc> {
    let dir = PathBuf::from(vars.get("VCToolsInstallDir")?);
    let version = dir.file_name()?.to_string_lossy().into_owned();
    dir.join("include").join("bit").is_file().then(|| Msvc { version, dir })
}

/// The SDK `vcvars` selected, out of `WindowsSdkDir` and `WindowsSDKVersion`.
pub fn sdk_from_vars(vars: &BTreeMap<String, String>) -> Option<Sdk> {
    let root = PathBuf::from(vars.get("WindowsSdkDir")?);
    let version = vars.get("WindowsSDKVersion")?.trim_end_matches('\\').to_string();
    if version.is_empty() {
        return None;
    }
    let sdk = Sdk { include: root.join("Include").join(&version), lib: root.join("Lib").join(&version), version };
    (sdk.has_window_header() && sdk.has_kernel32()).then_some(sdk)
}

/// Where `clang-cl.exe` may be: `NX_LLVM`, `NX_TOOLCHAIN\llvm\bin`, LLVM's own
/// install folder, then PATH. The first that exists wins, and the folder found
/// is what CMake is given, so the compiler cannot drift from the one validated.
pub fn clang_candidates() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for var in ["NX_LLVM", "NX_LLVM_BIN"] {
        if let Ok(p) = std::env::var(var) {
            dirs.push(PathBuf::from(p));
        }
    }
    if let Ok(tc) = std::env::var("NX_TOOLCHAIN") {
        dirs.push(PathBuf::from(tc).join("llvm").join("bin"));
    }
    dirs.push(PathBuf::from(r"C:\Program Files\LLVM\bin"));
    if let Ok(path) = std::env::var("PATH") {
        dirs.extend(path.split(';').filter(|d| !d.trim().is_empty()).map(PathBuf::from));
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for exe in dirs.into_iter().map(|d| d.join("clang-cl.exe")).filter(|p| p.is_file()) {
        if found.iter().all(|s| !same_file(s, &exe)) {
            found.push(exe);
        }
    }
    found
}

fn same_file(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

pub fn find_clang() -> Option<Clang> {
    let exe = clang_candidates().into_iter().next()?;
    let version = clang_version(&exe).unwrap_or_default();
    let major = version_parts(&version).map(|p| p[0]).unwrap_or(0);
    Some(Clang { exe, version, major })
}

/// The first version number in `clang-cl --version`. Recent builds print
/// `clang version 23.1.4 (https://…)`; LLVM prints `LLVM version 23.1.4` for
/// some tools, so both spellings are accepted.
pub fn clang_version(exe: &Path) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version");
    let out = hidden(&mut cmd).output().ok()?;
    parse_clang_version(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_clang_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        for key in ["clang version ", "llvm version "] {
            if let Some(rest) = lower.find(key).map(|i| &line[i + key.len()..]) {
                let token = rest.split_whitespace().next()?;
                if version_parts(token).is_some() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

/// The leading numeric version of a token: `4.5.0-rc3`, `3.31.2`, and `v1.13.2` keep their numbers and lose the
/// distributor's suffix. `version_parts` alone rejects a prerelease suffix, which made an installed `4.5.0-rc3` look
/// missing while the Requirements page (whose parser already trimmed it) said installed (issue 35).
fn leading_version(token: &str) -> Option<String> {
    let token = token.trim_end_matches(',').trim_start_matches('v');
    let end = token.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(token.len());
    let head = token[..end].trim_end_matches('.');
    version_parts(head).filter(|p| p.len() >= 2).map(|_| head.to_string())
}

fn version_of(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    // The same PATH every build step gets. A CMake or Ninja in `C:\Program Files\CMake\bin`, in winget's Links folder or
    // anywhere else the launcher adds is what `deps` reports as installed, and a bare lookup here said "CMake was not
    // found on PATH" right next to that line (issue 30).
    cmd.args(args).env("PATH", tool_path());
    let out = hidden(&mut cmd).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // `cmake version 3.31.2`, `1.12.1` (ninja prints only the number), `4.5.0-rc3`
    text.lines().flat_map(|l| l.split_whitespace()).find_map(leading_version)
}

pub fn cmake_version() -> Option<String> {
    version_of("cmake", &["--version"])
}

pub fn ninja_version() -> Option<String> {
    version_of("ninja", &["--version"])
}

/// Probe the toolchain cheaply: `vswhere` plus a few version commands, no
/// `vcvars`. This is what the launcher's pages ask for, so it stays fast and
/// says nothing about the SDK, which cannot be known without the environment the
/// build gets. `init` does that part.
pub fn probe() -> Toolchain {
    let vs_all = vs_installs();
    let vs = vs_selected(&vs_all);
    let msvc = vs.as_ref().and_then(|v| msvc_toolset(&v.path));
    let sdk = sdk_selected();
    let clang = find_clang();
    let cmake = cmake_version();
    let ninja = ninja_version();

    let mut tc = Toolchain { vs, vs_all, msvc, sdk, clang, cmake, ninja, dev_env: None, problems: Vec::new() };

    if tc.vs.is_none() {
        match tc.vs_all.first() {
            Some(old) => tc.problems.push(format!(
                "Visual Studio {} at {} is too old. The engine's C++ needs 2022 (17) or newer for `<bit>`, and this is what \
                 the `'bit' file not found` build failure means.\n\nInstall the current tools with:\n{VS_INSTALL}",
                old.version,
                old.path.display(),
                VS_INSTALL = crate::engine::VS_INSTALL,
            )),
            None => tc.problems.push(format!(
                "no Visual Studio C++ tools were found. `vswhere.exe` reports no install with the x64 C++ workload ({}).\n\n\
                 Install Visual Studio 2022 Build Tools with \"Desktop development with C++\":\n{VS_INSTALL}",
                VC_COMPONENT,
                VS_INSTALL = crate::engine::VS_INSTALL,
            )),
        }
    }
    if let Some(vs) = &tc.vs {
        match &tc.msvc {
            None => tc.problems.push(format!(
                "Visual Studio at {} has no MSVC toolset (`VC\\Tools\\MSVC` is empty). The install is there but the C++ \
                 workload with its headers and libraries is not: open Visual Studio Installer, modify that install, and \
                 add \"Desktop development with C++\".",
                vs.path.display()
            )),
            Some(m) if !m.has_bit() => tc.problems.push(format!(
                "the MSVC toolset {} has no `bit` header ({}), so its C++ standard library is older than 2022 or its \
                 libraries did not install. Update the C++ workload in Visual Studio Installer, or install the current \
                 tools with:\n{VS_INSTALL}",
                m.version,
                m.include().display(),
                VS_INSTALL = crate::engine::VS_INSTALL,
            )),
            Some(_) => {}
        }
    }
    match &tc.clang {
        None => tc.problems.push(
            "clang-cl was not found. Looked in `NX_LLVM`, `NX_TOOLCHAIN\\llvm\\bin`, `C:\\Program Files\\LLVM\\bin` and \
             PATH. Install LLVM 23 or newer with `winget install LLVM.LLVM`"
                .into(),
        ),
        Some(c) if c.major < MIN_CLANG_MAJOR => tc.problems.push(format!(
            "clang-cl {} is older than {MIN_CLANG_MAJOR}: the engine's lifted code is generated for that codegen. It was \
             found at {}. Install the current one with `winget upgrade LLVM.LLVM`",
            c.version,
            c.exe.display()
        )),
        Some(c) if c.lld_link().is_none() => tc.problems.push(format!(
            "`lld-link.exe` is missing beside {}. CMake uses it as clang-cl's linker, and without it every link step fails \
             with \"linker command failed\". Reinstall LLVM so its `bin` folder is complete",
            c.exe.display()
        )),
        Some(_) => {}
    }
    if let Some(c) = &tc.cmake {
        let parts = version_parts(c).unwrap_or_default();
        if parts.len() >= 2 && (parts[0], parts[1]) < (3, 25) {
            tc.problems.push(format!(
                "CMake {c} is too old: the engine's projects ask for 3.25, and an older one reports only \"CMake Generate \
                 step failed\". Update it with `winget upgrade Kitware.CMake`"
            ));
        }
    } else {
        tc.problems.push("CMake was not found on PATH. Install it with `winget install Kitware.CMake`".into());
    }
    if tc.ninja.is_none() {
        tc.problems.push("Ninja was not found on PATH. Install it with `winget install Ninja-build.Ninja`".into());
    }
    tc
}

/// Run `vcvars64.bat` once and keep what it defines, so every child process
/// gets the same `INCLUDE`, `LIB` and `PATH` instead of inheriting whatever the
/// shell the launcher was started from had.
pub fn vcvars_env(vcvars: &Path) -> Result<BTreeMap<String, String>, String> {
    if !vcvars.is_file() {
        return Err(format!("{} is missing, so the Visual Studio environment cannot be set up", vcvars.display()));
    }
    // A temporary script, not `cmd /C "call … && set"`: cmd's quote handling
    // eats the nested quotes, and running it from a file is the one form that
    // works whatever the install path contains.
    let script = std::env::temp_dir().join(format!("emblemweave-vcvars-{}.cmd", std::process::id()));
    let body = format!("@echo off\r\ncall \"{}\" 1>nul 2>nul\r\nset\r\n", vcvars.display());
    std::fs::write(&script, body).map_err(|e| format!("writing {}: {e}", script.display()))?;
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(&script);
    let out = hidden(&mut cmd).output();
    let _ = std::fs::remove_file(&script);
    let out = out.map_err(|e| format!("running {}: {e}", vcvars.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let env = parse_set(&text);
    if !out.status.success() || !env.contains_key("INCLUDE") || !env.contains_key("LIB") {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "`vcvars64.bat` did not set up a C++ environment ({}). Its INCLUDE and LIB are what clang-cl needs; a Visual \
             Studio install without the C++ workload reports this. Open Visual Studio Installer, modify the install, and \
             add \"Desktop development with C++\".{}",
            vcvars.display(),
            if err.trim().is_empty() { String::new() } else { format!(" ({})", err.trim()) }
        ));
    }
    Ok(env)
}

/// The SDK problem, worded once: `init` reports it after reading the environment
/// Visual Studio sets, which is the only place that knows where the SDK is.
pub fn no_sdk_problem() -> String {
    format!(
        "no complete Windows SDK was found. Visual Studio's own environment names none, and {} has no version with both \
         `Include\\<version>\\um\\windows.h` and `Lib\\<version>\\um\\x64\\kernel32.lib`. An SDK on another drive is fine \
         once Visual Studio knows about it; otherwise install one through Visual Studio Installer (Individual components, \
         \"Windows 11 SDK\") or from https://developer.microsoft.com/windows/downloads/windows-sdk/",
        program_files_x86().join("Windows Kits").join("10").display()
    )
}

/// The `set` dump of a cmd session: `KEY=VALUE` lines, ignoring cmd's own
/// `=C:=C:\…` pseudo variables and any line without a key.
pub fn parse_set(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.is_empty() || key.starts_with('=') || key.contains(' ') {
            continue;
        }
        if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '(' || c == ')') {
            out.insert(key.to_string(), value.to_string());
        }
    }
    out
}

/// The variables a compiler, linker and CMake read, taken from the `vcvars`
/// dump. Everything else is left to the process.
pub const FROM_VCVARS: [&str; 8] =
    ["INCLUDE", "LIB", "LIBPATH", "PATH", "UCRTVersion", "WindowsSDKVersion", "VCToolsInstallDir", "VCINSTALLDIR"];

pub fn env_from_vcvars(env: &BTreeMap<String, String>) -> Vec<(String, String)> {
    FROM_VCVARS.iter().filter_map(|k| env.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect()
}

// ---------------------------------------------------------------------------
// Validation by building something
// ---------------------------------------------------------------------------

const PROBE_SOURCE: &str = r#"#include <bit>
#include <cstdint>
#include <string>
#include <vector>
#include <windows.h>

int main()
{
    std::string name = "probe";
    std::vector<int> values{1, 2, 3};
    const unsigned width = std::bit_width(static_cast<unsigned>(values.size()));
    return static_cast<int>(width + name.size() - GetCurrentProcessId() * 0);
}
"#;

/// Compile and link one file with the exact environment the build will use.
/// A header from another toolchain, a missing SDK import library or a
/// mismatched `LIB` all fail here in a second, naming the compiler and the file
/// it could not find, instead of twenty minutes into the recompile.
pub fn probe_build(clang: &Path, env: &[(String, String)], dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let src = dir.join("toolchain_probe.cpp");
    std::fs::write(&src, PROBE_SOURCE).map_err(|e| format!("writing {}: {e}", src.display()))?;
    let obj = dir.join("toolchain_probe.obj");
    let exe = dir.join("toolchain_probe.exe");
    let _ = std::fs::remove_file(&exe);
    let apply = |cmd: &mut Command| {
        scrub(cmd);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.current_dir(dir);
        hidden(cmd);
    };

    let mut cc = Command::new(clang);
    cc.args(["/nologo", "/std:c++20", "/EHsc", "/c", "/Fo"]).arg(&obj).arg(&src).args(["/W3", "/DNOMINMAX"]);
    apply(&mut cc);
    let out = cc.output().map_err(|e| format!("running {}: {e}", clang.display()))?;
    if !out.status.success() {
        return Err(probe_reason(clang, "compile", &out.stderr, &out.stdout, env));
    }

    let Some(lld) = clang.parent().map(|d| d.join("lld-link.exe")).filter(|p| p.is_file()) else {
        return Ok(()); // the missing linker is reported by `probe`, not here
    };
    let mut ld = Command::new(&lld);
    ld.arg("/nologo").arg("/subsystem:console").arg(format!("/out:{}", exe.display())).arg(&obj);
    apply(&mut ld);
    let out = ld.output().map_err(|e| format!("running {}: {e}", lld.display()))?;
    if !out.status.success() {
        return Err(probe_reason(&lld, "link", &out.stderr, &out.stdout, env));
    }
    Ok(())
}

fn probe_reason(tool: &Path, stage: &str, stderr: &[u8], stdout: &[u8], env: &[(String, String)]) -> String {
    let text = String::from_utf8_lossy(stderr).into_owned() + &String::from_utf8_lossy(stdout);
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.to_lowercase().contains("error") || l.to_lowercase().contains("cannot open"))
        .unwrap_or_else(|| text.lines().last().unwrap_or_default())
        .trim()
        .to_string();
    let include = env.iter().find(|(k, _)| k == "INCLUDE").map(|(_, v)| v.as_str()).unwrap_or("(none)");
    format!(
        "the toolchain cannot build the smallest test file: {stage} with {} failed.\n{line}\n\n`INCLUDE` is {include}. \
         That folder list comes from `vcvars64.bat`; if it points at an install without the C++ workload, or at an older \
         toolset than the one validated, add \"Desktop development with C++\" in Visual Studio Installer and build again.",
        tool.display()
    )
}

// ---------------------------------------------------------------------------
// CMake caches
// ---------------------------------------------------------------------------

/// The cache values that tie a build folder to one toolchain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheFacts {
    pub c_compiler: Option<String>,
    pub cxx_compiler: Option<String>,
    pub generator: Option<String>,
    pub make_program: Option<String>,
    pub toolchain_file: Option<String>,
}

/// Read the `NAME:TYPE=VALUE` lines of a `CMakeCache.txt`.
pub fn cache_facts(text: &str) -> CacheFacts {
    let mut f = CacheFacts::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let Some((name, _kind)) = key.split_once(':') else { continue };
        let value = value.trim();
        if value.is_empty() || value.ends_with("-NOTFOUND") {
            continue;
        }
        let slot = match name {
            "CMAKE_C_COMPILER" => &mut f.c_compiler,
            "CMAKE_CXX_COMPILER" => &mut f.cxx_compiler,
            "CMAKE_GENERATOR" => &mut f.generator,
            "CMAKE_MAKE_PROGRAM" => &mut f.make_program,
            "CMAKE_TOOLCHAIN_FILE" => &mut f.toolchain_file,
            _ => continue,
        };
        *slot = Some(value.to_string());
    }
    f
}

fn tidy_path(p: &str) -> String {
    // A cache keeps the spelling the configure was given: `build_game.cmd` passes `recomp\scripts\..\cmake\clang-cl.cmake` and the
    // launcher passes `recomp\cmake\clang-cl.cmake`, which name the same file. Resolve `.` and `..` textually (the file may not exist
    // yet in a folder that is being checked) so the comparison sees one path.
    let spelled = p.trim().trim_matches('"').replace('/', "\\").to_lowercase();
    let mut parts: Vec<&str> = Vec::new();
    for part in spelled.split('\\') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("\\")
}

/// Why a build folder cannot be reused: its cache names a compiler, generator
/// or toolchain file that the current one is not. `None` means it is reusable.
pub fn cache_mismatch(facts: &CacheFacts, compiler: &Path, generator: &str, toolchain_file: &str) -> Option<String> {
    let want = tidy_path(&compiler.display().to_string());
    for (label, got) in [("CMAKE_C_COMPILER", &facts.c_compiler), ("CMAKE_CXX_COMPILER", &facts.cxx_compiler)] {
        if let Some(got) = got {
            let got_path = tidy_path(got);
            // A cache may hold the bare name `clang-cl` when CMake found it on
            // PATH; that is the same compiler only if PATH still resolves there.
            let same = got_path == want || (got_path == "clang-cl" && find_clang().is_some());
            if !same {
                return Some(format!("{label} in the cache is `{got}`, and the build now uses `{}`", compiler.display()));
            }
        }
    }
    if let Some(got) = &facts.generator {
        if !got.eq_ignore_ascii_case(generator) {
            return Some(format!("the cache was generated for {got}, not {generator}"));
        }
    }
    if let Some(got) = &facts.toolchain_file {
        // The file name alone is not enough: a checkout in another folder has
        // its own `cmake\clang-cl.cmake`, and that one may name another toolchain.
        if !toolchain_file.is_empty() && tidy_path(got) != tidy_path(toolchain_file) {
            return Some(format!("the cache uses the toolchain file `{got}`, not `{toolchain_file}`"));
        }
    }
    None
}

/// Check a build folder on disk. Missing cache is `None`: CMake will create one.
pub fn cache_mismatch_in(build: &Path, compiler: &Path, generator: &str, toolchain_file: &str) -> Option<String> {
    let text = std::fs::read_to_string(build.join("CMakeCache.txt")).ok()?;
    cache_mismatch(&cache_facts(&text), compiler, generator, toolchain_file)
}

/// Remove the parts of a build folder that make CMake refuse to configure it,
/// keeping the compiled objects so a rebuild is not started from nothing.
pub fn clear_cache(build: &Path) {
    let _ = std::fs::remove_file(build.join("CMakeCache.txt"));
    let _ = std::fs::remove_dir_all(build.join("CMakeFiles"));
}

/// The environment `vcvars64.bat` produces, read once per process. The
/// Requirements page asks for it as well as the build, and running vcvars twice
/// would cost a second each time.
pub struct DevEnv {
    /// The raw `set` dump, for the values that are not passed to children
    /// (`WindowsSdkDir`, `VCToolsInstallDir`).
    pub vars: BTreeMap<String, String>,
    /// What a child process gets: `INCLUDE`, `LIB`, `LIBPATH`, `PATH` and the rest.
    pub env: Vec<(String, String)>,
}

pub fn dev_env() -> Result<&'static DevEnv, String> {
    static CACHE: OnceLock<Result<DevEnv, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let tc = probe();
            let vs = tc.vs.as_ref().ok_or_else(|| "Visual Studio's C++ tools were not found".to_string())?;
            let vars = vcvars_env(&vs.vcvars())?;
            Ok(DevEnv { env: env_from_vcvars(&vars), vars })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The toolchain the build uses, set up once per process: the probe, the
/// `vcvars` environment, and the folder `clang-cl` is in.
pub fn init() -> Result<&'static Resolved, String> {
    static ONCE: OnceLock<Result<Resolved, String>> = OnceLock::new();
    ONCE.get_or_init(resolve).as_ref().map_err(Clone::clone)
}

/// What the rest of the build needs to know about the toolchain.
pub struct Resolved {
    pub toolchain: Toolchain,
    pub env: Vec<(String, String)>,
    pub compiler: PathBuf,
}

fn resolve() -> Result<Resolved, String> {
    let mut toolchain = probe();
    if let Some(problem) = toolchain.problems.first() {
        return Err(problem.clone());
    }
    let clang = toolchain.clang.clone().ok_or("clang-cl was not found")?;
    // Now the part that needs the environment: the MSVC toolset and the SDK that
    // `vcvars64.bat` selects are what the build will actually use, and they are
    // not always in the folders a scan would guess.
    let dev = dev_env()?;
    let vars = &dev.vars;
    if let Some(m) = msvc_from_vars(vars) {
        toolchain.msvc = Some(m);
    } else if toolchain.msvc.as_ref().is_none_or(|m| !m.has_bit()) {
        return Err(toolchain
            .msvc
            .as_ref()
            .map(|m| format!("the MSVC toolset {} has no `bit` header in {}", m.version, m.include().display()))
            .unwrap_or_else(|| "Visual Studio sets up no MSVC toolset".to_string()));
    }
    match sdk_from_vars(vars).or_else(sdk_selected) {
        Some(sdk) => toolchain.sdk = Some(sdk),
        None => return Err(no_sdk_problem()),
    }
    let mut env = dev.env.clone();
    // PATH from vcvars, with the LLVM and CMake folders in front of it.
    let path = format!("{};{}", tool_path(), vars.get("PATH").cloned().unwrap_or_default());
    if let Some(slot) = env.iter_mut().find(|(k, _)| k == "PATH") {
        slot.1 = path;
    } else {
        env.push(("PATH".to_string(), path));
    }
    for (k, v) in toolchain.vars() {
        if !env.iter().any(|(name, _)| *name == k) {
            env.push((k, v));
        }
    }
    toolchain.dev_env = Some(env.clone());
    Ok(Resolved { toolchain, env, compiler: clang.exe })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toolchain_file_spelled_with_dot_dot_is_the_same_file() {
        // build_game.cmd passes `recomp\scripts\..\cmake\clang-cl.cmake`, the launcher passes `recomp\cmake\clang-cl.cmake`; the
        // cache keeps whichever ran, and a difference here threw a whole build folder away.
        let cache = concat!(
            "CMAKE_C_COMPILER:FILEPATH=C:/llvm/bin/clang-cl.exe
",
            "CMAKE_CXX_COMPILER:FILEPATH=C:/llvm/bin/clang-cl.exe
",
            "CMAKE_GENERATOR:INTERNAL=Ninja
",
            "CMAKE_TOOLCHAIN_FILE:UNINITIALIZED=C:/repo/engine/fefw-recomp/recomp/scripts/../cmake/clang-cl.cmake",
        );
        let facts = cache_facts(cache);
        let compiler = Path::new("C:/llvm/bin/clang-cl.exe");
        let got = cache_mismatch(&facts, compiler, "Ninja", "C:/repo/engine/fefw-recomp/recomp/cmake/clang-cl.cmake");
        assert_eq!(got, None);
        let other = cache_mismatch(&facts, compiler, "Ninja", "C:/other/recomp/cmake/clang-cl.cmake");
        assert!(other.is_some(), "a toolchain file in another checkout is still a mismatch");
    }

    #[test]
    fn vs_versions_compare_by_major() {
        assert_eq!(vs_major("17.9.34728.123"), Some(17));
        assert_eq!(vs_major("16.11.40.0"), Some(16));
        assert_eq!(vs_major(""), None);
    }

    #[test]
    fn clang_version_parses_both_spellings() {
        let text = "clang version 23.1.4 (https://github.com/llvm/llvm-project 1234567890)\nTarget: x86_64-pc-windows-msvc\n";
        assert_eq!(parse_clang_version(text).as_deref(), Some("23.1.4"));
        let llvm = "LLVM version 22.0.0\n";
        assert_eq!(parse_clang_version(llvm).as_deref(), Some("22.0.0"));
        assert_eq!(parse_clang_version("no version here\n"), None);
    }

    #[test]
    fn set_dump_keeps_environment_and_drops_cmd_internals() {
        let text = "=C:=C:\\work\r\nINCLUDE=C:\\VC\\include;C:\\SDK\\Include\r\nLIB=\r\nVCINSTALLDIR=C:\\VC\r\n\r\nnot a line\r\n";
        let env = parse_set(text);
        assert_eq!(env.get("INCLUDE").unwrap(), "C:\\VC\\include;C:\\SDK\\Include");
        assert_eq!(env.get("LIB").unwrap(), "");
        assert_eq!(env.get("VCINSTALLDIR").unwrap(), "C:\\VC");
        assert!(!env.contains_key("=C:"));
        assert_eq!(env.len(), 3);
    }

    #[test]
    fn vcvars_variables_are_selected_by_name() {
        let mut env = BTreeMap::new();
        env.insert("INCLUDE".to_string(), "inc".to_string());
        env.insert("LIB".to_string(), "lib".to_string());
        env.insert("LIBPATH".to_string(), "libpath".to_string());
        env.insert("ProgramFiles".to_string(), "ignored".to_string());
        let kept = env_from_vcvars(&env);
        let names: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["INCLUDE", "LIB", "LIBPATH"]);
    }

    #[test]
    fn cache_facts_ignore_notfound_and_types() {
        let text = "\
CMAKE_C_COMPILER:FILEPATH=C:/Program Files/LLVM/bin/clang-cl.exe
CMAKE_CXX_COMPILER:FILEPATH=C:/Program Files/LLVM/bin/clang-cl.exe
CMAKE_GENERATOR:INTERNAL=Ninja
CMAKE_MAKE_PROGRAM:FILEPATH=C:/ninja/ninja.exe
CMAKE_TOOLCHAIN_FILE:UNINITIALIZED=C:/engine/recomp/cmake/clang-cl.cmake
CMAKE_HOME_DIRECTORY:INTERNAL=C:/engine
SOME_MISSING:STRING=value-NOTFOUND
";
        let facts = cache_facts(text);
        assert_eq!(facts.c_compiler.as_deref(), Some("C:/Program Files/LLVM/bin/clang-cl.exe"));
        assert_eq!(facts.generator.as_deref(), Some("Ninja"));
        assert_eq!(facts.make_program.as_deref(), Some("C:/ninja/ninja.exe"));
    }

    #[test]
    fn a_cache_from_another_compiler_is_not_reused() {
        let facts = cache_facts("CMAKE_C_COMPILER:FILEPATH=C:/other/bin/clang-cl.exe\nCMAKE_GENERATOR:INTERNAL=Ninja\n");
        let compiler = Path::new("C:/Program Files/LLVM/bin/clang-cl.exe");
        let why = cache_mismatch(&facts, compiler, "Ninja", "").expect("mismatch");
        assert!(why.contains("CMAKE_C_COMPILER"), "{why}");
        assert!(why.contains("clang-cl.exe"), "{why}");
    }

    #[test]
    fn a_matching_cache_is_reused() {
        let facts = cache_facts(
            "CMAKE_C_COMPILER:FILEPATH=C:/Program Files/LLVM/bin/clang-cl.exe\n\
             CMAKE_CXX_COMPILER:FILEPATH=c:/program files/llvm/bin/CLANG-CL.EXE\n\
             CMAKE_GENERATOR:INTERNAL=Ninja\n",
        );
        let compiler = Path::new(r"C:\Program Files\LLVM\bin\clang-cl.exe");
        assert_eq!(cache_mismatch(&facts, compiler, "Ninja", ""), None);
    }

    #[test]
    fn another_generator_or_toolchain_file_is_a_mismatch() {
        let facts = cache_facts("CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n");
        let compiler = Path::new("C:/LLVM/bin/clang-cl.exe");
        assert!(cache_mismatch(&facts, compiler, "Ninja", "").unwrap().contains("Visual Studio 17 2022"));
        let facts = cache_facts("CMAKE_TOOLCHAIN_FILE:UNINITIALIZED=C:/old/cmake/clang-cl.cmake\n");
        assert!(cache_mismatch(&facts, compiler, "Ninja", "cmake/clang-cl.cmake").unwrap().contains("toolchain file"));
    }

    #[test]
    fn a_prerelease_version_is_still_a_version() {
        // CMake 4.5.0-rc3 is what the reporter of issue 35 had installed, while the build said "CMake was not found".
        assert_eq!(leading_version("4.5.0-rc3"), Some("4.5.0".into()));
        assert_eq!(leading_version("cmake"), None);
        assert_eq!(leading_version("version"), None);
        assert_eq!(leading_version("4.4.4,"), Some("4.4.4".into()));
        assert_eq!(leading_version("v1.13.2"), Some("1.13.2".into()));
    }

    #[test]
    fn versions_sort_numerically() {
        let mut names = vec!["14.9.1", "14.44.35207", "14.10.0"];
        names.sort_by_key(|v| version_parts(v).unwrap());
        assert_eq!(names, ["14.9.1", "14.10.0", "14.44.35207"]);
        assert_eq!(version_parts("14.44.35207"), Some(vec![14, 44, 35207]));
        assert_eq!(version_parts("nope"), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_child_command_cannot_inherit_another_pythons_path() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "echo %PYTHONPATH% %PYTHONHOME%"]);
        cmd.env("PYTHONPATH", r"C:\Python311\DLLs");
        cmd.env("PYTHONHOME", r"C:\Python311");
        scrub(&mut cmd);
        let out = cmd.output().expect("cmd runs");
        let said = String::from_utf8_lossy(&out.stdout);
        assert_eq!(said.trim(), "%PYTHONPATH% %PYTHONHOME%", "the child saw a Python environment (issue 43)");
    }
}
