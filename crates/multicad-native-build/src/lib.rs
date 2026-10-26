//! Build-time only. Immutable source + compiler/target/config identity select
//! a shared cache; Cargo check/test/build serialize its creation with a file lock.
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, OpenOptions},
    path::PathBuf,
    process::Command,
};

fn source_digest(source: &std::path::Path) -> String {
    fn files(path: &std::path::Path, result: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).expect("native source directory") {
            let entry = entry.expect("native source entry");
            let kind = entry.file_type().expect("source file type");
            assert!(
                !kind.is_symlink(),
                "Native source symlinks are not supported"
            );
            if kind.is_dir() {
                files(&entry.path(), result);
            } else if kind.is_file() {
                result.push(entry.path());
            }
        }
    }
    let mut paths = Vec::new();
    files(source, &mut paths);
    paths.sort();
    let mut hash = Sha256::new();
    for path in paths {
        hash.update(
            path.strip_prefix(source)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
                .as_bytes(),
        );
        hash.update([0]);
        hash.update(Sha256::digest(
            fs::read(path).expect("native source contents"),
        ));
    }
    format!("{:x}", hash.finalize())
}

pub const LIBRARIES: &[&str] = &[
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
];

const TOOLKITS: &str = "TKDESTEP TKMesh TKBool TKPrim";
const PROFILE: &str = "Release";
const LINK: &str = "Static";
const OFF_MODULES: &[&str] = &[
    "FoundationClasses",
    "ModelingData",
    "ModelingAlgorithms",
    "Visualization",
    "ApplicationFramework",
    "DataExchange",
    "Draw",
    "DETools",
];
const OFF_OPTIONS: &[&str] = &[
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
];

pub fn build() -> PathBuf {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root");
    let pin: serde_json::Value =
        serde_json::from_str(include_str!("../../../native-dependencies.json"))
            .expect("native dependency pin");
    let pin = &pin["occt"];
    let source = repository
        .join("artifacts/native-deps")
        .join(pin["directory"].as_str().expect("source directory"));
    let archive = repository.join("artifacts/native-deps/occt-7.9.3.tar.gz");
    let bytes = fs::read(&archive)
        .expect("Run ./build or node tools/foundation-deps.mjs to acquire OCCT source");
    let checksum = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        checksum,
        pin["sha256"].as_str().unwrap(),
        "Cached OCCT archive checksum mismatch"
    );
    assert!(
        source.join("CMakeLists.txt").is_file(),
        "OCCT source is incomplete; rerun dependency bootstrap"
    );
    let target = env::var("TARGET").expect("target triple");
    let source_hash = source_digest(&source);
    println!("cargo:rustc-env=MULTICAD_OCCT_SOURCE_HASH={source_hash}");
    println!("cargo:rerun-if-changed={}", source.display());
    // Match cmake-rs' fixed compiler setup. Native OCCT is always Release;
    // changing the Rust debug/release profile must not rebuild the same OCCT.
    let compiler = cc::Build::new()
        .cpp(true)
        .opt_level(0)
        .debug(false)
        .warnings(false)
        .get_compiler();
    let version = Command::new(compiler.path())
        .arg(if compiler.is_like_msvc() {
            "/?"
        } else {
            "--version"
        })
        .output()
        .expect("C++ compiler");
    let mut key = Sha256::new();
    for text in [
        checksum.as_bytes(),
        source_hash.as_bytes(),
        target.as_bytes(),
        TOOLKITS.as_bytes(),
        PROFILE.as_bytes(),
        LINK.as_bytes(),
        OFF_MODULES.join(";").as_bytes(),
        OFF_OPTIONS.join(";").as_bytes(),
        LIBRARIES.join(";").as_bytes(),
        compiler.path().as_os_str().as_encoded_bytes(),
        version.stdout.as_slice(),
        version.stderr.as_slice(),
        format!("{:?}", compiler.args()).as_bytes(),
    ] {
        key.update(text);
        key.update([0]);
    }
    for name in [
        "CXX",
        "CC",
        "CXXFLAGS",
        "CFLAGS",
        "SDKROOT",
        "MACOSX_DEPLOYMENT_TARGET",
        "CMAKE_GENERATOR",
        "CMAKE_TOOLCHAIN_FILE",
        "CARGO_CFG_TARGET_FEATURE",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        key.update(env::var_os(name).unwrap_or_default().as_encoded_bytes());
        key.update([0]);
    }
    let cache = repository
        .join("target/native/occt")
        .join(&target)
        .join(format!("{:x}", key.finalize()));
    fs::create_dir_all(&cache).expect("native cache directory");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cache.join("build.lock"))
        .expect("cache lock");
    lock.lock().expect("lock native build cache");
    let marker = cache.join("complete");
    let library_exists = |name: &&str| {
        cache
            .join("lib")
            .join(if target.contains("windows-msvc") {
                format!("{name}.lib")
            } else {
                format!("lib{name}.a")
            })
            .is_file()
    };
    if !marker.is_file() || !LIBRARIES.iter().all(library_exists) {
        let mut config = cmake::Config::new(&source);
        config
            .profile(PROFILE)
            .out_dir(&cache)
            .define("BUILD_LIBRARY_TYPE", LINK)
            .define("BUILD_ADDITIONAL_TOOLKITS", TOOLKITS)
            .define("INSTALL_DIR_LIB", "lib")
            .define("INSTALL_DIR_INCLUDE", "include");
        for name in OFF_MODULES {
            config.define(format!("BUILD_MODULE_{name}"), "OFF");
        }
        for name in OFF_OPTIONS {
            config.define(format!("USE_{name}"), "OFF");
        }
        config.build();
        assert!(
            LIBRARIES.iter().all(library_exists),
            "Native build did not produce required OCCT libraries"
        );
        fs::write(
            &marker,
            format!("OCCT {} {checksum}\n", pin["version"].as_str().unwrap()),
        )
        .expect("cache completion marker");
    }
    println!("cargo:rerun-if-changed={}", archive.display());
    cache
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_source_edits_change_identity_without_depending_on_checkout_path() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["a", "b"] {
            fs::create_dir_all(directory.path().join(name)).unwrap();
            fs::write(directory.path().join(name).join("source.cpp"), "source").unwrap();
        }
        assert_eq!(
            source_digest(&directory.path().join("a")),
            source_digest(&directory.path().join("b"))
        );
        fs::write(directory.path().join("b/source.cpp"), "modified source").unwrap();
        assert_ne!(
            source_digest(&directory.path().join("a")),
            source_digest(&directory.path().join("b"))
        );
    }
}
