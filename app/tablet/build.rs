use std::path::PathBuf;
use std::{env, fs};

fn main() {
    if env::var_os("CARGO_FEATURE_TAKEOVER").is_none() {
        return;
    }

    println!("cargo:rerun-if-env-changed=REMARQUE_UI_FONT");
    let font = env::var_os("REMARQUE_UI_FONT")
        .map(PathBuf::from)
        .or_else(|| {
            [
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.exists())
        })
        .expect("set REMARQUE_UI_FONT to a readable TrueType font");
    println!("cargo:rerun-if-changed={}", font.display());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("ui-font.ttf");
    fs::copy(font, output).expect("copy Remarque UI font");

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
