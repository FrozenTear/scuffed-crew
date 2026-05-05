use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("bundled_portraits.rs");

    let portraits_dir = Path::new("portraits");
    let mut entries = Vec::new();

    if portraits_dir.exists() {
        if let Ok(read_dir) = fs::read_dir(portraits_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("png") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let abs_path = fs::canonicalize(&path).unwrap();
                        entries.push((stem.to_string(), abs_path.display().to_string()));
                    }
                }
            }
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut code = String::new();
    code.push_str("/// Auto-generated bundled hero portraits.\n");
    code.push_str("/// To update: add/replace PNGs in crates/stat-tracker/portraits/ and rebuild.\n");
    code.push_str("pub fn bundled_portraits() -> &'static [(&'static str, &'static [u8])] {\n");
    code.push_str("    &[\n");

    for (name, path) in &entries {
        code.push_str(&format!(
            "        (\"{name}\", include_bytes!(\"{path}\")),\n"
        ));
    }

    code.push_str("    ]\n");
    code.push_str("}\n");

    fs::write(&dest_path, code).unwrap();

    println!("cargo::rerun-if-changed=portraits");
}
