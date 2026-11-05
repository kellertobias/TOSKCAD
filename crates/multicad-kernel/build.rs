fn main() {
    let root = multicad_native_build::build();
    let mut build = cxx_build::bridge("src/lib.rs");
    // The adapter throws C++ exceptions; MSVC needs unwind semantics for them.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.flag("/EHsc");
    }
    build
        .file("src/bridge.cpp")
        .file("src/curves.cpp")
        .file("src/ruled.cpp")
        .file("src/analytic.cpp")
        .file("src/drawing.cpp")
        .file("src/distance.cpp")
        .file("src/planar_contours.cpp")
        .file("src/advanced.cpp")
        .file("src/sketch_references.cpp")
        .file("src/threads.cpp")
        .file("src/gears.cpp")
        .file("src/bevel.cpp")
        .file("src/worm.cpp")
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
        // OCCT's OSD layer uses the registry and file-security APIs.
        println!("cargo:rustc-link-lib=advapi32");
    }
    for path in [
        "src/lib.rs",
        "src/bridge.cpp",
        "include/bridge.hpp",
        "src/curves.cpp",
        "include/curves.hpp",
        "src/ruled.cpp",
        "include/ruled.hpp",
        "include/measure.hpp",
        "src/analytic.cpp",
        "include/analytic.hpp",
        "src/drawing.cpp",
        "include/drawing.hpp",
        "src/distance.cpp",
        "include/distance.hpp",
        "src/planar_contours.cpp",
        "include/planar_contours.hpp",
        "src/advanced.cpp",
        "include/advanced.hpp",
        "src/sketch_references.cpp",
        "include/sketch_references.hpp",
        "src/threads.cpp",
        "include/threads.hpp",
        "src/gears.cpp",
        "include/gears.hpp",
        "src/bevel.cpp",
        "include/bevel.hpp",
        "src/worm.cpp",
        "include/worm.hpp",
        "include/worm_verification.hpp",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}
