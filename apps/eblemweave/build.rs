//! Build metadata and Windows executable branding.
//!
//! Records the git revision for the About page and, on Windows, embeds the
//! EmblemWeave icon and version information into the executable.

fn main() {
    let rev = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=EW_GIT_REV={rev}");
    // HEAD names the branch; the branch ref (or packed-refs) moves on every commit.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Ok(head) = std::fs::read_to_string("../../.git/HEAD") {
        if let Some(r) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=../../.git/{r}");
        }
    }
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "EmblemWeave")
            .set("FileDescription", "EmblemWeave: native PC recompilation launcher")
            .set("CompanyName", "XiØN")
            .set("LegalCopyright", "XiØN. Not affiliated with Nintendo or Intelligent Systems.");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows resources: {e}");
        }
    }
}
