//! The portable toolchain: clang-cl + lld-link, CMake, Ninja and a relocatable Python, fetched into
//! `engine/fefw-recomp/tools/bin`.
//!
//! `toolchain::tool_path` already puts that folder first on the PATH every probe and build step reads, so a piece that
//! lands there is found before anything installed on the PC, and the Requirements page reports it as carried rather than
//! installed. What is left is the download: one table of artifacts with their URLs, sizes and SHA-256, a curl per piece
//! with progress taken from the file's own size, a hash check that refuses a damaged file, and an unpack through the
//! `tar.exe` Windows ships (it reads zip) or the built-in zstd/tar stack for LLVM.
//!
//! Only the files the build actually runs are installed. LLVM's release archive holds every tool and static library
//! (about 3.4 GB unpacked); the compiler, linker, resource compiler, manifest tool and clang's resource headers are a
//! tenth of that. The 901 MB `.tar.xz` is also replaced by LLVM's 264 MB `.tar.zst`, so a first run downloads less and
//! leaves less behind.
//!
//! Visual Studio's C++ tools and the Windows SDK are not here on purpose: those headers and import libraries are
//! Microsoft's and are not ours to redistribute (`docs/INSTALL.md`).

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// One archive the release carries.
#[derive(Debug, Clone, Copy)]
pub struct Piece {
    /// Stable identifier: the stamp file and the CLI use it.
    pub key: &'static str,
    /// What the Requirements row is called.
    pub name: &'static str,
    /// The download's file name.
    pub file: &'static str,
    pub url: &'static str,
    /// SHA-256 of the archive, pinned here: a truncated or substituted download is refused, not unpacked.
    pub sha256: &'static str,
    /// The archive's size, for the progress bar before the server answers.
    pub bytes: u64,
    /// The executable inside `tools/bin` that proves the piece is in place.
    pub exe: &'static str,
    /// The version the piece reports once it runs.
    pub version: &'static str,
    /// Files and folders to take from a `.tar.zst` archive, relative to its top folder. Empty means everything. The
    /// small zip pieces are extracted whole.
    pub keep: &'static [&'static str],
}

/// The LLVM release has a tool for every job. These are the ones the engine's CMake projects invoke, plus clang's
/// built-in resource headers. `clang.exe` is the C compiler the launcher's own recompiler may use when no `zig` is
/// present; `llvm-rc` is the resource compiler the runtime's `.rc` file needs.
const CLANG_KEEP: &[&str] = &[
    "bin/clang-cl.exe",
    "bin/clang.exe",
    "bin/lld-link.exe",
    "bin/llvm-lib.exe",
    "bin/llvm-rc.exe",
    "bin/llvm-mt.exe",
    "bin/vcruntime140.dll",
    "bin/vcruntime140_1.dll",
    "lib/clang/23/include/",
];

/// The four pieces, at the versions the engine builds against. LLVM 23 is the release `MIN_CLANG_MAJOR` asks for;
/// CMake 3.31 is past the 3.25 the projects require; the 3.12 embeddable Python is the interpreter the translator runs.
pub const PIECES: &[Piece] = &[
    Piece {
        key: "clang",
        name: "LLVM (clang-cl, lld-link and llvm-rc)",
        file: "clang+llvm-23.1.5-x86_64-pc-windows-msvc.tar.zst",
        url: "https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/clang%2Bllvm-23.1.5-x86_64-pc-windows-msvc.tar.zst",
        sha256: "b2be33385ddc86e83511417a060de927ae8b09d1f3017cc3cdce5c73cd5029d7",
        bytes: 276368249,
        exe: "clang-cl.exe",
        version: "23.1.5",
        keep: CLANG_KEEP,
    },
    Piece {
        key: "cmake",
        name: "CMake",
        file: "cmake-3.31.6-windows-x86_64.zip",
        url: "https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-windows-x86_64.zip",
        sha256: "d163cd3ab4959b0a53fa8988f2ddbd2e6c501658201e6a154386bad9dbe4f836",
        bytes: 46473549,
        exe: "cmake.exe",
        version: "3.31.6",
        keep: &[],
    },
    Piece {
        key: "ninja",
        name: "Ninja",
        file: "ninja-win.zip",
        url: "https://github.com/ninja-build/ninja/releases/download/v1.13.1/ninja-win.zip",
        sha256: "26a40fa8595694dec2fad4911e62d29e10525d2133c9a4230b66397774ae25bf",
        bytes: 289808,
        exe: "ninja.exe",
        version: "1.13.1",
        keep: &[],
    },
    Piece {
        key: "python",
        name: "Python",
        file: "python-3.12.8-embed-amd64.zip",
        url: "https://www.python.org/ftp/python/3.12.8/python-3.12.8-embed-amd64.zip",
        sha256: "8d3f33be9eb810f23c102f08475af2854e50484b8e4e06275e937be61ce3d2fb",
        bytes: 11094114,
        exe: "python.exe",
        version: "3.12.8",
        keep: &[],
    },
];

