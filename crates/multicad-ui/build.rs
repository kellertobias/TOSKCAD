fn main() {
    println!("cargo:rerun-if-changed=src/native_window.rs");
    println!("cargo:rerun-if-changed=src/native_window.mm");
    println!("cargo:rerun-if-changed=src/native_window.hpp");
    println!("cargo:rerun-if-changed=src/native_documents.mm");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cxx_build::bridge("src/native_window.rs")
            .file("src/native_window.mm")
            .file("src/native_documents.mm")
            .include("src")
            .flag("-std=c++17")
            .flag("-fobjc-arc")
            .compile("multicad-window");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
