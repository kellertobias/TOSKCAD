// SPDX-License-Identifier: GPL-3.0-or-later
mod files;
mod self_test;

use multicad_core::BuildInfo;
use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

enum Action {
    Run,
    Help,
    Version,
    Diagnostics,
    SelfTest(PathBuf),
}
fn parse_action(arguments: impl Iterator<Item = OsString>) -> Result<Action, String> {
    let arguments: Vec<_> = arguments.collect();
    match arguments.as_slice() {
        [] => Ok(Action::Run),
        [option] => match option.to_str() {
            Some("--help" | "-h") => Ok(Action::Help),
            Some("--version" | "-V") => Ok(Action::Version),
            Some("--diagnostics") => Ok(Action::Diagnostics),
            _ => Err(format!(
                "unsupported argument {:?}; see --help",
                option.to_string_lossy()
            )),
        },
        [option, path] if option == "--self-test" => Ok(Action::SelfTest(PathBuf::from(path))),
        _ => Err("unsupported arguments; see --help".into()),
    }
}
fn main() -> ExitCode {
    let action = match parse_action(env::args_os().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("multicad: {error}");
            return ExitCode::from(2);
        }
    };
    let build = BuildInfo::current();
    let result = match action {
        Action::Run => multicad_ui::launch_with_storage(Box::new(files::SnapshotStore)),
        Action::Help => {
            println!(
                "{} {}\n\nUsage: multicad [--help | --version | --diagnostics | --self-test OUTPUT_DIRECTORY]\n\nLaunches the native application host with no arguments.\nCAD workspaces, native project files and MCP remain under development.",
                build.product, build.version
            );
            Ok(())
        }
        Action::Version => {
            println!("{} {}", build.product, build.version);
            Ok(())
        }
        Action::Diagnostics => {
            println!(
                "{} {}\nStage: native-integration\nTarget: {}/{}\nSource commit: {}\nCAD kernel: OCCT 7.9.3 (statically linked)\nOCCT source: {}\nSketch solver: sketch-solver 0.1.0\nNative UI: eframe 0.36.2 / wgpu\nCAD workspaces: not implemented\nNative project format: not implemented\nMCP server: not implemented\nRuntime: compiled Rust and native libraries; no Python or Node.js dependency",
                build.product,
                build.version,
                build.target_os,
                build.target_arch,
                build.commit.unwrap_or("not stamped"),
                multicad_kernel::source_identity()
            );
            Ok(())
        }
        Action::SelfTest(path) => self_test::run(&path).map_err(|e| e.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("multicad: {error}");
            ExitCode::FAILURE
        }
    }
}