const STAMP_PREFIX: &str = ".carried-";

/// `engine/fefw-recomp/tools`: next to the engine the launcher runs.
pub fn dir() -> Option<PathBuf> {
    crate::engine::root().map(|r| r.join("tools"))
}

/// `engine/fefw-recomp/tools/bin`: the one folder `toolchain::tool_path` puts first.
pub fn bin_dir() -> Option<PathBuf> {
    dir().map(|d| d.join("bin"))
}

/// The files and folders under `tools` this piece installs that are not there. The stamp only records that the archive was
/// unpacked once: an antivirus quarantine removes individual files and leaves the stamp, so checking `piece.exe` alone
/// called a toolchain with no `lld-link.exe` or `llvm-rc.exe` carried, and the build failed at its first link or resource
/// compile instead (issue #116).
fn missing_in(tools: &Path, piece: &Piece) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let exe = tools.join("bin").join(piece.exe);
    if !exe.is_file() {
        missing.push(exe);
    }
    for rel in piece.keep {
        let path = tools.join(rel);
        let present = if rel.ends_with('/') { path.is_dir() } else { path.is_file() };
        if !present && !missing.contains(&path) {
            missing.push(path);
        }
    }
    missing
}

/// Whether this piece is in place: its stamp matches the archive, and every file the piece installs is still there.
fn carried_in(tools: &Path, piece: &Piece) -> bool {
    missing_in(tools, piece).is_empty()
        && fs::read_to_string(tools.join("bin").join(format!("{STAMP_PREFIX}{}", piece.key)))
            .map(|s| s.trim() == piece.sha256)
            .unwrap_or(false)
}

/// Whether this piece is already in place, from the stamp the installer writes and the files it unpacked.
pub fn carried(piece: &Piece) -> bool {
    dir().is_some_and(|tools| carried_in(&tools, piece))
}

/// One carried piece whose files are partly gone: enough to report on Overview and repair with one press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Damage {
    pub name: &'static str,
    pub missing: Vec<PathBuf>,
}

impl Damage {
    /// `LLVM (clang-cl, lld-link and llvm-rc): lld-link.exe` for a one-line report.
    pub fn summary(&self) -> String {
        let files = self
            .missing
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}: {files}", self.name)
    }
}

/// The carried pieces whose stamp is still there but whose files are partly gone, with the missing paths. A piece that
/// was never carried is left to the first-run fetch, which installs it without being asked.
fn damage_in(tools: &Path) -> Vec<Damage> {
    let bin = tools.join("bin");
    PIECES
        .iter()
        .filter(|piece| bin.join(format!("{STAMP_PREFIX}{}", piece.key)).is_file())
        .filter_map(|piece| {
            let missing = missing_in(tools, piece);
            (!missing.is_empty()).then_some(Damage { name: piece.name, missing })
        })
        .collect()
}

/// The pieces a repair has to fetch again, in the same order as [`PIECES`].
fn damaged_in(tools: &Path) -> Vec<&'static Piece> {
    let bin = tools.join("bin");
    PIECES
        .iter()
        .filter(|piece| bin.join(format!("{STAMP_PREFIX}{}", piece.key)).is_file())
        .filter(|piece| !missing_in(tools, piece).is_empty())
        .collect()
}

