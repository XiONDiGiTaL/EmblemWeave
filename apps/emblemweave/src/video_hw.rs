//! Optional D3D11VA movie decoding, and the verified FFmpeg runtime it needs.
//!
//! The runtime is shipped with the release, so this module normally only
//! detects and enables it. If a player removes the folder (or a future package
//! leaves it out), the same manifest can install the exact ABI from the
//! project's pinned source, with a SHA-256 check before any file is written.
//!
//! The download is about 97 MB, including the licence and source notes.
//!
//! D3D11VA is a Windows path. The capability check reads the Vulkan AV1
//! video-decode extension because the launcher already enumerates Vulkan
//! devices; both APIs use the same hardware decoder. A GPU that does not
//! advertise it may still be enabled by hand, and the engine falls back to its
//! built-in dav1d if the first movie is refused.

use crate::gpu::Device;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RuntimeFile {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// Everything needed for a complete runtime, including the licence and the
/// source notes. The three DLLs are the files `engine::sdk_ffmpeg` checks.
pub const FILES: &[RuntimeFile] = &[
    RuntimeFile {
        name: "avcodec-62.dll",
        url: "https://raw.githubusercontent.com/XiONDiGiTaL/EmblemWeave/f280c51502f020e73f9a30fd89201319096559f8/upscalers/ffmpeg/avcodec-62.dll",
        sha256: "c202c131bb051e2298aaf5aca2571fc65d2e781ac07568c63c925b6d4e128877",
        bytes: 97_584_640,
    },
    RuntimeFile {
        name: "avutil-60.dll",
        url: "https://raw.githubusercontent.com/XiONDiGiTaL/EmblemWeave/f280c51502f020e73f9a30fd89201319096559f8/upscalers/ffmpeg/avutil-60.dll",
        sha256: "1645ea1d5774d07304731f842e6b16507f5000c331cca3cf6d71346ebc02541b",
        bytes: 2_930_688,
    },
    RuntimeFile {
        name: "swresample-6.dll",
        url: "https://raw.githubusercontent.com/XiONDiGiTaL/EmblemWeave/f280c51502f020e73f9a30fd89201319096559f8/upscalers/ffmpeg/swresample-6.dll",
        sha256: "ce4dce3d71ea83c49cc332437fd61fe847fa055f4f25a9cf120fd4f41294f1c4",
        bytes: 722_944,
    },
    RuntimeFile {
        name: "README.md",
        url: "https://raw.githubusercontent.com/XiONDiGiTaL/EmblemWeave/f280c51502f020e73f9a30fd89201319096559f8/upscalers/ffmpeg/README.md",
        sha256: "4ceb2f43dd30df1add758af80cfea554f8b873803e4b42fc8df7ec3d150b5012",
        bytes: 2_661,
    },
    RuntimeFile {
        name: "SHA256SUMS",
        url: "https://raw.githubusercontent.com/XiONDiGiTaL/EmblemWeave/f280c51502f020e73f9a30fd89201319096559f8/upscalers/ffmpeg/SHA256SUMS",
        sha256: "f048fd2b12c1ae5de5f487d35d5cebcef4950c6797d85291854476be45b13427",
        bytes: 244,
    },
    RuntimeFile {
        name: "LICENSE.txt",
        url: "https://raw.githubusercontent.com/XiONDiGiTaL/EmblemWeave/f280c51502f020e73f9a30fd89201319096559f8/upscalers/ffmpeg/LICENSE.txt",
        sha256: "8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903",
        bytes: 35_147,
    },
    RuntimeFile {
        name: "COPYING.GPLv3",
        url: "https://raw.githubusercontent.com/XiONDiGiTaL/EmblemWeave/f280c51502f020e73f9a30fd89201319096559f8/upscalers/ffmpeg/COPYING.GPLv3",
        sha256: "3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986",
        bytes: 35_149,
    },
];

/// The launcher-owned copy the engine finds under the play folder.
pub fn target_dir() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("upscalers").join("ffmpeg"))
}

/// Which device the GPU field will make the engine open, using the same rule
/// as the engine: a full name, a category, or Automatic (the best adapter type,
/// then the most video memory).
fn kind_rank(kind: &str) -> u8 {
    match kind {
        "Discrete" => 4,
        "Integrated" => 3,
        "Virtual" => 2,
        _ => 1,
    }
}

