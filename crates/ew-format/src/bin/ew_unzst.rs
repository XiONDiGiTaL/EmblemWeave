//! Decompress a `.zst` file from the game's RomFS, for inspection.
//!
//! usage: ew_unzst <input.zst> <output>
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: ew_unzst <input.zst> <output>");
        std::process::exit(2);
    }
    let input = std::fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("reading {}: {e}", args[1]);
        std::process::exit(1)
    });
    let mut out = Vec::new();
    let result = ruzstd::StreamingDecoder::new(std::io::Cursor::new(input)).map_err(|e| e.to_string()).and_then(|mut d| d.read_to_end(&mut out).map_err(|e| e.to_string()));
    if let Err(e) = result {
        eprintln!("decompressing {}: {e}", args[1]);
        std::process::exit(1);
    }
    if let Err(e) = std::fs::write(&args[2], &out) {
        eprintln!("writing {}: {e}", args[2]);
        std::process::exit(1);
    }
    println!("{} bytes", out.len());
}
