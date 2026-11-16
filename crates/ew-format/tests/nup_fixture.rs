//! A real `.nup` container, checked in, plus mutations of its tables.
//!
//! The fixture is 711 bytes and carries no game data: two stored files, one
//! base package and title metadata. `tests/fixtures/make_nup.py` writes it from
//! the layout in `src/nup.rs`, so the bytes can be audited instead of taken on
//! trust. The mutation test repairs the TOC hash after each edit, which makes
//! the parser walk a table it cannot dismiss as damaged before it reaches the
//! offset and count checks.

use ew_format::nup::{Nup, PackageKind};
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny.nup")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ew-nup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_checked_in_fixture_opens_and_reads() {
    let nup = Nup::open(&fixture()).expect("the checked-in fixture is a valid .nup");
    assert_eq!(nup.size, 711);
    assert_eq!(nup.packages.len(), 1);
    let pkg = nup.program_package().expect("the base package is the program");
    assert_eq!(pkg.kind, PackageKind::Base);
    assert_eq!(pkg.title_id, 0x0400_7EF0_1F8A_E000);
    assert_eq!(pkg.version, 1);

    let meta = nup.metadata(pkg).expect("metadata is inside the container");
    assert_eq!(meta.display_version().as_deref(), Some("1.0.0"));
    assert_eq!(meta.title().as_deref(), Some("Tiny Fixture"));
    assert_eq!(meta.publisher().as_deref(), Some("EmblemWeave tests"));

    assert_eq!(nup.dirs[pkg.root as usize].name, "root");
    let exefs = nup.walk(pkg.exefs);
    assert_eq!(exefs, vec![("main".to_string(), 0)]);
    assert_eq!(nup.read(exefs[0].1).unwrap(), b"MAIN\n");
    let romfs = nup.walk(pkg.romfs);
    assert_eq!(romfs, vec![("data.bin".to_string(), 1)]);
    assert_eq!(nup.read(romfs[0].1).unwrap(), b"GAME\n");
}

/// Mutate the table, repair its SHA-256, and require an error rather than a
/// panic. Truncations keep the original hash and hit the bounds checks too.
#[test]
fn malformed_tables_are_errors_not_panics() {
    let original = std::fs::read(fixture()).unwrap();
    let dir = scratch("mutations");
    let path = dir.join("case.nup");
    let mut cases = 0usize;

    let mut try_case = |bytes: &[u8], label: String| {
        std::fs::write(&path, bytes).unwrap();
        let outcome = std::panic::catch_unwind(|| Nup::open(&path));
        assert!(outcome.is_ok(), "{label} with {:#x} bytes panicked", bytes.len());
        cases += 1;
    };

    // Every 16-byte step through the superblock and TOC, at the values that
    // turn a small count into a zero or an all-ones one.
    for at in (0..original.len()).step_by(16) {
        for value in [0x00u8, 0x7f, 0x80, 0xff] {
            let mut b = original.clone();
            b[at] = value;
            try_case(&b, format!("byte {at} = {value:#x}"));
        }
    }

    // Mutate inside the TOC and repair the superblock digest, so the parser
    // reaches the tables: package, directory and file counts, pool offsets,
    // child indexes and file extents.
    for at in (0x80..original.len().min(0x300)).step_by(23) {
        for value in [0x00u8, 0x01, 0xff] {
            let mut b = original.clone();
            b[at] = value;
            let toc_size = u64::from_le_bytes(b[16..24].try_into().unwrap()) as usize;
            let digest = <sha2::Sha256 as sha2::Digest>::digest(&b[0x80..0x80 + toc_size]);
            b[24..56].copy_from_slice(&digest);
            try_case(&b, format!("TOC byte {at} = {value:#x}"));
        }
    }

    for len in (0..original.len()).step_by(11) {
        try_case(&original[..len], format!("truncated to {len}"));
    }
    assert!(cases > 100, "only {cases} mutations ran");
    let _ = std::fs::remove_dir_all(dir);
}