fn default_device(devices: &[Device]) -> Option<&Device> {
    devices
        .iter()
        .max_by_key(|d| (kind_rank(d.kind), d.vram_mb))
}

pub fn selected_device<'a>(value: &str, devices: &'a [Device]) -> Option<&'a Device> {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "" | "automatic" | "auto" => default_device(devices),
        "discrete" => devices
            .iter()
            .filter(|d| d.kind == "Discrete")
            .max_by_key(|d| d.vram_mb)
            .or_else(|| default_device(devices)),
        "integrated" => devices
            .iter()
            .filter(|d| d.kind == "Integrated")
            .max_by_key(|d| d.vram_mb)
            .or_else(|| default_device(devices)),
        _ => devices
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(value))
            .or_else(|| {
                let needle = value.to_ascii_lowercase();
                devices
                    .iter()
                    .find(|d| d.name.to_ascii_lowercase().contains(&needle))
            })
            .or_else(|| default_device(devices)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The Vulkan scan has not finished yet.
    Checking,
    /// The scan finished but found no usable adapter.
    NoDevice,
    Supported(String),
    Unsupported(String),
}

impl Check {
    pub fn device(&self) -> Option<&str> {
        match self {
            Check::Supported(name) | Check::Unsupported(name) => Some(name),
            _ => None,
        }
    }

    pub fn supported(&self) -> bool {
        matches!(self, Check::Supported(_))
    }
}

/// Read the capability the GPU driver reports. This is deliberately a check,
/// not a block: the engine still falls back to dav1d on a failed first frame.
pub fn check(gpu: &str, devices: Option<&[Device]>) -> Check {
    let Some(devices) = devices else {
        return Check::Checking;
    };
    let Some(device) = selected_device(gpu, devices) else {
        return Check::NoDevice;
    };
    if device.av1_decode {
        Check::Supported(device.name.clone())
    } else {
        Check::Unsupported(device.name.clone())
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn matches(path: &Path, file: &RuntimeFile) -> bool {
    path.is_file() && sha256_file(path).is_ok_and(|got| got == file.sha256)
}

/// Download every missing file to a cache. Existing cached files with the
/// pinned hash are reused, so a retry after a failed install does not start
/// the 93 MiB DLL again.
pub fn download(
    progress: &mut dyn FnMut(u64, u64),
    cancel: &dyn Fn() -> bool,
) -> Result<PathBuf, String> {
    let config = crate::config::Config::dir().ok_or("the settings folder is unavailable")?;
    let cache = config.join("downloads").join("video-hw");
    fs::create_dir_all(&cache).map_err(|e| format!("creating {}: {e}", cache.display()))?;
    let total: u64 = FILES.iter().map(|f| f.bytes).sum();
    let mut done = 0u64;
    for file in FILES {
        let path = cache.join(file.name);
        if matches(&path, file) {
            done += file.bytes;
            progress(done, total);
            continue;
        }
        let part = cache.join(format!("{}.part", file.name));
        let _ = fs::remove_file(&part);
        let mut child =
            crate::process::hidden(&mut Command::new(crate::process::system_tool("curl.exe")))
                .args([
                    "-sSfL",
                    "--retry",
                    "2",
                    "--max-time",
                    "1800",
                    "-A",
                    concat!("EmblemWeave/", env!("CARGO_PKG_VERSION")),
                    "-o",
                ])
                .arg(&part)
                .arg(file.url)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("running curl: {e}"))?;
        let mut seen = 0u64;
        loop {
            if cancel() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&part);
                return Err("the download was cancelled".into());
            }
            let now = fs::metadata(&part)
                .map(|m| m.len())
                .unwrap_or(0)
                .min(file.bytes);
            if now != seen {
                seen = now;
                progress(done + now, total);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        let mut err = String::new();
                        if let Some(mut e) = child.stderr.take() {
                            let _ = e.read_to_string(&mut err);
                        }
                        let _ = fs::remove_file(&part);
                        return Err(format!("downloading {} failed: {}", file.name, err.trim()));
                    }
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                Err(e) => {
                    let _ = fs::remove_file(&part);
                    return Err(format!("waiting for curl: {e}"));
                }
            }
        }
        if !matches(&part, file) {
            let _ = fs::remove_file(&part);
            return Err(format!("{} did not match its pinned SHA-256", file.name));
        }
        let _ = fs::remove_file(&path);
        fs::rename(&part, &path).map_err(|e| format!("keeping {}: {e}", path.display()))?;
        done += file.bytes;
        progress(done, total);
    }
    Ok(cache)
}