/// The damage report for the engine this launcher runs, for the Overview. Empty when every carried piece is whole.
pub fn damage() -> Vec<Damage> {
    dir().map(|tools| damage_in(&tools)).unwrap_or_default()
}

/// Names the carried pieces whose files are partly gone, with the way back. None when every carried piece is whole.
fn damage_note_in(tools: &Path) -> Option<String> {
    let damaged = damage_in(tools);
    if damaged.is_empty() {
        return None;
    }
    let lines = damaged.iter().map(|d| format!("  {}", d.summary())).collect::<Vec<_>>();
    Some(format!(
        "files the toolchain EmblemWeave carried into tools\\bin needs are missing:\n{}\n\nAn antivirus quarantine removes \
         files like these and leaves the install stamp behind, so the build would fail at its first link or resource compile. \
         Put them back with Repair on Overview (or `EmblemWeave.exe --fetch-tools`), or restore them from the antivirus \
         quarantine. Then press Rebuild.",
        lines.join("\n")
    ))
}

/// [`damage_note_in`] for the engine this launcher runs.
pub fn damage_note() -> Option<String> {
    dir().and_then(|tools| damage_note_in(&tools))
}

/// Whether the PC already has a tool this piece would supply. Checked in the background before the launcher downloads
/// anything: a player with LLVM 23, CMake, Ninja and Python installed gets no duplicate copy and no download.
fn system_ready(piece: &Piece) -> bool {
    match piece.key {
        "clang" => crate::toolchain::clang_candidates().into_iter().any(|exe| {
            let major = crate::toolchain::clang_version(&exe)
                .as_deref()
                .and_then(crate::toolchain::version_parts)
                .and_then(|parts| parts.first().copied())
                .unwrap_or(0);
            major >= crate::toolchain::MIN_CLANG_MAJOR
                && exe.parent().is_some_and(|bin| {
                    ["clang.exe", "lld-link.exe", "llvm-lib.exe", "llvm-rc.exe", "llvm-mt.exe"]
                        .iter()
                        .all(|tool| bin.join(tool).is_file())
                })
        }),
        "cmake" => crate::toolchain::cmake_version()
            .is_some_and(|version| crate::deps::compare(&version, &[3, 25])),
        "ninja" => crate::toolchain::ninja_version().is_some(),
        "python" => crate::deps::pythons()
            .iter()
            .any(|python| crate::deps::compare(&python.version, &[3, 9])),
        _ => false,
    }
}

