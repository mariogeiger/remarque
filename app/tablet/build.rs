use std::env;
use std::path::PathBuf;

fn main() {
    if env::var_os("CARGO_FEATURE_TAKEOVER").is_none() {
        return;
    }

    let repository = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let quill = env::var_os("REMARQUE_QUILL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join(".build/quill"));
    println!(
        "cargo:rustc-link-search=native={}",
        quill.join("build").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        quill.join("vendor").display()
    );
    println!("cargo:rustc-link-lib=dylib=quill");
    println!("cargo:rustc-link-lib=dylib=qsgepaper");
    println!("cargo:rustc-link-lib=dylib=pdfium");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/home/root/remarque/lib:/usr/lib/plugins/scenegraph");
    if let Some(sysroot) = env::var_os("SDKTARGETSYSROOT") {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath-link,{}/usr/lib",
            PathBuf::from(sysroot).display()
        );
    }
}
