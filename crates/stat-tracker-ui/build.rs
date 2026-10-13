//! Embeds `SST_GUI_BUILD_ID`: a hash over the inputs compiled into the GUI
//! (alpha.7 G3). See `src/build_id_hash.rs` for what counts. The release
//! version and timestamps do not, so a daemon-only release keeps the id and
//! a running GUI does not ask to restart.

#[path = "src/build_id_hash.rs"]
mod build_id_hash;

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"),
    );
    let inputs = build_id_hash::workspace_inputs(&manifest_dir);
    let built = match build_id_hash::compute(&inputs) {
        Ok(built) => built,
        Err(e) => panic!("GUI build id: {e}"),
    };
    for path in &built.watched {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    println!("cargo::rustc-env=SST_GUI_BUILD_ID={}", built.id);
}
