fn main() {
    let root = multicad_native_build::build();
    cxx_build::bridge("src/lib.rs")
        .file("src/bridge.cpp")
        .file("src/curves.cpp")
        .file("src/ruled.cpp")
        .file("src/analytic.cpp")
        .file("src/drawing.cpp")
        .file("src/distance.cpp")
        .file("src/planar_contours.cpp")
