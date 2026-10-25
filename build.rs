// Fails early with a clear message when the embedded fonts have not been fetched,
// and gives the Windows .exe its icon (packaging/icon.ico, through MinGW's windres).
fn main() {
    windows_icon();
    for f in ["NotoSans-Regular.ttf", "DejaVuSans.ttf"] {
        let p = std::path::Path::new("assets/fonts").join(f);
        println!("cargo:rerun-if-changed={}", p.display());
        if !p.exists() {
            panic!("{} is missing. Run tools/fetch-assets.sh first (fonts are not kept in the repository).", p.display());
        }
    }
}

fn windows_icon() {
    println!("cargo:rerun-if-changed=packaging/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = std::fs::canonicalize("packaging/icon.ico").unwrap();
    let rc = out.join("icon.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "/"))).unwrap();
    let obj = out.join("icon.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
    let ok = std::process::Command::new(&windres)
        .args(["--input-format=rc", "--output-format=coff", "-i"])
        .arg(&rc)
        .arg("-o")
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("could not run {windres} for the Windows icon: {e}"));
    if !ok.success() {
        panic!("{windres} failed on {}", rc.display());
    }
    println!("cargo:rustc-link-arg-bins={}", obj.display());
}
