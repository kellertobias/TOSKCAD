//! Generate owned generic planting presets in the existing native Library.
#[allow(dead_code)]
#[path = "support/native_preview.rs"]
mod native_preview;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{EntityId, Mutation},
    plants::{self, PlantKind},
    project_file::{self, ProjectFile, info::FileInfo},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use multicad_model::Compiler;
pub use native_preview::render_preview_with_color;
use std::{io::Write, path::Path};
pub type Error = Box<dyn std::error::Error>;
pub fn build(kind: PlantKind) -> Result<ProjectFile, Error> {
    let definition = plants::generic_definition(kind)?;
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
    let prefix = if kind == PlantKind::Tree {
        0x1000
    } else {
        0x2000
    };
    let identity =
        |role| EntityId::parse(&format!("cad20200-0000-4000-8000-{:012x}", prefix + role));
    author.definition = identity(60)?;
    session.commit(
        symbols::prepare_author(&session, session.revision(), actor.clone(), &author)?.prepared,
    )?;
    let retained = symbols::read_definition(session.document(), author.definition)?;
    if retained.pin()? != definition.pin()? {
        return Err("Plant Library and creation tool do not share the same preset pin".into());
    }
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let preview = render_preview_with_color(session.document(), &model, false, |name| {
        if name.to_ascii_lowercase().contains("trunk") {
            [0.35, 0.23, 0.14]
        } else {
            [0.28, 0.48, 0.26]
        }
    })?;
    let info=FileInfo{name:definition.name.clone(),description:"Generic editable planting context. Height and outer canopy/spread are instance inputs. Planned/mature defaults are illustrative, not species dimensions or growth predictions. Simple 3D envelopes and authored monochrome plan circles share one retained pin and placement. Trees use two rings; shrubs use one. Transparent plan linework preserves buildings and paths underneath. Created for ToskCAD; no third-party artwork.".into(),preview:Some(preview),capture_on_save:true};
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = identity(70)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor,
        label: "Plant Library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}
fn main() -> Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Usage: plant_library OUTPUT_FOLDER")?;
    let root = Path::new(&root);
    std::fs::create_dir_all(root)?;
    for (kind, name) in [
        (PlantKind::Tree, "plant-generic-tree.tcd"),
        (PlantKind::Shrub, "plant-generic-shrub.tcd"),
    ] {
        let project = build(kind)?;
        let mut bytes = vec![];
        project_file::write(&project, &mut bytes, Default::default())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(name))?;
        file.write_all(&bytes)?;
        println!(
            "{}",
            serde_json::json!({"file":name,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()})
        );
    }
    Ok(())
}
