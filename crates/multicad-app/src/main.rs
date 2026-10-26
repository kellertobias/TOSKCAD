// SPDX-License-Identifier: GPL-3.0-or-later

use std::{env, ffi::OsString, process::ExitCode};

use multicad_core::BuildInfo;

enum Action {
    Help,
    Version,
    Diagnostics,
}

fn parse_action(mut arguments: impl Iterator<Item = OsString>) -> Result<Action, String> {
    let Some(argument) = arguments.next() else {
        return Ok(Action::Help);
    };
    if arguments.next().is_some() {
        return Err("expected at most one option; see --help".into());
    }
    match argument.to_str() {
        Some("--help" | "-h") => Ok(Action::Help),
        Some("--version" | "-V") => Ok(Action::Version),
        Some("--diagnostics") => Ok(Action::Diagnostics),
        _ => Err(format!(
            "unsupported argument {:?}; see --help",
            argument.to_string_lossy()
        )),
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
    match action {
        Action::Help => println!(
            "{} {}\n\nUsage: multicad [--help | --version | --diagnostics]\n\n\
             Repository bootstrap only.\n\
             Native CAD UI, geometry kernel and MCP server are not implemented yet.",
            build.product, build.version
        ),
        Action::Version => println!("{} {}", build.product, build.version),
        Action::Diagnostics => println!(
            "{} {}\n\
             Stage: repository-bootstrap\n\
             Target: {}/{}\n\
             Source commit: {}\n\
             CAD kernel: not integrated\n\
             Native UI: not implemented\n\
             MCP server: not implemented\n\
             Runtime: compiled Rust; no Python or Node.js runtime dependency",
            build.product,
            build.version,
            build.target_os,
            build.target_arch,
            build.commit.unwrap_or("not stamped")
        ),
    }
    ExitCode::SUCCESS
}
