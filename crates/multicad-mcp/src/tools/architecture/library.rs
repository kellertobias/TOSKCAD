use super::*;
fn library(host: &dyn Host, args: &LibrarySource) -> Result<symbols::SymbolDefinition, ToolError> {
    use multicad_core::project_file::{self, FileLimits};
    let path = super::imports::scoped(host, &args.file_name, &[".tcd"])?;
    let path = path
        .canonicalize()
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    let folder = host
        .scope()
        .export_directory
        .ok_or_else(|| ToolError::invalid("No exchange folder"))?
        .canonicalize()
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    if path.parent() != Some(folder.as_path()) {
        return Err(ToolError::new(
            "out_of_scope",
            "Library must remain inside the configured exchange folder",
        ));
    }
    let mut file = std::fs::File::open(path).map_err(|e| ToolError::invalid(e.to_string()))?;
    let opened = project_file::read(
        &mut file,
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .map_err(|e| ToolError::invalid(e.to_string()))?;
    let selected = args.source_definition.as_deref().map(id).transpose()?;
    let mut definitions = Vec::new();
    for (key, definition) in symbols::definitions(&opened.project.document)? {
        if selected.is_none_or(|id| id == key) && blueprint_for_definition(&definition)?.is_some() {
            definitions.push(definition);
        }
    }
    if definitions.len() != 1 {
        return Err(ToolError::invalid(
            "Choose exactly one architectural source definition from the library file",
        ));
    }
    Ok(definitions.remove(0))
}
pub(in crate::tools) fn import(host: &dyn Host, call: &Request, args: ImportArgs) -> ToolResult {
    let definition = library(host, &args.source)?;
    let revision = args.expected_revision;
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        Ok::<_, ToolError>(symbols::prepare_import(
            s,
            revision,
            owned.actor.clone(),
            &definition,
        )?)
    })??;
    let definition = prepared.definition;
    finish(
        host,
        call,
        prepared.prepared,
        revision,
        json!({"definition":definition,"native_geometry_checked":false,"geometry_scope":"Embedded construction source only; draw an instance to check receiving geometry"}),
    )
}
fn update_request(
    doc: &Document,
    params: &EvaluatedParameters,
    args: CompareArgs,
    replacement: symbols::SymbolDefinition,
) -> Result<symbols::SymbolUpdateRequest, ToolError> {
    let definition = id(&args.definition)?;
    if blueprint_for_definition(&symbols::read_definition(doc, definition)?)?.is_none() {
        return Err(ToolError::invalid("Choose an architectural definition"));
    }
    let mut request = symbols::SymbolUpdateRequest::new(doc, definition, replacement)?;
    for (key, value) in args.inputs {
        let key = id(&key)?;
        let previous = request.architecture_inputs.get(&key).ok_or_else(|| {
            ToolError::invalid("Override repair does not name an affected architectural record")
        })?;
        let value = value.value(doc, params, Some(previous))?;
        request.architecture_inputs.insert(key, value);
    }
    for (key, value) in args.contexts {
        let key = id(&key)?;
        if !request.architecture_inputs.contains_key(&key) {
            return Err(ToolError::invalid(
                "Context repair does not name an affected architectural record",
            ));
        }
        let previous = read_architecture_instance(doc, key)?;
        request
            .architecture_contexts
            .insert(key, value.value(doc, params, Some(&previous))?);
    }
    Ok(request)
}
pub(in crate::tools) fn compare(host: &dyn Host, _: &Request, args: CompareArgs) -> ToolResult {
    let replacement = library(host, &args.source)?;
    let (doc, revision, params) = snapshot(host)?;
    let request = update_request(&doc, &params, args, replacement)?;
    let report = symbols::compare_update_request(&doc, &request)?;
    Ok(
        json!({"revision":revision,"report":report,"native_geometry_checked":false,"status":"Source comparison only; no pin or receiving geometry changed"}),
    )
}
pub(in crate::tools) fn update(host: &dyn Host, call: &Request, args: UpdateArgs) -> ToolResult {
    let replacement = library(host, &args.update.source)?;
    let revision = args.expected_revision;
    let owned = call.clone();
    let (prepared, report) = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        let request = update_request(s.document(), s.parameters(), args.update, replacement)?;
        let report = symbols::compare_update_request(s.document(), &request)?;
        let prepared = symbols::prepare_update(s, revision, owned.actor.clone(), &request)?;
        Ok::<_, ToolError>((prepared.prepared, report))
    })??;
    finish(
        host,
        call,
        prepared,
        revision,
        json!({"native_geometry_checked":!report.affected_instances.is_empty(),"report":report}),
    )
}
