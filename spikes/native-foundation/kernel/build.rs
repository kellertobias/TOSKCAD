fn main() {
    let source = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../artifacts/native-deps/OCCT-7_9_3");
    assert!(
        source.join("CMakeLists.txt").is_file(),
        "Run node tools/foundation-deps.mjs first"
    );
    let mut config = cmake::Config::new(source);
    config
        .profile("Release")
        .define("BUILD_LIBRARY_TYPE", "Static")
        .define("BUILD_ADDITIONAL_TOOLKITS", "TKDESTEP TKMesh TKBool TKPrim")
        .define("INSTALL_DIR_LIB", "lib")
        .define("INSTALL_DIR_INCLUDE", "include");
    for name in [
        "FoundationClasses",
        "ModelingData",
        "ModelingAlgorithms",
        "Visualization",
        "ApplicationFramework",
        "DataExchange",
        "Draw",
        "DETools",
    ] {
        config.define(format!("BUILD_MODULE_{name}"), "OFF");
    }
    for name in [
        "TBB",
        "FREETYPE",
        "FREEIMAGE",
        "FFMPEG",
        "VTK",
        "TCL",
        "TK",
        "OPENGL",
        "GLES2",
        "D3D",
        "EIGEN",
        "RAPIDJSON",
        "DRACO",
        "OPENVR",
        "XLIB",
    ] {
        config.define(format!("USE_{name}"), "OFF");
    }
    let root = config.build();
    cxx_build::bridge("src/lib.rs")
        .file("src/bridge.cpp")
        .include("include")
        .include(root.join("include"))
        .std("c++17")
        .compile("multicad-kernel-proof");
    println!(
        "cargo:rustc-link-search=native={}",
        root.join("lib").display()
    );
    for name in [
        "TKDESTEP",
        "TKDE",
        "TKXSBase",
        "TKShHealing",
        "TKMesh",
        "TKBool",
        "TKBO",
        "TKPrim",
        "TKTopAlgo",
        "TKGeomAlgo",
        "TKBRep",
        "TKGeomBase",
        "TKG3d",
        "TKG2d",
        "TKMath",
        "TKernel",
    ] {
        println!("cargo:rustc-link-lib=static={name}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=user32");
    }
    for path in ["src/lib.rs", "src/bridge.cpp", "include/bridge.hpp"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
