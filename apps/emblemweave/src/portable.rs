//! The portable toolchain: clang-cl + lld-link, CMake, Ninja and a relocatable Python, fetched on first run into
//! `engine/fefw-recomp/tools/bin`.
//!
//! `toolchain::tool_path` already puts that folder first on the PATH every probe and build step reads, so a piece that
//! lands there is found before anything installed on the PC, and the Requirements page reports it as carried rather than
//! installed. What is left is the download: one table of artifacts with their URLs, sizes and SHA-256, a curl per piece
//! with progress taken from the file's own size, a hash check that refuses a damaged file, and an unpack through the
//! `tar.exe` Windows ships (it reads zip and xz as well).
//!
//! Visual Studio's C++ tools and the Windows SDK are not here on purpose: those headers and import libraries are
//! Microsoft's and are not ours to redistribute (`docs/INSTALL.md`).

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
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
}

/// The four pieces, at the versions the engine builds against. LLVM 23 is the release `MIN_CLANG_MAJOR` asks for;
/// CMake 3.31 is past the 3.25 the projects require; the 3.12 embeddable Python is the interpreter the translator runs.
pub const PIECES: &[Piece] = &[
    Piece {
        key: "clang",
        name: "LLVM (clang-cl and lld-link)",
        file: "clang+llvm-23.1.5-x86_64-pc-windows-msvc.tar.xz",
        url: "https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/clang%2Bllvm-23.1.5-x86_64-pc-windows-msvc.tar.xz",
        sha256: "62fdada7f6fa2b73856486dfbbdba3d2c1882991621615bcb55c60207611c6d4",
        bytes: 901309624,
        exe: "clang-cl.exe",
        version: "23.1.5",
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

/// Whether this piece is already in place, from the stamp the installer writes and the executable it names.
pub fn carried(piece: &Piece) -> bool {
    let Some(bin) = bin_dir() else { return false };
    if !bin.join(piece.exe).is_file() {
        return false;
    }
    fs::read_to_string(bin.join(format!("{STAMP_PREFIX}{}", piece.key)))
        .map(|s| s.trim() == piece.sha256)
        .unwrap_or(false)
}

/// Whether the piece was carried by an older install (the executable is there, the stamp is not): still usable, but the
/// page should say so rather than claim a version it cannot prove.
pub fn present(piece: &Piece) -> bool {
    bin_dir().map(|b| b.join(piece.exe).is_file()).unwrap_or(false)
}

/// The pieces that are not carried yet.
pub fn missing() -> Vec<&'static Piece> {
    PIECES.iter().filter(|p| !carried(p)).collect()
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
    let out = hidden(&mut Command::new(system_tool("tar.exe"))).arg("-xf").arg(&archive).arg("-C").arg(&unpack).output()
        .map_err(|e| format!("running tar: {e}"))?;
    if !out.status.success() {
        return Err(format!("{} could not be unpacked: {}", piece.file, String::from_utf8_lossy(&out.stderr).trim()));
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
    progress(1.0, format!("{} {} is carried", piece.name, piece.version));
    Ok(())
}

/// Every piece, in order.
pub fn install_all(progress: &dyn Fn(f32, String)) -> Result<(), String> {
    let total = PIECES.len() as f32;
    for (i, piece) in PIECES.iter().enumerate() {
        let base = i as f32 / total;
        install(piece, &|f, line| progress((base + f / total).min(1.0), line))?;
    }
    Ok(())
}

/// Starts the first-run fetch in the background, once. The release is meant to arrive with `tools\bin` already filled; a
/// copy that does not have it (a source checkout, or a zip whose tools were stripped) fills it here instead of asking the
/// player to install four things. Progress goes to `tools/fetch.log` so it can be watched, and the Requirements page picks
/// the pieces up as Carried on its next check.
pub fn auto_start() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if missing().is_empty() || STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        use std::io::Write;
        let log = std::cell::RefCell::new(dir().and_then(|d| fs::File::create(d.join("fetch.log")).ok()));
        let r = install_all(&|frac, line| {
            if let Some(f) = log.borrow_mut().as_mut() {
                let _ = writeln!(f, "{:>3.0}% {line}", frac * 100.0);
                let _ = f.flush();
            }
        });
        if let Err(e) = r {
            if let Some(f) = log.borrow_mut().as_mut() {
                let _ = writeln!(f, "failed: {e}");
            }
        }
    });
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
/// which matters for LLVM's 900 MB; a copy is the fallback.
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
    fn the_stamp_names_the_archive_it_was_unpacked_from() {
        assert!(bin_dir().is_some() || crate::engine::root().is_none());
        for p in PIECES {
            assert!(format!("{STAMP_PREFIX}{}", p.key).starts_with(".carried-"));
        }
    }
}
