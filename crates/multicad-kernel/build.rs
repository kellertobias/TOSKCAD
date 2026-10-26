fn main() {
    let root = multicad_native_build::build();
    cxx_build::bridge("src/lib.rs")
        .file("src/bridge.cpp")
        .include("include")
        .include(root.join("include"))
        .std("c++17")
        .compile("multicad-kernel");
    println!(
        "cargo:rustc-link-search=native={}",
        root.join("lib").display()
    );
    for name in multicad_native_build::LIBRARIES {
        println!("cargo:rustc-link-lib=static={name}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=user32");
    }
    for path in ["src/lib.rs", "src/bridge.cpp", "include/bridge.hpp"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
