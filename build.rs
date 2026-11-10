//! Link the Windows import libraries and application icon, and set the app version.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let vendor = root.join("vendor/mingw-import-libs");
    assert!(vendor.is_dir(), "Missing vendor/mingw-import-libs");
    println!("cargo:rustc-link-search=native={}", vendor.display());

    // Regenerate with: windres -O coff -I assets -i assets/app.rc -o assets/app.o
    let icon = root.join("assets/app.o");
    assert!(icon.is_file(), "Missing assets/app.o");
    println!("cargo:rustc-link-arg={}", icon.display());

    let version = std::env::var("ROG_BUILD_VERSION")
        .ok().filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    println!("cargo:rustc-env=ROG_APP_VERSION={}", version.trim());
    println!("cargo:rerun-if-env-changed=ROG_BUILD_VERSION");
    for path in ["build.rs", "Cargo.toml", "vendor/mingw-import-libs", "assets/app.o", "assets/app.rc", "assets/tray.ico"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
