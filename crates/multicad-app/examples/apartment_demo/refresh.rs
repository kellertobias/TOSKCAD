//! Explicitly refresh bundled references, retaining the apartment and placement IDs.
use super::*;
pub fn run(input: &std::path::Path, output: &std::path::Path) -> Result<(), Error> {
    if output.exists() {
        return Err("The output must be a new file".into());
    }
    let doc = project_file::read(
        &mut std::fs::File::open(input)?,
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project
    .document;
    let mut session = Session::new(doc)?;
    for (id, source) in symbols::project_sources(session.document())? {
        let path = input
            .parent()
            .ok_or("Input folder missing")?
            .join(&source.path);
        let p = project_references::prepare_reference_refresh(
            &session,
            session.revision(),
            actor(),
            id,
            Some(input),
            &path,
        )?;
        checked(&mut session, p.prepared)?;
    }
    let book = drawings::books(session.document())
        .next()
        .ok_or("Drawing book missing")?;
    let mut page = drawings::read(session.document(), book)?
        .pages
        .into_iter()
        .find(|p| p.number == PAGES[2])
        .ok_or("Reference drawing missing")?;
    let mut lines = vec![note([20., 20.], REFERENCE_NOTE)];
    for (i, label) in ["Workbench", "Cabinet"].into_iter().enumerate() {
        let instance = session
            .document()
            .entities()
            .find(|e| e.name == label && symbols::read_instance(session.document(), e.id).is_ok())
            .ok_or("Reference instance missing")?;
        let record = symbols::read_instance(session.document(), instance.id)?;
        let definition = symbols::read_definition(session.document(), record.definition)?;
        let source = symbols::project_source_of(session.document(), record.definition)?
            .ok_or("Reference source missing")?
            .1;
        lines.push(note(
            [20., 28. + 6. * i as f64],
            &format!(
                "{label}: {} · {} · revision {}",
                definition.name, source.path, definition.revision
            ),
        ));
    }
    page.annotations = lines;
    let p = prepare(
        &session,
        session.revision(),
        actor(),
        book,
        PageAction::Replace(page),
    )?;
    checked(&mut session, p)?;
    let image = render_document_preview(&session)?;
    let edits = project_file::info::edits(
        session.document(),
        &FileInfo {
            name: DOCUMENT_NAME.into(),
            description: DESCRIPTION.into(),
            preview: Some(multicad_ui::preview_capture::preview_from(&image)?),
            capture_on_save: true,
        },
    )?;
    let p = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Refresh apartment reference information and preview".into(),
        edits,
    })?;
    checked(&mut session, p)?;
    let checks = project_references::check_references(session.document(), Some(input), &[])?;
    for check in &checks {
        if !matches!(check.state, symbols::ProjectSourceState::Current) {
            return Err(format!("Reference is not current: {}", check.summary()).into());
        }
    }
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?
        .write_all(&bytes)?;
    println!(
        "Refreshed {} references; retained project, room and instance identities",
        checks.len()
    );
    Ok(())
}