/// Verify the whole cache first, then copy it into the launcher's `upscalers`
/// folder. A partial or modified download never reaches the engine.
pub fn install(cache: &Path) -> Result<PathBuf, String> {
    for file in FILES {
        let src = cache.join(file.name);
        if !matches(&src, file) {
            return Err(format!(
                "{} is missing or does not match its pinned SHA-256",
                src.display()
            ));
        }
    }
    let dest = target_dir().ok_or("the play folder is unavailable")?;
    fs::create_dir_all(&dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    for file in FILES {
        let src = cache.join(file.name);
        let out = dest.join(file.name);
        let tmp = dest.join(format!("{}.new", file.name));
        let _ = fs::remove_file(&tmp);
        fs::copy(&src, &tmp).map_err(|e| format!("copying {}: {e}", file.name))?;
        if !matches(&tmp, file) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("{} changed while it was copied", file.name));
        }
        let _ = fs::remove_file(&out);
        fs::rename(&tmp, &out).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("installing {}: {e}", file.name)
        })?;
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, kind: &'static str, vram_mb: u64, av1_decode: bool) -> Device {
        Device {
            name: name.into(),
            kind,
            vram_mb,
            av1_decode,
        }
    }

    #[test]
    fn every_runtime_file_is_https_and_has_a_full_hash() {
        assert!(!FILES.is_empty());
        for file in FILES {
            assert!(
                file.url.starts_with("https://"),
                "{} is not https",
                file.name
            );
            assert_eq!(file.sha256.len(), 64, "{} has no SHA-256", file.name);
            assert!(file.bytes > 0, "{} has no size", file.name);
            assert!(
                file.url
                    .contains("f280c51502f020e73f9a30fd89201319096559f8"),
                "{} is not pinned",
                file.name
            );
            assert!(
                file.url.ends_with(file.name),
                "{} points at another file",
                file.name
            );
        }
    }

    #[test]
    fn automatic_picks_the_discrete_adapter_with_most_memory() {
        let devices = vec![
            device("Radeon 780M Graphics", "Integrated", 512, true),
            device("NVIDIA RTX 4060 Laptop GPU", "Discrete", 8188, true),
            device("NVIDIA RTX 3050 Laptop GPU", "Discrete", 4096, false),
        ];
        assert_eq!(
            selected_device("Automatic", &devices).unwrap().name,
            "NVIDIA RTX 4060 Laptop GPU"
        );
        assert_eq!(
            selected_device("Discrete", &devices).unwrap().name,
            "NVIDIA RTX 4060 Laptop GPU"
        );
        assert_eq!(
            selected_device("Integrated", &devices).unwrap().name,
            "Radeon 780M Graphics"
        );
        assert_eq!(
            selected_device("RTX 3050", &devices).unwrap().name,
            "NVIDIA RTX 3050 Laptop GPU"
        );
        assert_eq!(
            selected_device("Missing card", &devices).unwrap().name,
            "NVIDIA RTX 4060 Laptop GPU"
        );
        let integrated_only = vec![device("Radeon 780M Graphics", "Integrated", 512, true)];
        assert_eq!(
            selected_device("Discrete", &integrated_only).unwrap().name,
            "Radeon 780M Graphics"
        );
    }

    #[test]
    fn the_check_follows_the_adapters_av1_capability() {
        let devices = vec![
            device("Radeon 780M Graphics", "Integrated", 512, true),
            device("NVIDIA RTX 4060 Laptop GPU", "Discrete", 8188, true),
            device("NVIDIA RTX 3050 Laptop GPU", "Discrete", 4096, false),
        ];
        assert_eq!(
            check("Automatic", Some(&devices)),
            Check::Supported("NVIDIA RTX 4060 Laptop GPU".into())
        );
        assert_eq!(
            check("RTX 3050", Some(&devices)),
            Check::Unsupported("NVIDIA RTX 3050 Laptop GPU".into())
        );
        assert_eq!(
            check("Integrated", Some(&devices)),
            Check::Supported("Radeon 780M Graphics".into())
        );
        assert_eq!(check("Automatic", None), Check::Checking);
        assert_eq!(check("Automatic", Some(&[])), Check::NoDevice);
    }
}
