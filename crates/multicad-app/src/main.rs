// SPDX-License-Identifier: GPL-3.0-or-later
mod app_control;
mod assembly_inspection;
mod drawings;
mod exchange;
mod external_queries;
mod file_picker;
mod files;
mod inspection;
mod library;
mod mcp;
mod mechanical_imports;
mod modeling;
mod print;
mod recent_projects;
mod reference_repairs;
mod self_test;

use multicad_core::BuildInfo;
use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

enum Action {
    Run,
    Help,
    Version,
    Diagnostics,
    SelfTest(PathBuf),
    Mcp(mcp::Options),
}
fn parse_action(arguments: impl Iterator<Item = OsString>) -> Result<Action, String> {
    let arguments: Vec<_> = arguments.collect();
    if arguments.first().is_some_and(|a| a == "--mcp") {
        return mcp::parse(&arguments[1..]).map(Action::Mcp);
    }
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
        Action::Run => run_application().inspect_err(|error| record_startup_error(error)),
        Action::Help => {
            println!(
                "{} {}\n\nUsage: multicad [--help | --version | --diagnostics | --self-test OUTPUT_DIRECTORY]\n       multicad --mcp [--project FILE.multicad] [--export-dir DIRECTORY] [--read-only]\n       multicad --mcp --app [--socket PATH]\n\nLaunches the native application host with no arguments.\nIncludes Parameters, constrained Sketch and initial New body extrusion. Native .multicad containers include atomic saves, embedded payloads, explicit JSON import and recovery copies. Advanced modeling and manufacturing exports remain under development.\n\n--mcp serves the Model Context Protocol on stdin/stdout for a local client that launches this process. It controls one headless document: the --project file (opened, or created on first save), else an unsaved document. Exports write only into --export-dir. --mcp --app instead bridges to the open window once \"Allow local MCP connections\" is on (macOS and Linux; same-user local socket). No network port is opened.",
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
                "{} {}\nStage: native-integration\nTarget: {}/{}\nSource commit: {}\nCAD kernel: OCCT 7.9.3 (statically linked)\nOCCT source: {}\nSketch solver: sketch-solver 0.1.0\nNative UI: eframe 0.36.2 / wgpu\nCAD workspaces: Parameters; constrained Sketch; initial New body extrusion\nNative project format: versioned .multicad container; atomic saves and explicit recovery copies\nMCP server: local stdio (multicad --mcp), rmcp 3.8.0\nRuntime: compiled Rust and native libraries; no Python or Node.js dependency",
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
        Action::Mcp(options) => mcp::run(options),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("multicad: {error}");
            ExitCode::FAILURE
        }
    }
}
/// Finder launches have no attached terminal. Retain a private temporary
/// diagnostic so startup failures remain inspectable instead of disappearing.
fn record_startup_error(error: &str) {
    use std::io::Write;
    if let Ok(mut log) = tempfile::Builder::new()
        .prefix("multicad-startup-")
        .suffix(".log")
        .tempfile()
        && writeln!(log, "{error}").is_ok()
    {
        let _ = log.keep();
    }
}
fn run_application() -> Result<(), String> {
    let mut app =
        multicad_ui::application::Application::welcome(Box::new(files::ProjectStore::default()))?;
    app.set_recent_store(Box::new(recent_projects::RecentFileStore::default()));
    app.set_library_store(Box::new(library::LibraryFileStore::default()));
    app.set_sketch_queries(std::sync::Arc::new(|| {
        Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
    }));
    let models: multicad_ui::viewport::BackendFactory =
        std::sync::Arc::new(|| Box::new(modeling::Backend::new()));
    app.set_scene_backends(std::sync::Arc::clone(&models));
    // The tool controller uses its own worker and publication lane so a
    // candidate can never replace canonical geometry under the same revision.
    app.set_model_backends(models);
    app.set_external_query_backends(std::sync::Arc::new(|| {
        Ok(Box::new(external_queries::Backend::new()))
    }))?;
    app.set_repair_set_checker(std::sync::Arc::new(reference_repairs::check));
    app.set_measurement_backends(std::sync::Arc::new(|| Box::new(inspection::Backend::new())));
    app.set_assembly_query_backends(std::sync::Arc::new(|| {
        Box::new(assembly_inspection::Backend::new())
    }));
    app.set_drawing_backends(std::sync::Arc::new(|| Box::new(drawings::Backend::new())));
    app.set_mechanical_import_backend(std::sync::Arc::new(mechanical_imports::Backend));
    app.set_export_backends(std::sync::Arc::new(|| Box::new(exchange::Backend::new())))?;
    app.set_print_backends(std::sync::Arc::new(|| Box::new(print::Backend::new())))?;
    multicad_ui::launch_with(Box::new(move |context| {
        app.set_file_picker(Box::new(file_picker::NativePicker::new(context)));
        app.set_external_control(Box::new(app_control::WindowControl::new(
            context.egui_ctx.clone(),
            app_control::default_endpoint(),
        )));
        Ok(Box::new(app))
    }))
}
