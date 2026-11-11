//! ew-decode: decompress an LZ4 NSO0 module into text.bin, rodata.bin and data.bin.
//!
//! Reads the user's own module and writes the segments to a directory; it never
//! modifies the input. ZBIC modules go through `tools/zbic_extract.py`.

use ew_format::nso::{Nso, SegmentKind};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: ew-decode <module> <out-dir>");
        std::process::exit(2);
    }
    let bytes = match std::fs::read(&args[0]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("reading {}: {e}", args[0]);
            std::process::exit(1);
        }
    };
    let run = || -> anyhow::Result<()> {
        let nso = Nso::parse(&bytes)?;
        let out = std::path::Path::new(&args[1]);
        std::fs::create_dir_all(out)?;
        for (kind, file) in [(SegmentKind::Text, "text.bin"), (SegmentKind::Rodata, "rodata.bin"), (SegmentKind::Data, "data.bin")] {
            let seg = nso.decompress_segment(kind)?;
            std::fs::write(out.join(file), &seg)?;
            println!("{:<7} {} bytes", kind.as_str(), seg.len());
        }
        println!("module  {} bss 0x{:x}", nso.header.module_name, nso.header.bss_size);
        Ok(())
    };
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
