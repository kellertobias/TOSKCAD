// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_multicad"))
        .args(arguments)
        .output()
        .expect("bootstrap executable should launch")
}

#[test]
fn help_and_default_entry_explain_the_bootstrap_boundary() {
    for arguments in [&[][..], &["--help"][..], &["-h"][..]] {
        let output = run(arguments);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).expect("help is UTF-8");
        assert!(text.contains("Usage: multicad"));
        assert!(text.contains("Repository bootstrap only."));
        assert!(text.contains("not implemented yet"));
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
            format!("MultiCAD {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn diagnostics_report_the_compiled_target_and_unimplemented_capabilities() {
    let output = run(&["--diagnostics"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).expect("diagnostics are UTF-8");
    assert!(text.contains(&format!(
        "Target: {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )));
    assert!(text.contains("Stage: repository-bootstrap"));
    assert!(text.contains("CAD kernel: not integrated"));
    assert!(text.contains("Native UI: not implemented"));
    assert!(text.contains("MCP server: not implemented"));
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
