// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_multicad"))
        .args(arguments)
        .output()
        .expect("bootstrap executable should launch")
}

#[test]
fn help_explains_the_native_host_boundary() {
    for arguments in [&["--help"][..], &["-h"][..]] {
        let output = run(arguments);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).expect("help is UTF-8");
        assert!(text.contains("Usage: multicad"));
        assert!(text.contains("Launches the native application host"));
        assert!(text.contains("under development"));
    }
}

#[test]
fn version_is_machine_readable_and_has_no_extra_diagnostics() {
    for option in ["--version", "-V"] {
        let output = run(&[option]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).expect("version is UTF-8"),
            format!(
                "ToskCAD {}\n",
                option_env!("MULTICAD_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
            )
        );
    }
}

#[test]
fn diagnostics_report_the_compiled_target_and_capabilities() {
    let output = run(&["--diagnostics"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).expect("diagnostics are UTF-8");
    assert!(text.contains(&format!(
        "Target: {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )));
    assert!(text.contains("Stage: native-integration"));
    assert!(text.contains(&format!(
        "Source commit: {}",
        option_env!("MULTICAD_BUILD_COMMIT").unwrap_or("not stamped")
    )));
    assert!(text.contains("CAD kernel: OCCT 7.9.3"));
    assert!(text.contains("Native UI: eframe 0.36.2 / wgpu"));
    assert!(text.contains("MCP server: local stdio (multicad --mcp)"));
}

#[test]
fn packaged_runtime_smoke_emits_real_step_and_reports_output_errors() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_multicad"))
        .arg("--self-test")
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Debug builds fall back to the repository examples; a package checks
    // its own Resources/resources copy the same way.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Bundled examples: ") && stdout.contains("C-channel front panel"),
        "{stdout}"
    );
    let step = std::fs::read_to_string(directory.path().join("native-smoke.step")).unwrap();
    assert!(step.starts_with("ISO-10303-21;"));
    assert!(step.contains("MANIFOLD_SOLID_BREP"));
    let file = directory.path().join("existing-file");
    std::fs::write(&file, b"preserve").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_multicad"))
        .arg("--self-test")
        .arg(&file)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(file).unwrap(), b"preserve");
}

#[test]
fn unsupported_inputs_fail_without_printing_success_output() {
    for arguments in [
        &["--unknown"][..],
        &["project.mcad"][..],
        &["--version", "--diagnostics"][..],
    ] {
        let output = run(arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).expect("error is UTF-8");
        assert!(error.starts_with("multicad: "));
        assert!(error.contains("--help"));
    }
}

#[cfg(unix)]
#[test]
fn non_unicode_arguments_are_reported_instead_of_panicking() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let output = Command::new(env!("CARGO_BIN_EXE_multicad"))
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .expect("bootstrap executable should launch");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported argument"));
}
