// SPDX-License-Identifier: GPL-3.0-or-later
mod app_control;
mod assembly_inspection;
mod bundled_examples;
mod bundled_library;
mod command;
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
mod preferences;
mod print;
mod recent_projects;
mod reference_repairs;
mod self_test;
mod sheet_pattern;

use multicad_core::BuildInfo;
use multicad_model::GeometryBackend;
use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

enum Action {
    /// Launch the window, optionally opening one native project file.
    Run(Option<PathBuf>),
    RunRustModel(Option<PathBuf>),
    Help,
    Version,
    Diagnostics,
    SelfTest(PathBuf),
    Mcp(mcp::Options),
    Command(command::Options),
}
fn parse_action(arguments: impl Iterator<Item = OsString>) -> Result<Action, String> {
    let arguments: Vec<_> = arguments.collect();
    if arguments.first().is_some_and(|a| a == "--command") {
        return command::parse(&arguments[1..]).map(Action::Command);
    }
    if arguments.first().is_some_and(|a| a == "--mcp") {
        return mcp::parse(&arguments[1..]).map(Action::Mcp);
    }
    match arguments.as_slice() {
        [] => Ok(Action::Run(None)),
        [option] => match option.to_str() {
            Some("--help" | "-h") => Ok(Action::Help),
            Some("--version" | "-V") => Ok(Action::Version),
            Some("--diagnostics") => Ok(Action::Diagnostics),
            Some("--rust-model") => Ok(Action::RunRustModel(None)),
            // File associations on Windows and Linux pass the document path.
            Some(path)
                if !path.starts_with('-')
                    && multicad_core::project_path::is_project_path(std::path::Path::new(path)) =>
            {
                Ok(Action::Run(Some(PathBuf::from(path))))
            }
            _ => Err(format!(
                "unsupported argument {:?}; see --help",
                option.to_string_lossy()
            )),
        },
        [option, path] if option == "--self-test" => Ok(Action::SelfTest(PathBuf::from(path))),
        [option, path]
            if option == "--rust-model"
                && path.to_str().is_some_and(|path| {
                    !path.starts_with('-')
                        && multicad_core::project_path::is_project_path(std::path::Path::new(path))
                }) =>
        {
            Ok(Action::RunRustModel(Some(PathBuf::from(path))))
        }
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
        Action::Run(project) => run_application(project, GeometryBackend::OpenCascade)
            .inspect_err(|error| record_startup_error(error)),
        Action::RunRustModel(project) => run_application(project, GeometryBackend::Rust)
            .inspect_err(|error| record_startup_error(error)),
        Action::Help => {
            println!(
                "{} {}\n\nUsage: multicad [PROJECT.tcd | --help | --version | --diagnostics | --self-test OUTPUT_DIRECTORY]\n       multicad --rust-model [PROJECT.tcd]\n       multicad --mcp [--project FILE.tcd] [--export-dir DIRECTORY] [--read-only] [--rust-model]\n       multicad --mcp --app [--socket PATH]\n       multicad --command REQUEST.json --project FILE.tcd [--read-only] [--save] [--rust-model]\n\nLaunches the native application host with no arguments, or opens PROJECT.tcd in it.\nIncludes Project, constrained Sketch and initial New body extrusion. Native .tcd project containers include atomic saves, embedded payloads, explicit JSON import and recovery copies; existing .multicad projects still open and save in place. Advanced modeling and manufacturing exports remain under development.\n\n--rust-model selects the experimental Rust geometry backend for the window or headless commands. STEP import embeds the source in the project and supports single solids in millimeters, including straight-profile sweeps, orthogonal solids, cylindrical pockets and circular annuli.\n\n--mcp serves the Model Context Protocol on stdin/stdout for a local client that launches this process. It controls one headless document: the --project file (opened, or created on first save), else an unsaved document. Exports write only into --export-dir. --mcp --app instead bridges to the open window once \"Allow local MCP connections\" is on (macOS and Linux; same-user local socket). The bridge follows the window’s selected geometry backend. No network port is opened.\n\n--command dispatches one JSON {{tool,arguments}} through the same canonical MCP/native validation path. It prints one JSON receipt and exits. Changes remain memory-only unless --save is present; failed commands never save. --read-only refuses mutations.",
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
                "{} {}\nStage: native-integration\nTarget: {}/{}\nSource commit: {}\nCAD kernel: OCCT 7.9.3 (statically linked)\nOCCT source: {}\nSketch solver: sketch-solver 0.1.0\nNative UI: eframe 0.36.2 / wgpu\nCAD workspaces: Project; constrained Sketch; initial New body extrusion\nNative project format: versioned .tcd container (legacy .multicad accepted); atomic saves and explicit recovery copies\nMCP server: local stdio (multicad --mcp), rmcp 3.8.0\nRuntime: compiled Rust and native libraries; no Python or Node.js dependency",
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
        Action::Command(options) => command::run(options),
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
fn run_application(project: Option<PathBuf>, model_backend: GeometryBackend) -> Result<(), String> {
    let mut app =
        multicad_ui::application::Application::welcome(Box::new(files::ProjectStore::default()))?;
    app.request_open_documents(project);
    app.set_recent_store(Box::new(recent_projects::RecentFileStore::default()));
    app.set_library_store(Box::new(library::LibraryFileStore::default()));
    app.set_example_library(bundled_examples::library());
    app.set_preference_store(Box::new(preferences::PreferenceFileStore::default()));
    app.set_sketch_queries(std::sync::Arc::new(move || {
        let queries = match model_backend {
            GeometryBackend::OpenCascade => multicad_kernel::NativeRegionQueries::default(),
            GeometryBackend::Rust => {
                multicad_kernel::NativeRegionQueries::analytic(Default::default())
                    .map_err(|error| error.to_string())?
            }
        };
        Ok(Box::new(queries))
    }));
    // One detached store survives project/editor/connection changes in this app.
    let pair_proofs = multicad_model::PairCycleProofStore::default();
    let models = multicad_model::with_pair_cycle_proofs(pair_proofs.clone(), || {
        modeling::Backend::factory(model_backend)
    })?;
    app.set_scene_backends(std::sync::Arc::clone(&models));
    // Preview and canonical lanes share a model worker while their request
    // stamps, cancellation and authoritative publications stay separate.
    app.set_model_backends(models);
    app.set_external_query_backends(std::sync::Arc::new(move || {
        Ok(Box::new(match model_backend {
            GeometryBackend::OpenCascade => external_queries::Backend::new(),
            GeometryBackend::Rust => {
                external_queries::Backend::with_geometry_backend(model_backend)
            }
        }))
    }))?;
    app.set_repair_set_checker(std::sync::Arc::new(
        move |snapshot, set, resources, cancel| {
            reference_repairs::check_with_geometry_backend(
                model_backend,
                snapshot,
                set,
                resources,
                cancel,
            )
        },
    ));
    let proofs = pair_proofs.clone();
    app.set_measurement_backends(std::sync::Arc::new(move || {
        multicad_model::with_pair_cycle_proofs(proofs.clone(), || {
            Box::new(inspection::Backend::with_geometry_backend(model_backend))
        })
    }));
    let proofs = pair_proofs.clone();
    app.set_assembly_query_backends(std::sync::Arc::new(move || {
        multicad_model::with_pair_cycle_proofs(proofs.clone(), || {
            Box::new(assembly_inspection::Backend::with_geometry_backend(
                model_backend,
            ))
        })
    }));
    let proofs = pair_proofs.clone();
    app.set_drawing_backends(std::sync::Arc::new(move || {
        multicad_model::with_pair_cycle_proofs(proofs.clone(), || {
            Box::new(match model_backend {
                GeometryBackend::OpenCascade => drawings::Backend::new(),
                GeometryBackend::Rust => drawings::Backend::with_geometry_backend(model_backend),
            })
        })
    }));
    app.set_mechanical_import_backend(std::sync::Arc::new(
        mechanical_imports::Backend::with_geometry_backend(model_backend),
    ));
    let proofs = pair_proofs.clone();
    app.set_export_backends(std::sync::Arc::new(move || {
        multicad_model::with_pair_cycle_proofs(proofs.clone(), || {
            Box::new(match model_backend {
                GeometryBackend::OpenCascade => exchange::Backend::new(),
                GeometryBackend::Rust => exchange::Backend::with_geometry_backend(model_backend),
            })
        })
    }))?;
    let proofs = pair_proofs.clone();
    app.set_print_backends(std::sync::Arc::new(move || {
        multicad_model::with_pair_cycle_proofs(proofs.clone(), || {
            Box::new(match model_backend {
                GeometryBackend::OpenCascade => print::Backend::new(),
                GeometryBackend::Rust => print::Backend::with_geometry_backend(model_backend),
            })
        })
    }))?;
    let proofs = pair_proofs.clone();
    app.set_sheet_pattern_backends(std::sync::Arc::new(move || {
        Box::new(sheet_pattern::Backend::new(proofs.clone(), model_backend))
    }));
    multicad_ui::launch_with(Box::new(move |context| {
        app.set_file_picker(Box::new(file_picker::NativePicker::new(context)));
        app.set_external_control(Box::new(multicad_model::with_pair_cycle_proofs(
            pair_proofs.clone(),
            || {
                app_control::WindowControl::new(
                    context.egui_ctx.clone(),
                    app_control::default_endpoint(),
                    model_backend,
                )
            },
        )));
        Ok(Box::new(app))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn parse(values: &[&str]) -> Result<Action, String> {
        parse_action(values.iter().map(OsString::from))
    }
    #[test]
    fn a_single_project_path_launches_the_window_with_that_document() {
        for name in ["Cabinet.tcd", "/p/UPPER.TCD", "legacy.multicad"] {
            assert!(
                matches!(parse(&[name]), Ok(Action::Run(Some(path))) if path == Path::new(name)),
                "{name}"
            );
        }
        assert!(matches!(parse(&[]), Ok(Action::Run(None))));
        for rejected in ["project.mcad", "board.step", "-x.tcd", "a.tcd.json"] {
            assert!(parse(&[rejected]).is_err(), "{rejected}");
        }
        assert!(parse(&["a.tcd", "b.tcd"]).is_err());
    }

    #[test]
    fn rust_model_launch_accepts_an_optional_native_project() {
        assert!(matches!(
            parse(&["--rust-model"]),
            Ok(Action::RunRustModel(None))
        ));
        assert!(
            matches!(parse(&["--rust-model", "Panel.tcd"]), Ok(Action::RunRustModel(Some(path))) if path == Path::new("Panel.tcd"))
        );
        assert!(parse(&["--rust-model", "part.step"]).is_err());
        assert!(parse(&["--rust-model", "-x.tcd"]).is_err());
    }
}