/// Downloads, checks and unpacks one piece. `progress` is told the fraction (0..1) and a line to show; it is called from
/// the downloading thread, so the UI can keep drawing.
pub fn install(piece: &'static Piece, progress: &dyn Fn(f32, String)) -> Result<(), String> {
    if carried(piece) {
        progress(1.0, format!("{} {} is already carried", piece.name, piece.version));
        return Ok(());
    }
    let tools = dir().ok_or("the engine folder engine/fefw-recomp was not found")?;
    let downloads = tools.join("download");
    let bin = tools.join("bin");
    fs::create_dir_all(&downloads).map_err(|e| format!("creating {}: {e}", downloads.display()))?;
    fs::create_dir_all(&bin).map_err(|e| format!("creating {}: {e}", bin.display()))?;

    let archive = downloads.join(piece.file);
    let good = archive.is_file() && sha256_file(&archive)? == piece.sha256;
    if !good {
        let _ = fs::remove_file(&archive);
        download(piece, &archive, progress)?;
        let got = sha256_file(&archive)?;
        if got != piece.sha256 {
            let _ = fs::remove_file(&archive);
            return Err(format!(
                "{} did not match its hash: expected {}, got {got}. The download was damaged or replaced; try again.",
                piece.file, piece.sha256
            ));
        }
    } else {
        progress(0.5, format!("{} is already downloaded", piece.file));
    }

    progress(0.9, format!("unpacking {} into tools\\bin", piece.file));
    let unpack = tools.join("unpack").join(piece.key);
    let _ = fs::remove_dir_all(&unpack);
    fs::create_dir_all(&unpack).map_err(|e| format!("creating {}: {e}", unpack.display()))?;
    if piece.file.ends_with(".tar.zst") {
        unpack_zstd(piece, &archive, &unpack)?;
    } else {
        let out = hidden(&mut Command::new(system_tool("tar.exe")))
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&unpack)
            .output()
            .map_err(|e| format!("running tar: {e}"))?;
        if !out.status.success() {
            return Err(format!("{} could not be unpacked: {}", piece.file, String::from_utf8_lossy(&out.stderr).trim()));
        }
    }

    let exe = find_file(&unpack, piece.exe, 5).ok_or_else(|| format!("{} holds no {}", piece.file, piece.exe))?;
    let source = exe.parent().ok_or("the archive's executable has no folder")?;
    merge_into(source, &bin)?;
    // A toolchain's `bin` sits beside the folders it needs at run time: LLVM's `lib/clang/<version>` headers and CMake's
    // `share/cmake-<version>` modules. They are carried beside `tools/bin` so the executables find them unchanged.
    if source.file_name().is_some_and(|n| n.eq_ignore_ascii_case("bin")) {
        if let Some(root) = source.parent() {
            for sibling in ["lib", "share", "include"] {
                let from = root.join(sibling);
                if from.is_dir() {
                    merge_into(&from, &tools.join(sibling))?;
                }
            }
        }
    }
    let _ = fs::remove_dir_all(&unpack);

    fs::write(bin.join(format!("{STAMP_PREFIX}{}", piece.key)), format!("{}\n", piece.sha256))
        .map_err(|e| format!("writing the stamp: {e}"))?;
    // The archive has been checked and unpacked: keeping it only costs a second copy of the download. A failed install
    // keeps it so the next attempt does not download again.
    let _ = fs::remove_file(&archive);
    progress(1.0, format!("{} {} is carried", piece.name, piece.version));
    Ok(())
}

/// A subset of the pieces, for the background first-run fetch. The CLI's `--fetch-tools` uses [`install_all`].
fn install_selected(pieces: &[&'static Piece], progress: &dyn Fn(f32, String)) -> Result<(), String> {
    let total = pieces.len().max(1) as f32;
    for (i, piece) in pieces.iter().enumerate() {
        let base = i as f32 / total;
        install(piece, &|f, line| progress((base + f / total).min(1.0), line))?;
    }
    Ok(())
}

/// Every piece, in order. Like the background first-run path, this trims an existing full LLVM install even when all the
/// pieces are already carried, then removes the archives a successful install leaves behind.
pub fn install_all(progress: &dyn Fn(f32, String)) -> Result<(), String> {
    let pieces: Vec<&'static Piece> = PIECES.iter().collect();
    trim_after_fetch(install_selected(&pieces, progress))
}

/// Trim whatever an older full LLVM install left behind, then clear the download cache only when every piece is in
/// place. A failed fetch keeps its archive so the next attempt does not download it again.
fn trim_after_fetch(r: Result<(), String>) -> Result<(), String> {
    tidy();
    if r.is_ok() {
        clean_downloads();
    }
    r
}

/// Re-fetch the carried pieces whose files are partly gone. Only those pieces are downloaded; a whole toolchain returns
/// without touching the network. `progress` covers the selected pieces, 0..1. This is what Overview's Repair runs, so a
/// player no longer needs `EmblemWeave.exe --fetch-tools` after an antivirus quarantine.
pub fn repair_damaged(progress: &dyn Fn(f32, String)) -> Result<(), String> {
    let tools = dir().ok_or("the engine folder engine/fefw-recomp was not found")?;
    let damaged = damaged_in(&tools);
    if damaged.is_empty() {
        progress(1.0, "The carried toolchain is complete".into());
        return Ok(());
    }
    trim_after_fetch(install_selected(&damaged, progress))
}

/// Starts the first-run fetch in the background, once. Only the pieces the PC does not already have are downloaded.
/// Progress goes to `tools/fetch.log` so it can be watched, and the Requirements page picks the pieces up as Carried on
/// its next check.
pub fn auto_start() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let wanted: Vec<&'static Piece> = PIECES
            .iter()
            .filter(|piece| !carried(piece) && !system_ready(piece))
            .collect();
        if wanted.is_empty() {
            let _ = trim_after_fetch(Ok(()));
            return;
        }
        use std::io::Write;
        let log = std::cell::RefCell::new(dir().and_then(|d| fs::File::create(d.join("fetch.log")).ok()));
        let r = install_selected(&wanted, &|frac, line| {
            if let Some(f) = log.borrow_mut().as_mut() {
                let _ = writeln!(f, "{:>3.0}% {line}", frac * 100.0);
                let _ = f.flush();
            }
        });
        if let Err(e) = &r {
            if let Some(f) = log.borrow_mut().as_mut() {
                let _ = writeln!(f, "failed: {e}");
            }
        }
        let _ = trim_after_fetch(r);
    });
}

