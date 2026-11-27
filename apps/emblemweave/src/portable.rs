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
    /// The files and folders the piece needs, relative to `tools`. They also select what to take from a `.tar.zst`
    /// archive; the small zip pieces are extracted whole, so for those this is the completeness check. Empty means the
    /// executable alone proves the piece is in place.
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

/// CMake's compiler-detection, compiler-specific and Ninja-generator modules live beside `tools/bin`. Without them every
/// configure fails before the project is read, so the files the engine's configure always loads are part of the piece; a
/// quarantine that took only part of the module tree is then caught like a missing `cmake.exe`.
const CMAKE_KEEP: &[&str] = &[
    "share/cmake-3.31/Modules/CMakeDetermineSystem.cmake",
    "share/cmake-3.31/Modules/CMakeDetermineCCompiler.cmake",
    "share/cmake-3.31/Modules/CMakeDetermineCXXCompiler.cmake",
    "share/cmake-3.31/Modules/CMakeCInformation.cmake",
    "share/cmake-3.31/Modules/CMakeCXXInformation.cmake",
    "share/cmake-3.31/Modules/CMakeTestCCompiler.cmake",
    "share/cmake-3.31/Modules/CMakeTestCXXCompiler.cmake",
    "share/cmake-3.31/Modules/CMakeNinjaFindMake.cmake",
    "share/cmake-3.31/Modules/Compiler/Clang.cmake",
    "share/cmake-3.31/Modules/Platform/Windows-Clang.cmake",
];

/// The embeddable Python cannot start its stdlib or load an extension without these: the interpreter DLL, the
/// stable-ABI DLL extensions link against, and the standard-library zip.
const PYTHON_KEEP: &[&str] = &[
    "bin/python312.dll",
    "bin/python3.dll",
    "bin/python312.zip",
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
        keep: CMAKE_KEEP,
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
        keep: PYTHON_KEEP,
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

const MANIFEST_SUFFIX: &str = ".manifest";

/// Why a required file failed its check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The path is not there at all.
    Missing,
    /// The path exists but is empty, the wrong size, or not the bytes that were installed.
    Damaged,
}

/// One required path that failed its check.
#[derive(Debug, Clone)]
pub struct Problem {
    pub path: PathBuf,
    pub fault: Fault,
}

