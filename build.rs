// Fails early with a clear message when the embedded fonts have not been fetched.
fn main() {
    for f in ["NotoSans-Regular.ttf", "DejaVuSans.ttf"] {
        let p = std::path::Path::new("assets/fonts").join(f);
        println!("cargo:rerun-if-changed={}", p.display());
        if !p.exists() {
            panic!("{} is missing. Run tools/fetch-assets.sh first (fonts are not kept in the repository).", p.display());
        }
    }
}