/// Unpacks the LLVM `.tar.zst`, taking only the files in `piece.keep`. `zstd` needs `window_log_max(31)` because the
/// LLVM release is compressed with a 1 GiB window; the decoder is streamed into the tar reader, so the 3.4 GB archive
/// never lands on disk.
fn unpack_zstd(piece: &Piece, archive: &Path, unpack: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| format!("reading {}: {e}", archive.display()))?;
    let mut decoder = zstd::stream::read::Decoder::new(BufReader::new(file))
        .map_err(|e| format!("opening {} as zstd: {e}", archive.display()))?;
    decoder
        .window_log_max(31)
        .map_err(|e| format!("setting the zstd window: {e}"))?;
    let mut tar = tar::Archive::new(decoder);
    let entries = tar.entries().map_err(|e| format!("reading {}: {e}", archive.display()))?;
    let mut files = 0usize;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("reading {}: {e}", archive.display()))?;
        let path = entry.path().map_err(|e| format!("reading a path in {}: {e}", archive.display()))?.into_owned();
        let Some(rel) = archive_relative(&path) else { continue };
        if !kept(piece, &rel) || !entry.header().entry_type().is_file() {
            continue;
        }
        let dest = unpack.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        entry.unpack(&dest).map_err(|e| format!("unpacking {}: {e}", dest.display()))?;
        files += 1;
    }
    if files == 0 {
        return Err(format!("{} held none of the files the build uses", piece.file));
    }
    Ok(())
}

/// The path inside the archive with its single top folder removed. Absolute paths, parent components and empty paths
/// are refused, so an archive cannot write outside the unpack folder.
fn archive_relative(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    match components.next()? {
        Component::Normal(_) => {}
        _ => return None,
    }
    let mut rel = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(part) => rel.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!rel.as_os_str().is_empty()).then_some(rel)
}

/// Whether a relative archive path is one of the files this piece keeps.
fn kept(piece: &Piece, rel: &Path) -> bool {
    if piece.keep.is_empty() {
        return true;
    }
    let text = rel.to_string_lossy().replace('\\', "/");
    piece.keep.iter().any(|entry| {
        if let Some(dir) = entry.strip_suffix('/') {
            text.get(..dir.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(dir))
                && text.as_bytes().get(dir.len()) == Some(&b'/')
        } else {
            text.eq_ignore_ascii_case(entry)
        }
    })
}

/// Removes what the old, full LLVM archive left in `tools`, plus an interrupted unpack folder. It leaves CMake, Python
/// and Ninja alone: those share `tools/bin` with LLVM, so only LLVM's own tool names are removed. Download cleanup belongs
/// to the fetch that knows the result, not here.
fn tidy() {
    let Some(tools) = dir() else { return };
    let _ = fs::remove_dir_all(tools.join("unpack"));
    let bin = tools.join("bin");
    if bin.join("clang-cl.exe").is_file() {
        trim_include(&tools.join("include"));
        trim_lib(&tools.join("lib"));
        trim_share(&tools.join("share"));
        trim_bin(&bin);
    }
}

fn trim_include(include: &Path) {
    let Ok(entries) = fs::read_dir(include) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if ["clang", "clang-c", "llvm", "llvm-c", "lld", "lldb", "mlir", "polly", "profile"]
            .iter()
            .any(|dir| name == *dir)
        {
            remove_path(&entry.path());
        }
    }
}

