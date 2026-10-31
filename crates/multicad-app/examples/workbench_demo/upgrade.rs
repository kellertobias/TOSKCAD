//! Upgrade the shipped project's source while retaining its project/part identities.
use super::*;
pub fn run(input: &std::path::Path, output: &std::path::Path) -> Result<(), Error> {
    if output.exists() {
        return Err("The upgrade output must be a new file".into());
    }
    let mut doc = project_file::read(
        &mut std::fs::File::open(input)?,
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project
    .document;
    if doc.entities().any(|e| e.name == "Vise fixed jaw") {
        return Err("This project already has the vise".into());
    }
    doc.rename(DOCUMENT_NAME)?;
    let mut session = Session::new(doc)?;
    let parameters = [
        parameter("vise_opening", "80", "Clear distance between the vise jaws"),
        parameter(
            "vise_center_x",
            "=$top_length / 5",
            "Vise center along the front edge",
        ),
        parameter("vise_jaw_width", "240", "Width of the two beech vise jaws"),
    ];
    let mut edits: Vec<_> = parameters.iter().cloned().map(Mutation::Insert).collect();
    let mut table=session.document().entities().find(|e| matches!(&e.data,EntityData::Extension {definition,..} if definition.type_id=="core.parameter_table")).ok_or("Parameter table missing")?.clone();
    if let EntityData::Extension { definition, inputs } = &mut table.data {
        let mut rows: Vec<EntityId> = serde_json::from_value(definition.data.clone())?;
        rows.extend(parameters.iter().map(|p| p.id));
        definition.data = serde_json::to_value(rows)?;
        inputs.extend(parameters.iter().map(|p| Link::new(p.id, "row")));
    }
    edits.push(Mutation::Replace(table));
    let p = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Add vise parameters".into(),
        edits,
    })?;
    checked(&mut session, p)?;
    let named = |s: &Session, n: &str| -> Result<EntityId, Error> {
        Ok(s.document()
            .entities()
            .find(|e| e.name == n)
            .ok_or_else(|| format!("Missing {n}"))?
            .id)
    };
    let stock = named(&session, "Beech block 40")?;
    let steel = named(&session, "S235 steel")?;
    let beech = named(&session, "Beech")?;
    let vise = vise::build(&mut session, stock, steel, beech)?;
    let book = multicad_core::drawings::books(session.document())
        .next()
        .ok_or("Drawing book missing")?;
    let mut book_entity = session
        .document()
        .get(book)
        .ok_or("Drawing book missing")?
        .clone();
    book_entity.name = DOCUMENT_NAME.into();
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Rename workbench drawing book".into(),
        edits: vec![Mutation::Replace(book_entity)],
    })?;
    session.commit(prepared)?;
    let mut assembly = page(&session, book, PAGES[0])?;
    assembly.info.project = DOCUMENT_NAME.into();
    assembly.info.author = "ToskCAD".into();
    for view in &mut assembly.views {
        if let ViewSource::Model { entities } = &mut view.source {
            entities.extend(&vise);
        }
    }
    let p = prepare(
        &session,
        session.revision(),
        actor(),
        book,
        PageAction::Replace(assembly),
    )?;
    checked(&mut session, p)?;
    refresh_tables(&mut session, book)?;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let mut parts: Vec<_> = model
        .member_cut_list(session.document())?
        .into_iter()
        .map(|r| r.part)
        .collect();
    parts.push(named(&session, "Worktop")?);
    parts.extend(vise);
    let image = render_preview(&session, &parts)?;
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
        label: "Update workbench information and preview".into(),
        edits,
    })?;
    checked(&mut session, p)?;
    save(&session, book, output)?;
    println!(
        "Upgraded {} parts; preserved project identity; all drawing layouts clear",
        parts.len()
    );
    Ok(())
}
pub fn refresh(input: &std::path::Path, output: &std::path::Path) -> Result<(), Error> {
    if output.exists() {
        return Err("Output must be a new file".into());
    }
    let doc = project_file::read(
        &mut std::fs::File::open(input)?,
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project
    .document;
    let mut session = Session::new(doc)?;
    let book = multicad_core::drawings::books(session.document())
        .next()
        .ok_or("Drawing book missing")?;
    refresh_tables(&mut session, book)?;
    save(&session, book, output)
}
fn save(session: &Session, book: EntityId, output: &std::path::Path) -> Result<(), Error> {
    let report = page_report(session, book)?;
    if let Some((number, conflicts)) = report.iter().find(|(_, c)| !c.is_empty()) {
        let p = page(session, book, number)?;
        for a in &p.annotations {
            if conflicts.iter().any(|c| c.contains(&a.id.to_string())) {
                match &a.kind {
                    AnnotationKind::Table {
                        title,
                        rows,
                        position_mm,
                        ..
                    } => eprintln!(
                        "Conflicting table {title}: {} rows at {position_mm:?}, bounds {:?}",
                        rows.len(),
                        table_bounds(a)
                    ),
                    AnnotationKind::Note { text, position_mm } => {
                        eprintln!("Conflicting note at {position_mm:?}: {text}")
                    }
                    _ => eprintln!("Conflicting callout {}", a.id),
                }
            }
        }
        return Err(format!("{number}: {conflicts:?}").into());
    }
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?
        .write_all(&bytes)?;
    Ok(())
}
