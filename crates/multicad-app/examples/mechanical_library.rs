//! Build the curated offline hardware files using the canonical/native APIs.
//! `cargo run -p multicad-app --example mechanical_library -- library`
//! Existing files are refused. Every identity and preview is reproducible;
//! dimension changes require an explicit new symbol/catalog revision.
#[allow(dead_code)]
#[path = "cabinet_demo.rs"]
mod cabinet_demo;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, MechanicalPartParameters, MechanicalPartProvenance, MechanicalPartRequest, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use multicad_model::Compiler;
use serde::Deserialize;
use std::{io::Write, path::Path};
pub type Error = Box<dyn std::error::Error>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curation {
    pub format: String,
    pub version: u32,
    pub revision: String,
    pub data_source: String,
    pub data_revision: String,
    pub data_license: String,
    pub data_sha256: String,
    pub verified_on: String,
    pub scope: String,
    pub entries: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub file: String,
    pub designation: String,
    pub bore_mm: f64,
    pub outside_mm: f64,
    pub width_mm: f64,
    pub protection: String,
    pub verification_url: String,
}
pub fn curated() -> Result<Curation, Error> {
    let value: Curation = serde_json::from_str(include_str!("../../../library/curation.json"))?;
    if value.format != "toskcad.hardware-curation"
        || value.version != 1
        || value.entries.is_empty()
        || value.entries.len() > 256
        || value.data_license != "LGPL-2.1-or-later"
        || value.data_sha256
            != project_file::ContentHash::of(include_bytes!(
                "../../../library/sources/bolts/bearings.blt"
            ))
            .to_string()
        || value.scope.is_empty()
    {
        return Err("Unsupported curated hardware source".into());
    }
    Ok(value)
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn identity(entry: &Entry, role: usize) -> Result<EntityId, Error> {
    let designation = u64::from(entry.designation.parse::<u32>()?);
    Ok(EntityId::parse(&format!(
        "cad20000-0000-4000-8000-{:012x}",
        designation * 100 + role as u64
    ))?)
}
fn literal(entry: &Entry) -> MechanicalPartParameters {
    MechanicalPartParameters::BearingEnvelope {
        bore_diameter: length(entry.bore_mm),
        outside_diameter: length(entry.outside_mm),
        width: length(entry.width_mm),
        frame: Default::default(),
    }
}
pub fn build(curation: &Curation, index: usize) -> Result<ProjectFile, Error> {
    let entry = curation.entries.get(index).ok_or("Missing curated entry")?;
    if entry.protection != "open" || entry.file != format!("bearing-open-{}.tcd", entry.designation)
    {
        return Err("Unsupported bearing variant or filename".into());
    }
    let name = format!("{} open · bearing mounting envelope", entry.designation);
    // Use the same checked source decode as saved projects to initialize the
    // catalog's fixed document identity; all subsequent changes are typed.
    let document: Document = serde_json::from_value(serde_json::json!({
        "schema_version":multicad_core::document::DOCUMENT_SCHEMA,
        "id":identity(entry,0)?, "name":name, "entities":{},
    }))?;
    let mut session = Session::new(document)?;
    let mut exposed = Vec::new();
    let mut edits = Vec::new();
    for (role, name, value) in [
        (1, "bore", entry.bore_mm),
        (2, "outside", entry.outside_mm),
        (3, "width", entry.width_mm),
    ] {
        let mut entity = Entity::new(
            name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        entity.id = identity(entry, role)?;
        exposed.push(entity.id);
        edits.push(Mutation::Insert(entity));
    }
    session.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Bearing envelope dimensions".into(),
        edits,
    })?;
    let dimension = |name| -> Result<Dimension, Error> {
        Ok(Dimension::Expression {
            expression: BoundExpression::bind(
                session.document(),
                &format!("=${name}"),
                "mm",
                None,
            )?,
        })
    };
    let mut request = MechanicalPartRequest::new(
        &name,
        MechanicalPartParameters::BearingEnvelope {
            bore_diameter: dimension("bore")?,
            outside_diameter: dimension("outside")?,
            width: dimension("width")?,
            frame: Default::default(),
        },
    );
    request.feature = identity(entry, 4)?;
    request.body = identity(entry, 5)?;
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: format!("toskcad.bolts.{}", entry.id),
        revision: format!("{} / curated {}", curation.data_revision, curation.revision),
        designation: name.clone(),
        source_url: curation.data_source.clone(),
        reference_geometry: literal(entry),
    });
    let prepared = model::prepare_mechanical_part(&session, session.revision(), actor(), &request)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision())?;
    session.commit(prepared.prepared)?;
    let mut author =
        SymbolAuthorRequest::new(&name, request.body, SymbolKind::Dynamic, exposed, None);
    author.definition = identity(entry, 6)?;
    author.symbol = identity(entry, 7)?;
    author.revision = curation.revision.clone();
    let prepared = symbols::prepare_author(&session, session.revision(), actor(), &author)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision())?;
    session.commit(prepared.prepared)?;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let preview =
        cabinet_demo::render_preview_with_color(session.document(), &model, false, |_| {
            [0.62, 0.64, 0.68]
        })?;
    let info = FileInfo {
        name,
        description: format!(
            "{} open bearing: {} × {} × {} mm external mounting envelope. Bore, outside diameter and width are editable instance inputs. Races, rolling elements, seals and fillets are omitted. Dimensions from BOLTS {} (LGPL-2.1-or-later), cross-checked {} at {}. The retained dimensions do not certify fits, loads or current-standard compliance.",
            entry.designation,
            entry.bore_mm,
            entry.outside_mm,
            entry.width_mm,
            curation.data_revision,
            curation.verified_on,
            entry.verification_url
        ),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = identity(entry, 8)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Bearing library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}
fn main() -> Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Usage: mechanical_library OUTPUT_FOLDER")?;
    let root = Path::new(&root);
    std::fs::create_dir_all(root)?;
    let curation = curated()?;
    let mut entries = Vec::new();
    for (index, entry) in curation.entries.iter().enumerate() {
        let project = build(&curation, index)?;
        let mut bytes = Vec::new();
        project_file::write(&project, &mut bytes, Default::default())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(&entry.file))?;
        file.write_all(&bytes)?;
        entries.push(serde_json::json!({"file":entry.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()}));
        println!("{} · {} bytes", entry.file, bytes.len());
    }
    let manifest = serde_json::json!({"format":"toskcad.bundled-library","version":1,"revision":curation.revision,"entries":entries,"requires":["curation.json","sources/bolts/bearings.blt","sources/bolts/COPYING.LESSER","README.md"]});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("library.json"))?;
    file.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}
