use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=ui/app.slint");
    println!("cargo:rerun-if-changed=ui/inspector.slint");
    println!("cargo:rerun-if-changed=ui/quick_settings.slint");
    println!("cargo:rerun-if-changed=ui/image_orientation.slint");
    println!("cargo:rerun-if-changed=ui/icons");
    println!("cargo:rerun-if-changed=ui/quick_editor.slint");
    println!("cargo:rerun-if-changed=ui/action_button.slint");
    println!("cargo:rerun-if-changed=ui/ip_settings.slint");
    println!("cargo:rerun-if-changed=ui/model_panel.slint");
    println!("cargo:rerun-if-env-changed=PYLON_ROOT");
    slint_build::compile("ui/app.slint").expect("Unable to compile Slint UI");

    let root = std::env::var_os("PYLON_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Basler\pylon"));
    let lib_dir = root.join("Development").join("lib").join("x64");
    let library = lib_dir.join("PylonC_v10.lib");
    assert!(
        library.is_file(),
        "Missing pylon C SDK import library: {}. Install the pylon Developer profile or set PYLON_ROOT.",
        library.display()
    );
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=PylonC_v10");
}
