use std::fs;
use std::path::PathBuf;

fn compile_scss(style_dir: &PathBuf) {
    let scss_files = ["settings-window", "screenshot", "locker"];

    for name in scss_files {
        let scss_path = style_dir.join(format!("{}.scss", name));
        let css_path = style_dir.join(format!("{}.css", name));

        if scss_path.exists() {
            println!("cargo:rerun-if-changed={}", scss_path.display());

            let css = grass::from_path(&scss_path, &grass::Options::default())
                .unwrap_or_else(|e| panic!("Failed to compile {}: {}", scss_path.display(), e));

            fs::write(&css_path, css)
                .unwrap_or_else(|e| panic!("Failed to write {}: {}", css_path.display(), e));
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root_dir = manifest_dir.join("..").join("..");
    let data_dir = root_dir.join("data");
    let style_dir = data_dir.join("style");
    let gresource = data_dir.join("swaysettings.gresource.xml");

    // Compile SCSS to CSS first
    compile_scss(&style_dir);

    println!("cargo:rerun-if-changed={}", gresource.display());
    println!("cargo:rerun-if-changed={}", style_dir.display());
    println!("cargo:rerun-if-changed={}", data_dir.join("ui").display());
    println!(
        "cargo:rerun-if-changed={}",
        data_dir.join("icons").display()
    );

    let gresource_str = gresource.to_string_lossy().to_string();
    glib_build_tools::compile_resources(&[data_dir], &gresource_str, "swaysettings.gresource");
}
