//! One-shot command transport over the same canonical MCP dispatcher.
use multicad_core::commands::{Actor, Channel};
use multicad_mcp::tools::{Call, catalog, dispatch};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Debug, PartialEq)]
pub(crate) struct Options {
    request: PathBuf,
    project: PathBuf,
    read_only: bool,
    rust_model: bool,
    save: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    tool: String,
    arguments: Value,
}
pub(crate) fn parse(arguments: &[OsString]) -> Result<Options, String> {
    let Some((request, arguments)) = arguments.split_first() else {
        return Err("--command needs a JSON request file".into());
    };
    if request.to_string_lossy().starts_with('-') {
        return Err("--command needs a JSON request file before options".into());
    }
    let mut project = None;
    let mut read_only = false;
    let mut rust_model = false;
    let mut save = false;
    let mut rest = arguments.iter();
    while let Some(arg) = rest.next() {
        match arg.to_str() {
            Some("--project") if project.is_none() => {
                project = Some(PathBuf::from(rest.next().ok_or("--project needs a path")?));
            }
            Some("--read-only") if !read_only => read_only = true,
            Some("--rust-model") if !rust_model => rust_model = true,
            Some("--save") if !save => save = true,
            _ => {
                return Err(format!(
                    "Unsupported --command argument {:?}",
                    arg.to_string_lossy()
                ));
            }
        }
    }
    let project = project.ok_or("--command requires --project FILE.tcd")?;
    if !multicad_core::project_path::is_project_path(&project) {
        return Err("--project must name a .tcd (or legacy .multicad) file".into());
    }
    if save && read_only {
        return Err("--save conflicts with --read-only".into());
    }
    Ok(Options {
        request: request.into(),
        project,
        read_only,
        rust_model,
        save,
    })
}

pub(crate) fn run(options: Options) -> Result<(), String> {
    let outcome = execute(options);
    match outcome {
        Ok(receipt) => {
            println!("{receipt}");
            Ok(())
        }
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            Err(error.to_string())
        }
    }
}
fn failure(message: impl ToString) -> Value {
    json!({"code":"command_failed","message":message.to_string()})
}
fn execute(options: Options) -> Result<Value, Value> {
    // Decode before opening the project, so malformed envelopes cannot alter it.
    let envelope: Envelope =
        serde_json::from_slice(&std::fs::read(&options.request).map_err(failure)?)
            .map_err(|e| failure(format!("Invalid command envelope: {e}")))?;
    let tool = catalog()
        .into_iter()
        .find(|tool| tool.name == envelope.tool)
        .ok_or_else(
            || json!({"code":"unknown_tool","message":format!("Unknown tool '{}'", envelope.tool)}),
        )?;
    let host = crate::mcp::headless(
        crate::mcp::Options {
            project: Some(options.project),
            read_only: options.read_only,
            rust_model: options.rust_model,
            ..Default::default()
        },
        options.save,
    )
    .map_err(failure)?;
    let call = Call {
        actor: Actor::local(Channel::CommandLine),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let result = dispatch(&host, &call, &envelope.tool, envelope.arguments)
        .map_err(|error| serde_json::to_value(&error).expect("ToolError serializes"))?;
    let saved = if options.save && !tool.read_only {
        let info = dispatch(&host, &call, "document_info", json!({}))
            .map_err(|error| serde_json::to_value(error).expect("ToolError serializes"))?;
        let revision = info["revision"]
            .as_u64()
            .ok_or_else(|| failure("Document information omitted revision"))?;
        Some(
            dispatch(
                &host,
                &call,
                "save_project",
                json!({"expected_revision":revision}),
            )
            .map_err(|error| serde_json::to_value(&error).expect("ToolError serializes"))?,
        )
    } else {
        None
    };
    Ok(json!({"ok":true,"tool":envelope.tool,"result":result,
        "persistence":if saved.is_some(){"saved"}else if tool.read_only{"read_only"}else{"memory_only"},"save":saved}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn persistence_and_scope_are_explicit() {
        let options = parse(&args(&["request.json", "--project", "owned.tcd"])).unwrap();
        assert!(!options.save && !options.read_only);
        assert!(!options.rust_model);
        assert!(
            parse(&args(&[
                "request.json",
                "--project",
                "owned.tcd",
                "--rust-model"
            ]))
            .unwrap()
            .rust_model
        );
        for invalid in [
            vec![],
            vec!["request.json"],
            vec!["--save"],
            vec![
                "request.json",
                "--project",
                "owned.tcd",
                "--save",
                "--read-only",
            ],
            vec!["request.json", "--project", "owned.tcd", "--save", "--save"],
            vec!["request.json", "--project", "owned.step"],
            vec!["request.json", "--project", "owned.tcd", "--app"],
            vec![
                "request.json",
                "--project",
                "owned.tcd",
                "--rust-model",
                "--rust-model",
            ],
        ] {
            assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
        }
    }
}