fn trim_lib(lib: &Path) {
    let Ok(entries) = fs::read_dir(lib) else { return };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().eq_ignore_ascii_case("clang") {
            remove_path(&entry.path());
        }
    }
}

fn trim_share(share: &Path) {
    let Ok(entries) = fs::read_dir(share) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().starts_with("cmake") {
            remove_path(&entry.path());
        }
    }
}

fn trim_bin(bin: &Path) {
    let Ok(entries) = fs::read_dir(bin) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if (is_llvm_tool(&name) && !bin_kept(&name)) || is_llvm_library(&name) {
            remove_path(&entry.path());
        }
    }
}

fn bin_kept(name: &str) -> bool {
    CLANG_KEEP
        .iter()
        .filter_map(|path| path.strip_prefix("bin/"))
        .any(|kept| kept.eq_ignore_ascii_case(name))
}

fn is_llvm_tool(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("clang")
        || name.starts_with("lld")
        || name.starts_with("llvm")
        || name.starts_with("ld.")
        || name.starts_with("ld64")
        || matches!(
            name.as_str(),
            "wasm-ld.exe"
                | "dsymutil.exe"
                | "opt.exe"
                | "llc.exe"
                | "bugpoint.exe"
                | "c-index-test.exe"
                | "diagtool.exe"
                | "find-all-symbols.exe"
                | "git-clang-format"
                | "hmaptool.exe"
                | "modularize.exe"
                | "scan-build"
                | "scan-view"
                | "sancov.exe"
                | "sanitizer-symbolizer.exe"
                | "split-file.exe"
                | "verify-uselistorder.exe"
                | "yaml2obj.exe"
                | "obj2yaml.exe"
                | "pp-trace.exe"
                | "lli.exe"
                | "llubi.exe"
                | "sanstats.exe"
                | "offload-arch.exe"
                | "nvptx-arch.exe"
        )
}

fn is_llvm_library(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "libclang.dll" | "libclang-cpp.dll" | "lto.dll" | "llvm-c.dll" | "remarks.dll"
    )
}

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

fn clean_downloads() {
    let Some(tools) = dir() else { return };
    let downloads = tools.join("download");
    let Ok(entries) = fs::read_dir(&downloads) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        if [".zip", ".xz", ".zst"].iter().any(|ext| name.ends_with(ext)) {
            let _ = fs::remove_file(path);
        }
    }
    let _ = fs::remove_dir(&downloads);
}

fn download(piece: &Piece, to: &Path, progress: &dyn Fn(f32, String)) -> Result<(), String> {
    if !piece.url.starts_with("https://") {
        return Err("the download address is not https".into());
    }
    let mut child = hidden(&mut Command::new(system_tool("curl.exe")))
        .args(["-sSfL", "--retry", "2", "--max-time", "1800", "-A", concat!("EmblemWeave/", env!("CARGO_PKG_VERSION")), "-o"])
        .arg(to)
        .arg(piece.url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("running curl: {e}"))?;
    // curl writes the file as it arrives, so the bytes on disk are the progress: no parsing of a terminal bar, and it
    // works the same whether the server sent Content-Length or not.
    let mut last = 0u64;
    loop {
        let done = fs::metadata(to).map(|m| m.len()).unwrap_or(0);
        if done != last {
            last = done;
            let frac = if piece.bytes > 0 { (done as f32 / piece.bytes as f32).min(0.99) } else { 0.0 };
            progress(frac, format!("downloading {} — {:.0} of {:.0} MB", piece.file, done as f64 / 1e6, piece.bytes as f64 / 1e6));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return Err(format!("downloading {} failed: {}", piece.file, err.trim()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(200)),
            Err(e) => return Err(format!("waiting for curl: {e}")),
        }
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("reading {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn find_file(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                if let Some(f) = find_file(&p, name, depth - 1) {
                    return Some(f);
                }
            }
        } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)) {
            return Some(p);
        }
    }
    None
}

