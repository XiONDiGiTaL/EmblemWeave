//! ew-nup: list a `.nup` game container, or copy its ExeFS out.
//!
//! usage: ew-nup <file.nup>                 list packages and the ExeFS
//!        ew-nup <file.nup> --exefs <dir>   write the program's ExeFS to <dir>
//!
//! Only reads the container; every file is checked against its SHA-256.

use ew_format::nup::Nup;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: ew-nup <file.nup> [--exefs <dir>]");
        std::process::exit(2);
    };
    if let Err(e) = run(path, args.get(1).map(String::as_str), args.get(2)) {
        eprintln!("ew-nup: {e:#}");
        std::process::exit(1);
    }
}

fn run(path: &str, flag: Option<&str>, out: Option<&String>) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let nup = Nup::open(std::path::Path::new(path))?;
    println!("{} bytes, {} directories, {} files, read in {:?}", nup.size, nup.dirs.len(), nup.files.len(), started.elapsed());
    for p in &nup.packages {
        println!("package {:016X} v{} {:?}: root {}, exefs {}, romfs {}", p.title_id, p.version, p.kind, p.root, p.exefs, p.romfs);
    }
    let pkg = nup.program_package().ok_or_else(|| anyhow::anyhow!("no base or update package"))?;
    let meta = nup.metadata(pkg)?;
    println!("title: {} {} ({})", meta.title().unwrap_or_default(), meta.display_version().unwrap_or_default(), meta.publisher().unwrap_or_default());
    let exefs = nup.walk(pkg.exefs);
    for (name, i) in &exefs {
        let e = &nup.files[*i as usize];
        println!("  exefs/{name:<16} {:>12} bytes ({} stored{})", e.size, e.stored, if e.compressed { ", zstd" } else { "" });
    }
    println!("romfs: {} files", nup.walk(pkg.romfs).len());
    if flag == Some("--exefs") {
        let dir = std::path::Path::new(out.ok_or_else(|| anyhow::anyhow!("--exefs needs a directory"))?);
        std::fs::create_dir_all(dir)?;
        for (name, i) in &exefs {
            std::fs::write(dir.join(name), nup.read(*i)?)?;
            println!("wrote {}", dir.join(name).display());
        }
    }
    Ok(())
}
