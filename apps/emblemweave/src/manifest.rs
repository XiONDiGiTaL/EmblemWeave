//! A read-only manifest of the two ways the launcher can obtain the game's
//! files: a `.nup` package and an extracted `exefs/` + `romfs/` folder.
//!
//! The comparison is deliberately independent of the build, the runtime and
//! the launcher's `romfs_overlay`. It hashes the effective ExeFS and RomFS
//! entries by relative path, so a report can show whether the two data sets
//! are the same before any runtime behaviour changes.
//!
//! A `.nup` manifest reads every selected entry through
//! `ew_format::nup::Nup::read`, which checks the entry's SHA-256 while it
//! decompresses. A folder manifest reads every file in the same way. The
//! report therefore contains observed bytes, not only the hashes declared in
//! the package's table of contents.

use ew_format::nup::{Nup, Package, PackageKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const MANIFEST_FORMAT: &str = "emblemweave-data-manifest";
pub const COMPARISON_FORMAT: &str = "emblemweave-data-comparison";
pub const FORMAT_VERSION: u32 = 1;

/// At most this many examples of each kind of difference are kept in the
/// report. The counts still describe the whole data set.
const DIFF_EXAMPLES: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDigest {
    /// Relative path, `/` separated. ExeFS and RomFS paths are compared in
    /// their own sections, so `main` in ExeFS never collides with `main` in
    /// RomFS.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageRef {
    pub title_id: String,
    pub version: u32,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    /// The path the manifest was read from. It is evidence, not a comparison
    /// key.
    pub source: String,
    /// `nup` or `folder`.
    pub kind: String,
    pub exefs: Vec<FileDigest>,
    pub romfs: Vec<FileDigest>,
    pub exefs_bytes: u64,
    pub romfs_bytes: u64,
    /// Which package supplied the ExeFS the build copies out. For a folder
    /// this is absent.
    pub exefs_package: Option<PackageRef>,
    /// Which package supplied the RomFS the runtime will scan. It can differ
    /// from `exefs_package` when an update has no RomFS.
    pub romfs_package: Option<PackageRef>,
    pub notes: Vec<String>,
}

impl Manifest {
    fn new(source: &Path, kind: &str, exefs: Vec<FileDigest>, romfs: Vec<FileDigest>) -> Manifest {
        Manifest {
            format: MANIFEST_FORMAT.to_string(),
            format_version: FORMAT_VERSION,
            source: source.display().to_string(),
            kind: kind.to_string(),
            exefs_bytes: total_bytes(&exefs),
            romfs_bytes: total_bytes(&romfs),
            exefs,
            romfs,
            exefs_package: None,
            romfs_package: None,
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Difference {
    pub path: String,
    pub nup_size: Option<u64>,
    pub folder_size: Option<u64>,
    pub nup_sha256: Option<String>,
    pub folder_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SectionDiff {
    pub equal: bool,
    pub nup_files: usize,
    pub folder_files: usize,
    pub nup_bytes: u64,
    pub folder_bytes: u64,
    pub missing_in_folder_count: usize,
    pub extra_in_folder_count: usize,
    pub size_mismatch_count: usize,
    pub hash_mismatch_count: usize,
    pub missing_in_folder: Vec<Difference>,
    pub extra_in_folder: Vec<Difference>,
    pub size_mismatches: Vec<Difference>,
    pub hash_mismatches: Vec<Difference>,
    /// True when a count is larger than the examples kept in the vectors.
    pub examples_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Comparison {
    pub format: String,
    pub format_version: u32,
    pub equal: bool,
    pub nup: Manifest,
    pub folder: Manifest,
    pub exefs: SectionDiff,
    pub romfs: SectionDiff,
    pub notes: Vec<String>,
}

/// Build the manifest of a `.nup` as the launcher and runtime currently see
/// it.
///
/// The ExeFS comes from `program_package`, which is what `gamedata::use_nup`
/// copies. The RomFS comes from the highest-ranked base/update package with a
/// valid RomFS directory, matching the runtime's independent RomFS selection.
/// The notes name the runtime's own ExeFS choice when it differs from the
/// launcher's copy, and name the package split when ExeFS and RomFS do not come
/// from one package, instead of pretending the launcher's single-package
/// assumption holds.
pub fn nup_manifest(
    path: &Path,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Manifest, String> {
    let nup = Nup::open(path).map_err(|e| format!("reading {}: {e:#}", path.display()))?;
    let exe_pkg = nup
        .program_package()
        .ok_or_else(|| format!("{} holds no base game or update", path.display()))?;
    let rom_pkg = best_romfs_package(&nup)
        .ok_or_else(|| format!("{} holds no package with a RomFS", path.display()))?;

    if !valid_dir(&nup, exe_pkg.exefs) {
        return Err(format!(
            "{}: the program package {} has no ExeFS directory",
            path.display(),
            package_label(exe_pkg)
        ));
    }

    let runtime_exe_pkg = best_exefs_package(&nup);
    let exefs_entries = nup.walk(exe_pkg.exefs);
    let romfs_entries = nup.walk(rom_pkg.romfs);
    let total = exefs_entries.len() + romfs_entries.len();
    let mut done = 0usize;

    let exefs = read_nup_entries(&nup, path, &exefs_entries, &mut done, total, progress)?;
    let romfs = read_nup_entries(&nup, path, &romfs_entries, &mut done, total, progress)?;

    let mut manifest = Manifest::new(path, "nup", exefs, romfs);
    manifest.exefs_package = Some(PackageRef::from(exe_pkg));
    manifest.romfs_package = Some(PackageRef::from(rom_pkg));
    manifest.notes.push(
        "only the program ExeFS and RomFS are manifested; add-on packages in the .nup are not included"
            .to_string(),
    );
    if let Some(runtime_exe_pkg) = runtime_exe_pkg {
        if runtime_exe_pkg.exefs != exe_pkg.exefs {
            manifest.notes.push(format!(
                "the launcher copies ExeFS from {} (program_package), while the runtime would select {}; compare the folder against both when this copy is used for a build",
                package_label(exe_pkg),
                package_label(runtime_exe_pkg)
            ));
        }
    }
    if !valid_dir(&nup, exe_pkg.romfs) {
        manifest.notes.push(format!(
            "the program package {} has no RomFS; the manifest uses {} for RomFS, which is what the runtime selects, but the launcher's packed_romfs path currently assumes one package",
            package_label(exe_pkg),
            package_label(rom_pkg)
        ));
    } else if exe_pkg.romfs != rom_pkg.romfs {
        manifest.notes.push(format!(
            "the ExeFS comes from {} and the RomFS from {}; the launcher currently assumes one package while the runtime selects RomFS independently",
            package_label(exe_pkg),
            package_label(rom_pkg)
        ));
    }
    Ok(manifest)
}

fn read_nup_entries(
    nup: &Nup,
    path: &Path,
    entries: &[(String, u32)],
    done: &mut usize,
    total: usize,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<FileDigest>, String> {
    let mut out = Vec::with_capacity(entries.len());
    for (rel, index) in entries {
        out.push(hash_nup_entry(nup, path, rel, *index)?);
        *done += 1;
        progress(*done, total);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Hash one `.nup` entry as it is read, so a multi-gigabyte RomFS file does
/// not have to sit in memory. The bytes still go through the same stored or
/// zstd path as `Nup::read`, and both the decoded size and the table's SHA-256
/// are checked before the digest is accepted as evidence.
fn hash_nup_entry(nup: &Nup, source: &Path, rel: &str, index: u32) -> Result<FileDigest, String> {
    let entry = nup
        .files
        .get(index as usize)
        .ok_or_else(|| format!("{} names a file outside the .nup table", source.display()))?;
    let mut file =
        File::open(&nup.path).map_err(|e| format!("opening {}: {e}", nup.path.display()))?;
    file.seek(SeekFrom::Start(entry.offset))
        .map_err(|e| format!("seeking to {rel} in {}: {e}", nup.path.display()))?;
    let stored = file.take(entry.stored);
    let (size, sha256) = if entry.compressed {
        let decoder = ruzstd::StreamingDecoder::new(BufReader::new(stored))
            .map_err(|e| format!("decompressing {rel} from {}: {e}", nup.path.display()))?;
        digest_reader(decoder, rel)?
    } else {
        digest_reader(stored, rel)?
    };
    if size != entry.size {
        return Err(format!(
            "{rel} decoded to {size} bytes, the .nup says {}",
            entry.size
        ));
    }
    if sha256 != entry.sha256 {
        return Err(format!(
            "{rel} does not match its SHA-256: the .nup is damaged"
        ));
    }
    Ok(FileDigest {
        path: normalize_rel(Path::new(rel)),
        size,
        sha256: hex_bytes(&sha256),
    })
}

fn digest_reader(mut reader: impl Read, what: &str) -> Result<(u64, [u8; 32]), String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("reading {what}: {e}"))?;
        if n == 0 {
            break;
        }
        size = size.saturating_add(n as u64);
        hasher.update(&buf[..n]);
    }
    Ok((size, hasher.finalize().into()))
}

fn valid_dir(nup: &Nup, dir: u32) -> bool {
    (dir as usize) < nup.dirs.len()
}

fn package_rank(pkg: &Package) -> Option<u32> {
    match pkg.kind {
        PackageKind::Base => Some(0),
        PackageKind::Update => Some(1u32.saturating_add(pkg.version)),
        PackageKind::AddOn | PackageKind::Other(_) => None,
    }
}

fn best_exefs_package(nup: &Nup) -> Option<&Package> {
    best_ranked_package(nup, |p| p.exefs)
}

fn best_romfs_package(nup: &Nup) -> Option<&Package> {
    best_ranked_package(nup, |p| p.romfs)
}

/// The runtime keeps the first package at the highest rank (`rank > best`),
/// not the last one, so this mirrors that tie rule.
fn best_ranked_package(nup: &Nup, dir: impl Fn(&Package) -> u32) -> Option<&Package> {
    let mut best: Option<&Package> = None;
    for pkg in &nup.packages {
        let Some(rank) = package_rank(pkg) else {
            continue;
        };
        if !valid_dir(nup, dir(pkg)) {
            continue;
        }
        if best.map_or(true, |b| rank > package_rank(b).unwrap_or(0)) {
            best = Some(pkg);
        }
    }
    best
}

fn package_label(pkg: &Package) -> String {
    format!(
        "{} v{} ({})",
        kind_name(pkg.kind),
        pkg.version,
        format!("{:016X}", pkg.title_id)
    )
}

fn kind_name(kind: PackageKind) -> &'static str {
    match kind {
        PackageKind::Base => "base",
        PackageKind::Update => "update",
        PackageKind::AddOn => "add-on",
        PackageKind::Other(_) => "other",
    }
}

impl From<&Package> for PackageRef {
    fn from(pkg: &Package) -> PackageRef {
        PackageRef {
            title_id: format!("{:016X}", pkg.title_id),
            version: pkg.version,
            kind: kind_name(pkg.kind).to_string(),
        }
    }
}

/// Build the manifest of an extracted folder. `root` may be the dump root,
/// `dump/`, `game/`, or an `exefs`/`romfs` folder whose sibling is the other.
pub fn folder_manifest(
    root: &Path,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Manifest, String> {
    let (exefs_dir, romfs_dir) = discover_folder(root)?;
    let mut exefs_files = Vec::new();
    let mut romfs_files = Vec::new();
    collect_files(&exefs_dir, Path::new(""), &mut exefs_files, 0)?;
    collect_files(&romfs_dir, Path::new(""), &mut romfs_files, 0)?;

    let total = exefs_files.len() + romfs_files.len();
    let mut done = 0usize;
    let exefs = hash_folder_files(&exefs_files, &mut done, total, progress)?;
    let romfs = hash_folder_files(&romfs_files, &mut done, total, progress)?;

    let mut manifest = Manifest::new(root, "folder", exefs, romfs);
    manifest.notes.push(format!(
        "ExeFS read from {}; RomFS read from {}; mods, add-on content and romfs_overlay are not part of this manifest",
        exefs_dir.display(),
        romfs_dir.display()
    ));
    Ok(manifest)
}

fn discover_folder(root: &Path) -> Result<(PathBuf, PathBuf), String> {
    let candidates = [
        root.to_path_buf(),
        root.join("dump"),
        root.join("game"),
        root.join("game").join("dump"),
    ];
    for candidate in &candidates {
        let exefs = candidate.join("exefs");
        let romfs = candidate.join("romfs");
        if exefs.is_dir() && romfs.is_dir() {
            return Ok((exefs, romfs));
        }
    }

    // A player often points at one of the two folders instead of the dump
    // root. Use the sibling for the other one, but only when it exists.
    if let Some(name) = root.file_name().map(|n| n.to_string_lossy().to_string()) {
        if name.eq_ignore_ascii_case("exefs") {
            if let Some(parent) = root.parent() {
                let romfs = parent.join("romfs");
                if romfs.is_dir() {
                    return Ok((root.to_path_buf(), romfs));
                }
            }
        }
        if name.eq_ignore_ascii_case("romfs") {
            if let Some(parent) = root.parent() {
                let exefs = parent.join("exefs");
                if exefs.is_dir() {
                    return Ok((exefs, root.to_path_buf()));
                }
            }
        }
    }

    Err(format!(
        "{} has no extracted ExeFS and RomFS: expected <folder>/exefs and <folder>/romfs (or a dump/ or game/dump/ folder with both)",
        root.display()
    ))
}

fn collect_files(
    dir: &Path,
    rel: &Path,
    out: &mut Vec<(PathBuf, PathBuf, u64)>,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 {
        return Err(format!(
            "{} is nested more than 64 folders deep",
            dir.display()
        ));
    }
    let mut entries = std::fs::read_dir(dir)
        .map_err(|e| format!("reading {}: {e}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("reading {}: {e}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("reading the type of {}: {e}", path.display()))?;
        let child = rel.join(entry.file_name());
        if kind.is_symlink() {
            return Err(format!(
                "{} is a symbolic link; the manifest does not follow links because it must read exactly one tree",
                path.display()
            ));
        }
        if kind.is_dir() {
            collect_files(&path, &child, out, depth + 1)?;
        } else if kind.is_file() {
            let size = entry
                .metadata()
                .map_err(|e| format!("reading the size of {}: {e}", path.display()))?
                .len();
            out.push((PathBuf::from(normalize_rel(&child)), path, size));
        } else {
            return Err(format!(
                "{} is not a regular file or folder",
                path.display()
            ));
        }
    }
    Ok(())
}

fn hash_folder_files(
    files: &[(PathBuf, PathBuf, u64)],
    done: &mut usize,
    total: usize,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<FileDigest>, String> {
    let mut out = Vec::with_capacity(files.len());
    for (rel, path, expected) in files {
        let mut file = File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        let mut got = 0u64;
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            if n == 0 {
                break;
            }
            got = got.saturating_add(n as u64);
            hasher.update(&buf[..n]);
        }
        if got != *expected {
            return Err(format!(
                "{} changed while it was read: expected {expected} bytes, read {got}",
                path.display()
            ));
        }
        out.push(FileDigest {
            path: normalize_rel(rel),
            size: got,
            sha256: hex_bytes(&hasher.finalize()),
        });
        *done += 1;
        progress(*done, total);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn normalize_rel(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn total_bytes(files: &[FileDigest]) -> u64 {
    files.iter().map(|f| f.size).sum()
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Build both manifests and compare them. The progress callback gets
/// `("nup" | "folder", done, total)`.
pub fn compare_sources(
    nup_path: &Path,
    folder: &Path,
    progress: &mut dyn FnMut(&'static str, usize, usize),
) -> Result<Comparison, String> {
    let nup = nup_manifest(nup_path, &mut |done, total| progress("nup", done, total))?;
    let folder = folder_manifest(folder, &mut |done, total| progress("folder", done, total))?;
    Ok(compare_manifests(nup, folder))
}

fn compare_manifests(nup: Manifest, folder: Manifest) -> Comparison {
    let exefs = compare_section(&nup.exefs, &folder.exefs);
    let romfs = compare_section(&nup.romfs, &folder.romfs);
    let equal = exefs.equal && romfs.equal;
    let mut notes = Vec::new();
    notes.extend(nup.notes.iter().cloned());
    notes.extend(folder.notes.iter().cloned());
    notes.push(
        "comparison is relative path (as written, case-sensitive) + size + SHA-256; ExeFS and RomFS are separate sections"
            .to_string(),
    );
    if equal {
        notes.push("both sections match".to_string());
    } else {
        notes.push("differences are reported from the .nup side to the folder side".to_string());
    }
    Comparison {
        format: COMPARISON_FORMAT.to_string(),
        format_version: FORMAT_VERSION,
        equal,
        nup,
        folder,
        exefs,
        romfs,
        notes,
    }
}

fn compare_section(nup: &[FileDigest], folder: &[FileDigest]) -> SectionDiff {
    let nup_by_path: BTreeMap<&str, &FileDigest> =
        nup.iter().map(|f| (f.path.as_str(), f)).collect();
    let folder_by_path: BTreeMap<&str, &FileDigest> =
        folder.iter().map(|f| (f.path.as_str(), f)).collect();
    let paths: BTreeSet<&str> = nup_by_path
        .keys()
        .chain(folder_by_path.keys())
        .copied()
        .collect();

    let mut diff = SectionDiff {
        nup_files: nup.len(),
        folder_files: folder.len(),
        nup_bytes: total_bytes(nup),
        folder_bytes: total_bytes(folder),
        ..Default::default()
    };
    for path in paths {
        match (nup_by_path.get(path), folder_by_path.get(path)) {
            (Some(a), None) => {
                diff.missing_in_folder_count += 1;
                if diff.missing_in_folder.len() < DIFF_EXAMPLES {
                    diff.missing_in_folder.push(difference(path, Some(a), None));
                }
            }
            (None, Some(b)) => {
                diff.extra_in_folder_count += 1;
                if diff.extra_in_folder.len() < DIFF_EXAMPLES {
                    diff.extra_in_folder.push(difference(path, None, Some(b)));
                }
            }
            (Some(a), Some(b)) => {
                if a.size != b.size {
                    diff.size_mismatch_count += 1;
                    if diff.size_mismatches.len() < DIFF_EXAMPLES {
                        diff.size_mismatches
                            .push(difference(path, Some(a), Some(b)));
                    }
                } else if a.sha256 != b.sha256 {
                    diff.hash_mismatch_count += 1;
                    if diff.hash_mismatches.len() < DIFF_EXAMPLES {
                        diff.hash_mismatches
                            .push(difference(path, Some(a), Some(b)));
                    }
                }
            }
            (None, None) => {}
        }
    }
    diff.examples_truncated = diff.missing_in_folder_count > diff.missing_in_folder.len()
        || diff.extra_in_folder_count > diff.extra_in_folder.len()
        || diff.size_mismatch_count > diff.size_mismatches.len()
        || diff.hash_mismatch_count > diff.hash_mismatches.len();
    diff.equal = diff.missing_in_folder_count == 0
        && diff.extra_in_folder_count == 0
        && diff.size_mismatch_count == 0
        && diff.hash_mismatch_count == 0
        && diff.nup_bytes == diff.folder_bytes;
    diff
}

fn difference(path: &str, nup: Option<&FileDigest>, folder: Option<&FileDigest>) -> Difference {
    Difference {
        path: path.to_string(),
        nup_size: nup.map(|f| f.size),
        folder_size: folder.map(|f| f.size),
        nup_sha256: nup.map(|f| f.sha256.clone()),
        folder_sha256: folder.map(|f| f.sha256.clone()),
    }
}

/// Write the complete report, including both manifests, as JSON.
pub fn write_report(path: &Path, report: &Comparison) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let file = File::create(path).map_err(|e| format!("creating {}: {e}", path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, report)
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    writer
        .flush()
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// The `--compare-data` implementation: build the report, write it, and print
/// a human-readable summary. Exit code 0 means the manifests match, 2 means
/// they differ, and 1 means the comparison could not be completed.
pub fn compare_data_cli(
    nup: &Path,
    folder: &Path,
    report_path: &Path,
) -> Result<Comparison, String> {
    let mut last = ("", 0usize);
    let report = compare_sources(nup, folder, &mut |phase, done, total| {
        if phase != last.0 {
            last = (phase, 0);
        }
        if done == total || done <= 1 || done >= last.1 + 500 {
            eprintln!("{phase}: {done}/{total} files");
            last = (phase, done);
        }
    })?;
    write_report(report_path, &report)?;

    println!(".nup:  {}", report.nup.source);
    println!(
        "  ExeFS: {} files, {} bytes",
        report.nup.exefs.len(),
        report.nup.exefs_bytes
    );
    println!(
        "  RomFS: {} files, {} bytes",
        report.nup.romfs.len(),
        report.nup.romfs_bytes
    );
    println!("folder: {}", report.folder.source);
    println!(
        "  ExeFS: {} files, {} bytes",
        report.folder.exefs.len(),
        report.folder.exefs_bytes
    );
    println!(
        "  RomFS: {} files, {} bytes",
        report.folder.romfs.len(),
        report.folder.romfs_bytes
    );
    for (label, section) in [("ExeFS", &report.exefs), ("RomFS", &report.romfs)] {
        println!(
            "{label}: {}",
            if section.equal { "match" } else { "DIFFERENT" }
        );
        if !section.equal {
            println!(
                "  missing in folder {}, extra in folder {}, size {}, hash {}",
                section.missing_in_folder_count,
                section.extra_in_folder_count,
                section.size_mismatch_count,
                section.hash_mismatch_count
            );
            for diff in section
                .missing_in_folder
                .iter()
                .chain(&section.extra_in_folder)
                .chain(&section.size_mismatches)
                .chain(&section.hash_mismatches)
                .take(10)
            {
                println!("  {}", diff.path);
            }
            if section.examples_truncated {
                println!(
                    "  (only the first {DIFF_EXAMPLES} examples of each kind are in the report)"
                );
            }
        }
    }
    println!("report: {}", report_path.display());
    if report.equal {
        println!("MATCH: the .nup and folder manifests are identical by path, size and SHA-256");
    } else {
        println!("DIFFERENT: the data sets are not identical; the report names the differences");
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ew_format::nup::Directory;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ew-manifest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_matching_folder(root: &Path, main: &[u8], data: &[u8], voice: &[u8]) {
        std::fs::create_dir_all(root.join("exefs")).unwrap();
        std::fs::create_dir_all(root.join("romfs")).unwrap();
        std::fs::write(root.join("exefs").join("main"), main).unwrap();
        std::fs::write(root.join("romfs").join("data.bin"), data).unwrap();
        std::fs::write(root.join("romfs").join("voice.bin"), voice).unwrap();
    }

    /// A tiny real `.nup` with one base package: a stored ExeFS module, one
    /// stored RomFS file, and one zstd-packed RomFS file. The bytes are
    /// deliberately not game data.
    fn write_nup(path: &Path) {
        let names = ["root", "exefs", "romfs", "main", "data.bin", "voice.bin"];
        let mut pool = Vec::new();
        let mut at = BTreeMap::new();
        for name in names {
            at.insert(name, pool.len() as u32);
            pool.extend_from_slice(name.as_bytes());
            pool.push(0);
        }
        let main = b"MAIN\n";
        let data = b"GAME\n";
        // A valid zstd frame for b"VOICE\n", generated by zstandard.
        let voice_frame: &[u8] = &[
            0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x06, 0x31, 0x00, 0x00, 0x56, 0x4f, 0x49, 0x43, 0x45,
            0x0a,
        ];
        let voice = b"VOICE\n";
        let n_pkg = 1usize;
        let n_dir = 3usize;
        let n_file = 3usize;
        let toc_size = 0x30 + n_pkg * 0x40 + n_dir * 0x18 + n_file * 0x50 + 5 * 4 + pool.len();
        let data_off = 0x80 + toc_size;
        let main_off = data_off;
        let data_file_off = main_off + main.len();
        let voice_off = data_file_off + data.len();
        let total = voice_off + voice_frame.len();

        let mut toc = vec![0u8; toc_size];
        toc[0..4].copy_from_slice(b"TOC\x01");
        toc[4..8].copy_from_slice(&(n_pkg as u32).to_le_bytes());
        toc[8..16].copy_from_slice(&(n_dir as u64).to_le_bytes());
        toc[16..24].copy_from_slice(&(n_file as u64).to_le_bytes());
        let pool_off = 0x30 + n_pkg * 0x40 + n_dir * 0x18 + n_file * 0x50 + 5 * 4;
        toc[24..32].copy_from_slice(&(pool_off as u64).to_le_bytes());
        toc[32..40].copy_from_slice(&(pool.len() as u64).to_le_bytes());

        let pkg = 0x30;
        toc[pkg..pkg + 8].copy_from_slice(&0x0400_7EF0_1F8A_E000u64.to_le_bytes());
        toc[pkg + 8..pkg + 12].copy_from_slice(&0u32.to_le_bytes());
        toc[pkg + 12..pkg + 16].copy_from_slice(&1u32.to_le_bytes());
        toc[pkg + 16..pkg + 20].copy_from_slice(&2u32.to_le_bytes());
        toc[pkg + 20..pkg + 24].copy_from_slice(&1u32.to_le_bytes());
        toc[pkg + 24] = 0; // base

        let dir_at = pkg + 0x40;
        let mut put_dir = |index: usize, name: &str, first: u32, count: u32| {
            let off = dir_at + index * 0x18;
            toc[off..off + 4].copy_from_slice(&at[name].to_le_bytes());
            toc[off + 4..off + 8].copy_from_slice(&(name.len() as u32).to_le_bytes());
            toc[off + 8..off + 12].copy_from_slice(&first.to_le_bytes());
            toc[off + 12..off + 16].copy_from_slice(&count.to_le_bytes());
        };
        put_dir(0, "root", 0, 2);
        put_dir(1, "exefs", 2, 1);
        put_dir(2, "romfs", 3, 2);

        let file_at = dir_at + n_dir * 0x18;
        let mut put_file =
            |index: usize, name: &str, bytes: &[u8], offset: u64, stored: u64, codec: u8| {
                let e = file_at + index * 0x50;
                toc[e..e + 4].copy_from_slice(&at[name].to_le_bytes());
                toc[e + 4..e + 8].copy_from_slice(&(name.len() as u32).to_le_bytes());
                toc[e + 8..e + 16].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
                toc[e + 16..e + 24].copy_from_slice(&offset.to_le_bytes());
                toc[e + 24..e + 32].copy_from_slice(&stored.to_le_bytes());
                toc[e + 32..e + 64].copy_from_slice(&Sha256::digest(bytes));
                toc[e + 64] = codec;
            };
        put_file(0, "main", main, main_off as u64, main.len() as u64, 0);
        put_file(
            1,
            "data.bin",
            data,
            data_file_off as u64,
            data.len() as u64,
            0,
        );
        put_file(
            2,
            "voice.bin",
            voice,
            voice_off as u64,
            voice_frame.len() as u64,
            1,
        );

        let child_at = file_at + n_file * 0x50;
        for (i, child) in [1u32, 2, 3, 4, 5].iter().enumerate() {
            toc[child_at + i * 4..child_at + i * 4 + 4].copy_from_slice(&child.to_le_bytes());
        }
        toc[pool_off..pool_off + pool.len()].copy_from_slice(&pool);

        let mut head = vec![0u8; 0x80];
        head[0..4].copy_from_slice(b"NUP\x01");
        head[4..6].copy_from_slice(&0x80u16.to_le_bytes());
        head[6..8].copy_from_slice(&1u16.to_le_bytes());
        head[8..16].copy_from_slice(&0x80u64.to_le_bytes());
        head[16..24].copy_from_slice(&(toc_size as u64).to_le_bytes());
        head[24..56].copy_from_slice(&Sha256::digest(&toc));
        head[56..64].copy_from_slice(&(data_off as u64).to_le_bytes());
        head[0x40..0x48].copy_from_slice(&(total as u64).to_le_bytes());
        head[0x48..0x4C].copy_from_slice(&(n_pkg as u32).to_le_bytes());

        let mut out = head;
        out.extend(toc);
        out.extend_from_slice(main);
        out.extend_from_slice(data);
        out.extend_from_slice(voice_frame);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn a_matching_nup_and_folder_are_identical() {
        let dir = temp_dir("equal");
        let nup = dir.join("game.nup");
        let folder = dir.join("dump");
        write_nup(&nup);
        write_matching_folder(&folder, b"MAIN\n", b"GAME\n", b"VOICE\n");

        let report = compare_sources(&nup, &folder, &mut |_, _, _| {}).unwrap();
        assert!(report.equal, "{report:#?}");
        assert_eq!(report.nup.exefs.len(), 1);
        assert_eq!(report.nup.romfs.len(), 2);
        assert_eq!(report.nup.exefs_bytes, report.folder.exefs_bytes);
        assert_eq!(report.nup.romfs_bytes, report.folder.romfs_bytes);
        assert!(report.exefs.equal);
        assert!(report.romfs.equal);
        assert!(report
            .nup
            .romfs
            .iter()
            .any(|f| f.path == "voice.bin" && f.size == 6));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_same_size_changed_voice_file_is_a_hash_mismatch() {
        let dir = temp_dir("changed");
        let nup = dir.join("game.nup");
        let folder = dir.join("dump");
        write_nup(&nup);
        write_matching_folder(&folder, b"MAIN\n", b"GAME\n", b"VOICE\n");
        std::fs::write(folder.join("romfs").join("voice.bin"), b"VOICE!").unwrap();

        let report = compare_sources(&nup, &folder, &mut |_, _, _| {}).unwrap();
        assert!(!report.equal);
        assert!(report.exefs.equal);
        assert!(!report.romfs.equal);
        assert_eq!(report.romfs.size_mismatch_count, 0);
        assert_eq!(report.romfs.hash_mismatch_count, 1);
        assert_eq!(report.romfs.hash_mismatches[0].path, "voice.bin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupted_nup_entry_is_refused_by_its_manifest() {
        let dir = temp_dir("corrupt");
        let nup = dir.join("game.nup");
        write_nup(&nup);
        let mut bytes = std::fs::read(&nup).unwrap();
        let at = bytes
            .windows(b"MAIN\n".len())
            .position(|w| w == b"MAIN\n")
            .expect("the fixture has the stored module");
        bytes[at] = b'X';
        std::fs::write(&nup, bytes).unwrap();

        let err = nup_manifest(&nup, &mut |_, _| {}).unwrap_err();
        assert!(err.contains("SHA-256"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_extra_romfs_files_are_reported() {
        let dir = temp_dir("missing");
        let nup = dir.join("game.nup");
        let folder = dir.join("dump");
        write_nup(&nup);
        write_matching_folder(&folder, b"MAIN\n", b"GAME\n", b"VOICE\n");
        std::fs::remove_file(folder.join("romfs").join("data.bin")).unwrap();
        std::fs::write(folder.join("romfs").join("extra.bin"), b"EXTRA").unwrap();

        let report = compare_sources(&nup, &folder, &mut |_, _, _| {}).unwrap();
        assert!(!report.equal);
        assert_eq!(report.romfs.missing_in_folder_count, 1);
        assert_eq!(report.romfs.missing_in_folder[0].path, "data.bin");
        assert_eq!(report.romfs.extra_in_folder_count, 1);
        assert_eq!(report.romfs.extra_in_folder[0].path, "extra.bin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_without_both_roots_is_refused_with_the_layout() {
        let dir = temp_dir("layout");
        std::fs::create_dir_all(dir.join("exefs")).unwrap();
        let err = folder_manifest(&dir, &mut |_, _| {}).unwrap_err();
        assert!(err.contains("exefs") && err.contains("romfs"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newest_valid_romfs_package_wins() {
        let dir = Directory {
            name: "romfs".into(),
            children: Vec::new(),
        };
        let nup = Nup {
            path: PathBuf::from("test.nup"),
            size: 0,
            packages: vec![
                Package {
                    title_id: 0x0400_7EF0_1F8A_E000,
                    root: 0,
                    exefs: 0,
                    romfs: 0,
                    version: 1,
                    kind: PackageKind::Base,
                    meta_offset: 0,
                    meta_len: 0,
                },
                Package {
                    title_id: 0x0400_7EF0_1F8A_E800,
                    root: 0,
                    exefs: u32::MAX,
                    romfs: 1,
                    version: 9,
                    kind: PackageKind::Update,
                    meta_offset: 0,
                    meta_len: 0,
                },
            ],
            dirs: vec![dir.clone(), dir],
            files: Vec::new(),
        };
        let found = best_romfs_package(&nup).expect("a RomFS package");
        assert_eq!(found.kind, PackageKind::Update);
        assert_eq!(found.version, 9);
        // The same update has no ExeFS, so the runtime's independent ExeFS
        // choice falls back to the base rather than failing the package.
        let exe = best_exefs_package(&nup).expect("an ExeFS package");
        assert_eq!(exe.kind, PackageKind::Base);
    }

    #[test]
    fn the_report_round_trips_with_both_manifests() {
        let dir = temp_dir("json");
        let nup = dir.join("game.nup");
        let folder = dir.join("dump");
        let report_path = dir.join("evidence").join("data-compare.json");
        write_nup(&nup);
        write_matching_folder(&folder, b"MAIN\n", b"GAME\n", b"VOICE\n");

        let report = compare_sources(&nup, &folder, &mut |_, _, _| {}).unwrap();
        write_report(&report_path, &report).unwrap();
        let text = std::fs::read_to_string(&report_path).unwrap();
        let read: Comparison = serde_json::from_str(&text).unwrap();
        assert_eq!(read, report);
        assert!(read.equal);
        assert!(text.contains(COMPARISON_FORMAT));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