/// Every path a piece needs at run time. A `keep` entry ending in `/` is a folder whose files the build reads (clang's
/// resource headers); a deep check walks it, while the shallow check the Overview runs on every frame only looks at the
/// folder itself. Walking and hashing a thousand headers per frame would cost more than the page.
fn required_files(tools: &Path, piece: &Piece, deep: bool) -> Vec<PathBuf> {
    let mut out = vec![tools.join("bin").join(piece.exe)];
    for rel in piece.keep {
        let path = tools.join(rel);
        if rel.ends_with('/') {
            if deep {
                walk_files(&path, &mut out);
            }
        } else {
            out.push(path);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The manifest written after an install: one `sha256 size relative/path` line per required file. The stamp only records
/// that the archive was unpacked once, and an antivirus quarantine removes individual files while leaving the stamp, so
/// the manifest is what lets the check tell a whole file from a truncated or substituted one (issue #116).
#[derive(Debug, Clone)]
struct ManifestEntry {
    sha256: String,
    size: u64,
}

fn manifest_path(tools: &Path, piece: &Piece) -> PathBuf {
    tools
        .join("bin")
        .join(format!("{STAMP_PREFIX}{}{MANIFEST_SUFFIX}", piece.key))
}

fn relative_path(tools: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(tools)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// Records what was installed, after the archive's own hash has been checked and before the stamp is written. A required
/// file the archive did not contain fails the install here, so a partial extraction is never stamped as whole.
fn write_manifest(tools: &Path, piece: &Piece) -> Result<(), String> {
    let mut lines = Vec::new();
    let mut absent = Vec::new();
    for path in required_files(tools, piece, true) {
        if !path.is_file() {
            absent.push(path);
            continue;
        }
        let Some(rel) = relative_path(tools, &path) else { continue };
        let size = fs::metadata(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?
            .len();
        let sha256 = sha256_file(&path)?;
        lines.push(format!("{sha256} {size} {rel}"));
    }
    if !absent.is_empty() {
        let names = absent
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "{} did not unpack {names}. Delete tools\\unpack and run the fetch again.",
            piece.file
        ));
    }
    lines.sort();
    fs::write(manifest_path(tools, piece), lines.join("\n") + "\n")
        .map_err(|e| format!("writing the toolchain manifest: {e}"))
}

fn read_manifest(tools: &Path, piece: &Piece) -> Option<Vec<(String, ManifestEntry)>> {
    let text = fs::read_to_string(manifest_path(tools, piece)).ok()?;
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, ' ');
        let sha256 = parts.next()?;
        let size = parts.next()?.parse().ok()?;
        let rel = parts.next()?;
        if sha256.len() != 64 || rel.is_empty() {
            continue;
        }
        out.push((rel.to_string(), ManifestEntry { sha256: sha256.to_string(), size }));
    }
    Some(out)
}

/// The required files that are missing or no longer intact. `deep` also re-hashes every file against the manifest; the
/// shallow check compares sizes only, which is what the Overview can afford to run on every frame. An install from before
/// manifests existed has no recorded hashes: its files are still checked for presence and emptiness, and the report says
/// they are unverified rather than pretending they are whole.
fn problems_in(tools: &Path, piece: &Piece, deep: bool) -> Vec<Problem> {
    let manifest: Option<std::collections::HashMap<String, ManifestEntry>> =
        read_manifest(tools, piece).map(|entries| entries.into_iter().collect());
    let mut out: Vec<Problem> = Vec::new();
    // A required folder that is gone has no files to walk, so name the folder itself.
    for rel in piece.keep {
        if rel.ends_with('/') && !tools.join(rel).is_dir() {
            out.push(Problem { path: tools.join(rel), fault: Fault::Missing });
        }
    }
    for path in required_files(tools, piece, deep) {
        let fault = match fs::metadata(&path) {
            Err(_) => Some(Fault::Missing),
            Ok(meta) if !meta.is_file() => Some(Fault::Missing),
            Ok(meta) if meta.len() == 0 => Some(Fault::Damaged),
            Ok(meta) => match relative_path(tools, &path).and_then(|rel| manifest.as_ref().map(|m| (rel, m))) {
                Some((rel, manifest)) => match manifest.get(&rel) {
                    Some(entry) if entry.size != meta.len() => Some(Fault::Damaged),
                    Some(entry) if deep => match sha256_file(&path) {
                        Ok(got) if got != entry.sha256 => Some(Fault::Damaged),
                        Ok(_) => None,
                        // Present but unreadable is damage, not a reason to pass it.
                        Err(_) => Some(Fault::Damaged),
                    },
                    _ => None,
                },
                None => None,
            },
        };
        if let Some(fault) = fault {
            if !out.iter().any(|p| p.path == path) {
                out.push(Problem { path, fault });
            }
        }
    }
    // A file inside a folder keep is not in `required_files` once it has been deleted, so the deep pass also walks the
    // manifest. That is how a quarantine that took one header out of clang's resource tree is named.
    if deep {
        if let Some(entries) = &manifest {
            for (rel, entry) in entries {
                let path = tools.join(rel);
                let fault = match fs::metadata(&path) {
                    Err(_) => Some(Fault::Missing),
                    Ok(meta) if !meta.is_file() => Some(Fault::Missing),
                    Ok(meta) if meta.len() != entry.size => Some(Fault::Damaged),
                    Ok(_) => match sha256_file(&path) {
                        Ok(got) if got != entry.sha256 => Some(Fault::Damaged),
                        Ok(_) => None,
                        Err(_) => Some(Fault::Damaged),
                    },
                };
                if let Some(fault) = fault {
                    if !out.iter().any(|p| p.path == path) {
                        out.push(Problem { path, fault });
                    }
                }
            }
        }
    }
    out
}

/// Whether this piece is in place: its stamp matches the archive, and every required file is present and the size the
/// install recorded. The stamp alone is not proof of anything (issue #116).
fn carried_in(tools: &Path, piece: &Piece) -> bool {
    problems_in(tools, piece, false).is_empty()
        && fs::read_to_string(tools.join("bin").join(format!("{STAMP_PREFIX}{}", piece.key)))
            .map(|s| s.trim() == piece.sha256)
            .unwrap_or(false)
}

/// Whether this piece is already in place, from the stamp the installer writes and the files it unpacked.
pub fn carried(piece: &Piece) -> bool {
    dir().is_some_and(|tools| carried_in(&tools, piece))
}

/// One carried piece whose files are partly gone: enough to report on Overview and repair with one press.
#[derive(Debug, Clone)]
pub struct Damage {
    pub piece: &'static Piece,
    /// Required paths that are not there.
    pub missing: Vec<PathBuf>,
    /// Required paths that are there but empty, truncated, or not the bytes that were installed.
    pub damaged: Vec<PathBuf>,
}

impl Damage {
    fn from_problems(piece: &'static Piece, problems: Vec<Problem>) -> Option<Self> {
        if problems.is_empty() {
            return None;
        }
        let (damaged, missing) = problems.into_iter().fold((Vec::new(), Vec::new()), |(mut d, mut m), p| {
            if p.fault == Fault::Damaged { d.push(p.path) } else { m.push(p.path) }
            (d, m)
        });
        Some(Damage { piece, missing, damaged })
    }

    /// `LLVM (clang-cl, lld-link and llvm-rc): lld-link.exe (damaged)` for a one-line report.
    pub fn summary(&self) -> String {
        let names = |paths: &[PathBuf], mark: &str| {
            paths
                .iter()
                .filter_map(|p| p.file_name())
                .map(|n| format!("{}{mark}", n.to_string_lossy()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let missing = names(&self.missing, "");
        let damaged = names(&self.damaged, " (damaged)");
        let files = [missing, damaged].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
        format!("{}: {files}", self.piece.name)
    }

    /// Every path the report names, missing first.
    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.missing.iter().chain(self.damaged.iter())
    }

    /// Whether the piece's own executable is gone or unusable. If it is still there, the build's `NX_LLVM`/PATH finds
    /// that broken carried copy before an installed one, so installed tools cannot take its place.
    fn executable_missing(&self) -> bool {
        let exe = self.piece.exe;
        self.paths()
            .any(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(exe)))
    }

    fn blocks_build_with(&self, system_ready: bool) -> bool {
        match self.piece.key {
            // `find_clang` skips an incomplete carried folder, so a complete installed LLVM takes over even while the
            // damaged `clang-cl.exe` is still present.
            "clang" => !system_ready,
            // CMake, Ninja and Python are single executables: a present carried copy is found first and cannot yield to
            // an installed one, so only a missing executable makes the fall-through safe.
            _ => !self.executable_missing() || !system_ready,
        }
    }

    /// Whether this damage must stop Rebuild. A complete installed copy can take over when the carried executable is
    /// gone, or when it is LLVM and selection skips the incomplete folder. Otherwise the present carried copy shadows
    /// the installed one.
    pub fn blocks_build(&self) -> bool {
        self.blocks_build_with(system_ready(self.piece))
    }
}

/// The carried pieces whose stamp is still there but whose files are partly gone, with the missing paths. A piece that
/// was never carried is left to the first-run fetch, which installs it without being asked.
fn damage_in(tools: &Path, deep: bool) -> Vec<Damage> {
    let bin = tools.join("bin");
    PIECES
        .iter()
        .filter(|piece| bin.join(format!("{STAMP_PREFIX}{}", piece.key)).is_file())
        .filter_map(|piece| Damage::from_problems(piece, problems_in(tools, piece, deep)))
        .collect()
}

/// The pieces a repair has to fetch again, in the same order as [`PIECES`]. A repair checks hashes, not just sizes: the
/// player pressed the button because something is wrong, and a same-size substitution is exactly what a size check misses.
fn damaged_in(tools: &Path) -> Vec<&'static Piece> {
    damage_in(tools, true).into_iter().map(|d| d.piece).collect()
}

/// The damage report for the engine this launcher runs, for the Overview. Sizes only: this runs on the UI thread, and a
/// truncated or quarantined file is the common case. Empty when every carried piece is whole.
pub fn damage() -> Vec<Damage> {
    dir().map(|tools| damage_in(&tools, false)).unwrap_or_default()
}

/// The same report with every required file re-hashed. Used before a build starts and by `--verify-tools`, where a few
/// hundred megabytes of hashing is cheap next to the build itself and a same-size substitution must not pass.
pub fn damage_deep() -> Vec<Damage> {
    dir().map(|tools| damage_in(&tools, true)).unwrap_or_default()
}

/// A line per carried piece for `--verify-tools`: what is intact, what is not, and whether an older install has no
/// manifest to check against.
pub fn verify_report() -> (String, bool) {
    let Some(tools) = dir() else {
        return ("the engine folder engine/fefw-recomp was not found".into(), false);
    };
    let stamp = |piece: &Piece| tools.join("bin").join(format!("{STAMP_PREFIX}{}", piece.key)).is_file();
    let carried: Vec<&Piece> = PIECES.iter().filter(|p| stamp(p)).collect();
    if carried.is_empty() {
        return ("no toolchain piece is carried; run EmblemWeave.exe --fetch-tools".into(), false);
    }
    let damaged = damage_in(&tools, true);
    let mut verified = damaged.is_empty();
    let mut lines = Vec::new();
    for piece in carried {
        match damaged.iter().find(|d| d.piece.key == piece.key) {
            Some(d) => lines.push(format!("{}: {}", d.piece.name, d.summary())),
            None if read_manifest(&tools, piece).is_none() => {
                verified = false;
                lines.push(format!(
                    "{}: files are present and non-empty, but this install has no recorded hashes; run --fetch-tools to \
                     reinstall it with a manifest",
                    piece.name
                ));
            }
            None => lines.push(format!("{}: intact", piece.name)),
        }
    }
    (lines.join("\n"), verified)
}

/// Names the pieces whose files are partly gone, with the way back. None when the list is empty.
pub fn damage_note_for(damaged: &[Damage]) -> Option<String> {
    if damaged.is_empty() {
        return None;
    }
    let lines = damaged.iter().map(|d| format!("  {}", d.summary())).collect::<Vec<_>>();
    Some(format!(
        "files the toolchain EmblemWeave carried into tools\\bin needs are missing or damaged:\n{}\n\nAn antivirus \
         quarantine removes or truncates files like these and leaves the install stamp behind. Press Repair on Overview, \
         restore them from the quarantine, or run EmblemWeave.exe --fetch-tools, then press Rebuild.",
        lines.join("\n")
    ))
}

/// Whether the PC already has a tool this piece would supply. Checked in the background before the launcher downloads
/// anything: a player with LLVM 23, CMake, Ninja and Python installed gets no duplicate copy and no download.
fn system_ready(piece: &Piece) -> bool {
    match piece.key {
        "clang" => crate::toolchain::clang_candidates()
            .into_iter()
            .any(|exe| crate::toolchain::clang_is_ready(&exe)),
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
/// the downloading thread, so the UI can keep drawing. `force` re-fetches a piece that is already carried; the CLI's
/// `--fetch-tools` uses it so an install from before manifests existed can gain recorded hashes.
pub fn install(piece: &'static Piece, progress: &dyn Fn(f32, String)) -> Result<(), String> {
    install_inner(piece, progress, false)
}

fn install_inner(piece: &'static Piece, progress: &dyn Fn(f32, String), force: bool) -> Result<(), String> {
    if !force && carried(piece) {
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

    // Record the installed files before stamping the piece: the stamp says the archive was unpacked, the manifest says
    // what it produced. A required file the archive did not hold fails here instead of being stamped as carried.
    write_manifest(&tools, piece)?;
    fs::write(bin.join(format!("{STAMP_PREFIX}{}", piece.key)), format!("{}\n", piece.sha256))
        .map_err(|e| format!("writing the stamp: {e}"))?;
    // The archive has been checked and unpacked: keeping it only costs a second copy of the download. A failed install
    // keeps it so the next attempt does not download again.
    let _ = fs::remove_file(&archive);
    progress(1.0, format!("{} {} is carried", piece.name, piece.version));
    Ok(())
}

/// A subset of the pieces, for the background first-run fetch. The CLI's `--fetch-tools` uses [`install_all`].
fn install_selected(pieces: &[&'static Piece], progress: &dyn Fn(f32, String), force: bool) -> Result<(), String> {
    let total = pieces.len().max(1) as f32;
    for (i, piece) in pieces.iter().enumerate() {
        let base = i as f32 / total;
        install_inner(piece, &|f, line| progress((base + f / total).min(1.0), line), force)?;
    }
    Ok(())
}

/// Every piece, in order, downloaded again even when its stamp is present: `--fetch-tools` is the documented way to get a
/// fresh, hash-verified toolchain and a manifest for an install from before manifests existed. Trims an existing full LLVM
/// install, then removes the archives a successful install leaves behind.
pub fn install_all(progress: &dyn Fn(f32, String)) -> Result<(), String> {
    let pieces: Vec<&'static Piece> = PIECES.iter().collect();
    trim_after_fetch(install_selected(&pieces, progress, true))
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
    trim_after_fetch(install_selected(&damaged, progress, false))
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
        }, false);
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
            // One file inside the folder keep, so the shallow/deep split has something to differ on.
            if piece.key == "clang" {
                let header = tools.join("lib/clang/23/include/stddef.h");
                fs::create_dir_all(header.parent().unwrap()).unwrap();
                fs::write(&header, b"header").unwrap();
            }
            write_manifest(tools, piece).unwrap();
        }
    }

    #[test]
    fn a_quarantined_file_in_a_carried_toolchain_is_named_with_the_way_back() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        let llvm = PIECES.iter().find(|p| p.key == "clang").expect("the LLVM piece");
        assert!(carried_in(tools, llvm), "the planted toolchain is complete");
        assert!(damage_note_for(&damage_in(tools, false)).is_none());

        fs::remove_file(tools.join("bin").join("lld-link.exe")).unwrap();
        assert!(!carried_in(tools, llvm), "a marker exe and a stamp are not a complete piece");
        let note = damage_note_for(&damage_in(tools, false)).expect("the damaged piece is reported");
        assert!(note.contains("lld-link.exe"), "{note}");
        assert!(note.contains("quarantine"), "{note}");
        assert!(note.contains("Repair on Overview"), "{note}");
        assert!(!note.contains("CMake"), "an untouched piece is not reported: {note}");

        fs::remove_dir_all(tools.join("lib").join("clang")).unwrap();
        let note = damage_note_for(&damage_in(tools, false)).expect("the resource headers are reported too");
        assert!(note.contains("include"), "{note}");
    }

    /// The Overview's Repair button must re-fetch exactly the pieces the report names, not the whole toolchain.
    #[test]
    fn a_repair_targets_only_the_damaged_pieces() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        assert!(damaged_in(tools).is_empty());
        assert!(damage_in(tools, false).is_empty());

        fs::remove_file(tools.join("bin").join("lld-link.exe")).unwrap();
        let keys = damaged_in(tools).iter().map(|p| p.key).collect::<Vec<_>>();
        assert_eq!(keys, vec!["clang"]);
        let damage = damage_in(tools, false);
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

    /// CMake's executables can survive a quarantine that took the modules they load. The module tree is part of the
    /// piece, not just `cmake.exe`, so a partial quarantine is named before a configure fails.
    #[test]
    fn a_quarantined_cmake_module_is_damage() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        assert!(damage_in(tools, false).is_empty());

        fs::remove_file(tools.join("share/cmake-3.31/Modules/CMakeDetermineSystem.cmake")).unwrap();
        let damage = damage_in(tools, false);
        assert_eq!(damage.len(), 1, "only CMake is damaged");
        assert!(
            damage[0].summary().contains("CMakeDetermineSystem.cmake"),
            "the module file is named: {}",
            damage[0].summary()
        );
        let keys = damaged_in(tools).iter().map(|p| p.key).collect::<Vec<_>>();
        assert_eq!(keys, vec!["cmake"]);
    }

    #[test]
    fn a_quarantined_python_dll_or_zip_is_damage() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        assert!(damage_in(tools, false).is_empty());

        fs::remove_file(tools.join("bin/python312.dll")).unwrap();
        fs::remove_file(tools.join("bin/python312.zip")).unwrap();
        let damage = damage_in(tools, false);
        assert_eq!(damage.len(), 1, "only Python is damaged");
        let summary = damage[0].summary();
        assert!(summary.contains("python312.dll"), "{summary}");
        assert!(summary.contains("python312.zip"), "{summary}");
        let keys = damaged_in(tools).iter().map(|p| p.key).collect::<Vec<_>>();
        assert_eq!(keys, vec!["python"]);
    }

    /// A quarantine can leave a file that still exists but is truncated. Presence alone (and the stamp) called that
    /// whole, and the build then died at the first link with no clue which file was short (issue #116).
    #[test]
    fn a_truncated_carried_executable_is_damage() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        assert!(damage_in(tools, false).is_empty());

        fs::write(tools.join("bin").join("clang-cl.exe"), b"short").unwrap();
        let damage = damage_in(tools, false);
        assert_eq!(damage.len(), 1, "only LLVM is damaged");
        assert!(
            damage[0].damaged.iter().any(|p| p.ends_with("clang-cl.exe")),
            "the truncated compiler is named: {}",
            damage[0].summary()
        );
        assert!(damage[0].summary().contains("damaged"), "{}", damage[0].summary());
    }

    /// A same-size substitution passes every size check. Only the deep check, which re-hashes against the manifest
    /// written at install time, can see it; this is what the build gate and `--verify-tools` run.
    #[test]
    fn a_same_size_substitution_is_caught_only_by_hashing() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        assert!(damage_in(tools, true).is_empty());

        fs::write(tools.join("bin").join("ninja.exe"), b"xxxxxxx").unwrap();
        assert!(damage_in(tools, false).is_empty(), "the size did not change");
        let damage = damage_in(tools, true);
        assert_eq!(damage.len(), 1);
        assert!(
            damage[0].damaged.iter().any(|p| p.ends_with("ninja.exe")),
            "the substituted executable is named: {}",
            damage[0].summary()
        );
    }

    /// The Overview's shallow check must not walk clang's resource tree on every frame. A missing header is caught by
    /// the deep check the build gate and `--verify-tools` run; the folder itself is still checked cheaply.
    #[test]
    fn a_header_inside_the_resource_tree_is_checked_only_by_a_deep_pass() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        fs::remove_file(tools.join("lib/clang/23/include/stddef.h")).unwrap();
        assert!(damage_in(tools, false).is_empty(), "the shallow pass does not walk the tree");
        let damage = damage_in(tools, true);
        assert_eq!(damage.len(), 1);
        assert!(
            damage[0].missing.iter().any(|p| p.ends_with("stddef.h")),
            "the missing header is named: {}",
            damage[0].summary()
        );
    }

    /// The install writes what it produced before it stamps the piece, so a later check has a hash and size to compare
    /// against rather than trusting the stamp.
    #[test]
    fn an_install_records_a_hash_and_size_for_every_required_file() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        let manifest = fs::read_to_string(manifest_path(tools, &PIECES[0])).expect("the LLVM manifest");
        assert!(manifest.contains("bin/lld-link.exe"), "{manifest}");
        assert!(manifest.lines().all(|line| {
            let parts = line.splitn(3, ' ').collect::<Vec<_>>();
            parts.len() == 3 && parts[0].len() == 64 && parts[1].parse::<u64>().is_ok()
        }));
    }

    #[test]
    fn a_piece_that_was_never_carried_is_left_to_the_first_run_fetch() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);
        fs::remove_file(tools.join("bin").join(format!("{STAMP_PREFIX}clang"))).unwrap();
        fs::remove_file(tools.join("bin").join("lld-link.exe")).unwrap();
        assert!(
            damage_note_for(&damage_in(tools, false)).is_none(),
            "no stamp means nothing was carried here"
        );
    }

    /// A broken carried copy shadows an installed one; a piece whose executable is gone lets the tool search fall
    /// through, so a complete installed copy makes that damage safe to build past. LLVM is the exception: its selection
    /// skips an incomplete carried folder, so an installed LLVM takes over even while `clang-cl.exe` is still present.
    #[test]
    fn damage_blocks_only_when_the_carried_copy_would_be_used() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path();
        plant_carried_toolchain(tools);

        fs::remove_file(tools.join("bin").join("cmake.exe")).unwrap();
        let cmake = damage_in(tools, false).pop().expect("CMake is damaged");
        assert!(cmake.executable_missing());
        assert!(!cmake.blocks_build_with(true), "an installed CMake can take over");
        assert!(cmake.blocks_build_with(false), "without one there is nothing to use");

        fs::remove_file(tools.join("bin").join("lld-link.exe")).unwrap();
        let clang = damage_in(tools, false)
            .into_iter()
            .find(|d| d.piece.key == "clang")
            .expect("LLVM is damaged");
        assert!(!clang.executable_missing(), "clang-cl.exe is still there");
        assert!(
            !clang.blocks_build_with(true),
            "a complete installed LLVM is selected instead of the incomplete carried folder"
        );
        assert!(
            clang.blocks_build_with(false),
            "without an installed LLVM there is no usable compiler"
        );

        fs::remove_file(tools.join("bin").join("llvm-rc.exe")).unwrap();
        let clang = damage_in(tools, false)
            .into_iter()
            .find(|d| d.piece.key == "clang")
            .expect("LLVM is damaged");
        assert!(
            !clang.blocks_build_with(true),
            "a missing resource compiler is also covered by the installed LLVM"
        );
    }
}
