use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root_dir = manifest_dir.join("..").join("..");
    let data_dir = root_dir.join("data");
    let gresource = data_dir.join("swaysettings.gresource.xml");

    println!("cargo:rerun-if-changed={}", gresource.display());
    println!("cargo:rerun-if-changed={}", data_dir.join("style").display());
    println!("cargo:rerun-if-changed={}", data_dir.join("ui").display());
    println!("cargo:rerun-if-changed={}", data_dir.join("icons").display());

    let gresource_str = gresource.to_string_lossy().to_string();
    glib_build_tools::compile_resources(&[data_dir], &gresource_str, "swaysettings.gresource");
}
