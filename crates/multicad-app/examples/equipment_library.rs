//! Generate owned generic apartment equipment in the existing native Library.
#[allow(dead_code)]
#[path = "plant_library.rs"]
pub mod plant_library;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{EntityId, Mutation},
    equipment::{self, Kind},
    project_file::{self, ProjectFile, info::FileInfo},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use std::{io::Write, path::Path};
pub type Error = Box<dyn std::error::Error>;
pub fn build(kind: Kind) -> Result<ProjectFile, Error> {
    let definition = equipment::generic_definition(kind)?;
    let mut session = Session::new(definition.template.clone())?;
    let actor = Actor::local(Channel::Mcp);
    let mut author = SymbolAuthorRequest::new(
        &definition.name,
        definition.root,
        SymbolKind::Dynamic,
        definition.exposed.clone(),
        definition.plan.clone(),
    );
    author.symbol = definition.symbol;
    let prefix = (Kind::ALL
        .iter()
        .position(|k| *k == kind)
        .expect("catalog kind") as u64
        + 1)
        * 10000;
    let identity =
        |role| EntityId::parse(&format!("cad20300-0000-4000-8000-{:012x}", prefix + role));
    author.definition = identity(60)?;
    session.commit(
        symbols::prepare_author(&session, session.revision(), actor.clone(), &author)?.prepared,
    )?;
    let retained = symbols::read_definition(session.document(), author.definition)?;
    if retained.pin()? != definition.pin()? {
        return Err("Equipment Library and placement tool do not share the same preset pin".into());
    }
    let preview =
        plant_library::render_symbol_preview(session.document(), author.definition, |name| {
            let name = name.to_ascii_lowercase();
            if name.contains("burner")
                || name.contains("door")
                || name.contains("port")
                || name.contains("control")
            {
                [0.16, 0.19, 0.22]
            } else if name.contains("glass") || name.contains("diffuser") {
                [0.65, 0.78, 0.82]
            } else {
                [0.78, 0.80, 0.81]
            }
        })?;
    let info = FileInfo {
        name: definition.name.clone(),
        description: format!(
            "{} · {} · generic editable planning envelope. Width/depth/height are instance inputs. Simple native 3D forms and authored monochrome plan footprints share one retained pin. ToskCAD generic v1 symbols; electrical marks retain an explicit paper size. No manufacturer data, connector fit geometry, circuit design or installation certification. Created for ToskCAD; no third-party geometry or artwork.",
            kind.preset().category.label(),
            kind.search_terms()
        ),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = identity(70)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor,
        label: "Equipment Library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}
fn main() -> Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Usage: equipment_library OUTPUT_FOLDER")?;
    let root = Path::new(&root);
    std::fs::create_dir_all(root)?;
    for kind in Kind::ALL {
        let name = file_name(kind);
        let project = build(kind)?;
        let mut bytes = vec![];
        project_file::write(&project, &mut bytes, Default::default())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(&name))?;
        file.write_all(&bytes)?;
        println!(
            "{}",
            serde_json::json!({"file":name,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()})
        );
    }
    Ok(())
}

pub fn file_name(kind: Kind) -> String {
    format!(
        "equipment-{}.tcd",
        serde_json::to_value(kind)
            .expect("catalog kind")
            .as_str()
            .expect("kind name")
            .replace('_', "-")
    )
}