/// Moves everything in `from` into `to`, keeping subfolders. A hard link is instant and costs no space on one volume,
/// which matters for LLVM's compiler executables; a copy is the fallback.
fn merge_into(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    for e in fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))?.flatten() {
        let src = e.path();
        let dst = to.join(e.file_name());
        if src.is_dir() {
            merge_into(&src, &dst)?;
            continue;
        }
        if dst.exists() {
            let _ = fs::remove_file(&dst);
        }
        if fs::hard_link(&src, &dst).is_err() {
            fs::copy(&src, &dst).map_err(|e| format!("copying {}: {e}", src.display()))?;
        }
    }
    Ok(())
}

fn system_tool(name: &str) -> PathBuf {
    std::env::var("SystemRoot").map(|r| Path::new(&r).join("System32").join(name)).unwrap_or_else(|_| PathBuf::from(name))
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.stdin(Stdio::null())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_pins_a_full_hash_and_an_https_address() {
        for p in PIECES {
            assert_eq!(p.sha256.len(), 64, "{}", p.key);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", p.key);
            assert!(p.url.starts_with("https://"), "{}", p.key);
            assert!(p.bytes > 0 && p.exe.ends_with(".exe"), "{}", p.key);
        }
    }

    #[test]
    fn the_llvm_archive_is_the_smaller_zstd_one() {
        let llvm = &PIECES[0];
        assert!(llvm.file.ends_with(".tar.zst"));
        assert!(llvm.bytes < 300_000_000, "the zstd release is 264 MB, not the 901 MB xz");
        assert!(!llvm.keep.is_empty());
    }

    #[test]
    fn only_the_builds_llvm_files_are_taken_from_the_archive() {
        let top = Path::new("clang+llvm-23.1.5-x86_64-pc-windows-msvc");
        let rel = |path: &str| archive_relative(&top.join(path)).expect("relative path");
        let llvm = &PIECES[0];
        assert!(kept(llvm, &rel("bin/clang-cl.exe")));
        assert!(kept(llvm, &rel("bin/lld-link.exe")));
        assert!(kept(llvm, &rel("bin/llvm-lib.exe")));
        assert!(kept(llvm, &rel("bin/llvm-rc.exe")));
        assert!(kept(llvm, &rel("bin/llvm-mt.exe")));
        assert!(kept(llvm, &rel("lib/clang/23/include/stddef.h")));
        assert!(!kept(llvm, &rel("bin/llvm-cvtres.exe")));
        assert!(!kept(llvm, &rel("bin/clangd.exe")));
        assert!(!kept(llvm, &rel("include/clang/AST/AST.h")));
        assert!(!kept(llvm, &rel("lib/LLVMCodeGen.lib")));
    }

    #[test]
    fn trimming_leaves_tools_that_are_not_llvm_alone() {
        assert!(is_llvm_tool("clangd.exe"));
        assert!(is_llvm_tool("ld.lld.exe"));
        assert!(is_llvm_tool("ld64.lld.exe"));
        assert!(is_llvm_tool("wasm-ld.exe"));
        assert!(is_llvm_tool("llvm-cvtres.exe"));
        assert!(is_llvm_library("LLVM-C.dll"));
        for name in ["cmake.exe", "cmake-gui.exe", "ctest.exe", "ninja.exe", "python.exe", "python312.dll"] {
            assert!(!is_llvm_tool(name), "{name} is not an LLVM tool");
            assert!(!is_llvm_library(name), "{name} is not an LLVM library");
        }
    }

    #[test]
    fn zstd_unpack_takes_the_kept_files_only() {
        let root = std::env::temp_dir().join(format!("emblemweave-unpack-test-{}", std::process::id()));
        let archive = root.join("fixture.tar.zst");
        let out = root.join("out");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&out).unwrap();

        let file = fs::File::create(&archive).unwrap();
        let encoder = zstd::stream::write::Encoder::new(file, 0).unwrap();
        let mut tar = tar::Builder::new(encoder);
        for (path, body) in [
            ("top/bin/clang-cl.exe", b"compiler".as_slice()),
            ("top/bin/clangd.exe", b"extra".as_slice()),
            ("top/lib/clang/23/include/stddef.h", b"header".as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, body).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();

        unpack_zstd(&PIECES[0], &archive, &out).expect("the fixture unpacks");
        assert!(out.join("bin/clang-cl.exe").is_file());
        assert!(out.join("lib/clang/23/include/stddef.h").is_file());
        assert!(!out.join("bin/clangd.exe").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_archive_cannot_escape_the_unpack_folder() {
        assert!(archive_relative(Path::new("top/../evil")).is_none());
        assert!(archive_relative(Path::new("../evil")).is_none());
        assert_eq!(archive_relative(Path::new("top/bin/clang-cl.exe")), Some(PathBuf::from("bin/clang-cl.exe")));
    }

    #[test]
    fn the_stamp_names_the_archive_it_was_unpacked_from() {
        assert!(bin_dir().is_some() || crate::engine::root().is_none());
        for p in PIECES {
            assert!(format!("{STAMP_PREFIX}{}", p.key).starts_with(".carried-"));
        }
    }

    /// Writes the stamps and every file the four pieces install, so a test can take one away the way a quarantine does.
    fn plant_carried_toolchain(tools: &Path) {
        let bin = tools.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for piece in PIECES {
            fs::write(bin.join(format!("{STAMP_PREFIX}{}", piece.key)), format!("{}\n", piece.sha256)).unwrap();
            fs::write(bin.join(piece.exe), b"carried").unwrap();
            for rel in piece.keep {
                let path = tools.join(rel);
                if rel.ends_with('/') {
                    fs::create_dir_all(&path).unwrap();
                } else {
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(&path, b"carried").unwrap();
                }
            }
        }
    }

    #[test]
    fn a_quarantined_file_in_a_carried_toolchain_is_named_with_the_way_back() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        let llvm = PIECES.iter().find(|p| p.key == "clang").expect("the LLVM piece");
        assert!(carried_in(tools, llvm), "the planted toolchain is complete");
        assert!(damage_note_in(tools).is_none());

        fs::remove_file(tools.join("bin").join("lld-link.exe")).unwrap();
        assert!(!carried_in(tools, llvm), "a marker exe and a stamp are not a complete piece");
        let note = damage_note_in(tools).expect("the damaged piece is reported");
        assert!(note.contains("lld-link.exe"), "{note}");
        assert!(note.contains("quarantine"), "{note}");
        assert!(note.contains("Repair on Overview"), "{note}");
        assert!(note.contains("--fetch-tools"), "{note}");
        assert!(!note.contains("CMake"), "an untouched piece is not reported: {note}");

        fs::remove_dir_all(tools.join("lib").join("clang")).unwrap();
        let note = damage_note_in(tools).expect("the resource headers are reported too");
        assert!(note.contains("include"), "{note}");
    }

    /// The Overview's Repair button must re-fetch exactly the pieces the report names, not the whole toolchain.
    #[test]
    fn a_repair_targets_only_the_damaged_pieces() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        assert!(damaged_in(tools).is_empty());
        assert!(damage_in(tools).is_empty());

        fs::remove_file(tools.join("bin").join("lld-link.exe")).unwrap();
        let keys = damaged_in(tools).iter().map(|p| p.key).collect::<Vec<_>>();
        assert_eq!(keys, vec!["clang"]);
        let damage = damage_in(tools);
        assert_eq!(damage.len(), 1);
        assert!(
            damage[0].summary().contains("lld-link.exe"),
            "the report names the missing file: {}",
            damage[0].summary()
        );

        fs::remove_file(tools.join("bin").join("cmake.exe")).unwrap();
        let keys = damaged_in(tools).iter().map(|p| p.key).collect::<Vec<_>>();
        assert_eq!(keys, vec!["clang", "cmake"], "the report keeps the pieces' order");
    }

    #[test]
    fn a_piece_that_was_never_carried_is_left_to_the_first_run_fetch() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        fs::remove_file(tools.join("bin").join(format!("{STAMP_PREFIX}clang"))).unwrap();
        fs::remove_file(tools.join("bin").join("lld-link.exe")).unwrap();
        assert!(damage_note_in(tools).is_none(), "no stamp means nothing was carried here");
    }
}
