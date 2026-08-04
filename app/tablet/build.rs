use std::path::PathBuf;
use std::{env, fs};

const QIMAGE_EXTERNAL_C1: &str = "_ZN6QImageC1EPhiixNS_6FormatEPFvPvES2_";
const QIMAGE_EXTERNAL_C2: &str = "_ZN6QImageC2EPhiixNS_6FormatEPFvPvES2_";

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

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let sysroot = PathBuf::from(
        env::var_os("SDKTARGETSYSROOT")
            .expect("source the firmware-matched SDK before building takeover"),
    );
    let include = sysroot.join("usr/include");
    let epaper = manifest.join("paper-pro-epaper");
    println!("cargo:rerun-if-env-changed=SDKTARGETSYSROOT");
    println!(
        "cargo:rerun-if-changed={}",
        epaper.join("paper_pro_epaper.cpp").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        epaper.join("paper_pro_epaper.h").display()
    );
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file(epaper.join("paper_pro_epaper.cpp"))
        .include(&epaper)
        .include(&include)
        .include(include.join("QtCore"))
        .include(include.join("QtGui"))
        .compile("paper_pro_epaper");

    println!("cargo:rustc-link-arg=-Wl,--export-dynamic-symbol={QIMAGE_EXTERNAL_C1}");
    println!("cargo:rustc-link-arg=-Wl,--export-dynamic-symbol={QIMAGE_EXTERNAL_C2}");
    println!("cargo:rustc-link-lib=dylib=Qt6Gui");
    println!("cargo:rustc-link-lib=dylib=Qt6Core");
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rustc-link-lib=dylib=pdfium");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/home/root/remarque/lib:/usr/lib/plugins/scenegraph");
    println!(
        "cargo:rustc-link-arg=-Wl,-rpath-link,{}/usr/lib",
        sysroot.display()
    );
}
