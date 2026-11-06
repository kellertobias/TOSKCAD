//! Build the curated offline hardware files using the canonical/native APIs.
//! `cargo run -p multicad-app --example mechanical_library -- /tmp/library-review`
//! Add `--machine-screws` after the folder to extend it with sourced screws.
//! Use `--hex-nuts` for edition-specific nuts with retained thread choices.
//! Existing files are refused. Every identity and preview is reproducible;
//! dimension changes require an explicit new symbol/catalog revision.
#[allow(dead_code)]
#[path = "equipment_library.rs"]
mod equipment_library;
#[path = "support/machine_screw_catalog.rs"]
mod machine_screw_catalog;
#[path = "support/nut_catalog.rs"]
pub mod nut_catalog;
use equipment_library::plant_library;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, MechanicalPartCatalogFacts, MechanicalPartParameters, MechanicalPartProvenance,
        MechanicalPartRequest, length,
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
        catalog_facts: None,
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
    let preview =
        plant_library::render_symbol_preview(session.document(), author.definition, |_| {
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasherCuration {
    pub format: String,
    pub version: u32,
    pub revision: String,
    pub data_source: String,
    pub data_revision: String,
    pub data_license: String,
    pub data_sha256: String,
    pub verified_on: String,
    pub scope: String,
    pub entries: Vec<WasherEntry>,
    pub coverage: Vec<WasherCoverage>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasherEntry {
    pub id: String,
    pub file: String,
    pub designation: String,
    pub nominal_mm: f64,
    pub bore_mm: f64,
    pub outside_mm: f64,
    pub thickness_mm: f64,
    pub series: String,
    pub material: Option<String>,
    pub grade: Option<String>,
    pub verification_url: String,
    pub verification_reference: String,
    pub correction: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasherCoverage {
    pub nominal_mm: f64,
    pub status: String,
    pub entry_id: Option<String>,
    pub reason: String,
}
fn nominal_key(nominal: f64) -> Result<u64, Error> {
    let scaled = nominal * 1000.;
    if !scaled.is_finite()
        || !(1. ..=100_000.).contains(&scaled)
        || (scaled.round() - scaled).abs() > 1e-8
    {
        return Err(
            "Washer nominal needs an explicit positive millimeter fact with micrometer precision"
                .into(),
        );
    }
    Ok(scaled.round() as u64)
}
pub fn curated_washers() -> Result<WasherCuration, Error> {
    let value: WasherCuration =
        serde_json::from_str(include_str!("../../../library/washers-curation.json"))?;
    if value.format != "toskcad.washer-curation"
        || value.version != 1
        || value.revision != "1"
        || value.entries.len() != 10
        || value.coverage.len() != 12
        || value.data_license != "LGPL-2.1-or-later"
        || value.data_revision != "5c83300a57761f70db87f50c9a1f647c7278a4c2"
        || value.data_sha256
            != project_file::ContentHash::of(include_bytes!(
                "../../../library/sources/bolts/washer.blt"
            ))
            .to_string()
        || value.scope.is_empty()
        || value.verified_on.is_empty()
    {
        return Err("Unsupported curated washer source or coverage".into());
    }
    let mut keys = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    let validation = Session::new(Document::new("Washer source validation")?)?;
    for entry in &value.entries {
        let key = nominal_key(entry.nominal_mm)?;
        let slug = entry.nominal_mm.to_string().replace('.', "-");
        if !keys.insert(key)
            || !ids.insert(&entry.id)
            || entry.id != format!("washer.normal.m{slug}")
            || entry.file != format!("washer-normal-m{slug}.tcd")
            || entry.designation != format!("M{}", entry.nominal_mm)
            || entry.series != "normal"
            || entry.material.is_some()
            || entry.grade.is_some()
            || !entry.verification_url.starts_with("https://")
            || entry.verification_reference.is_empty()
        {
            return Err("Invalid washer identity, series or source facts".into());
        }
        washer_literal(entry).resolve(validation.parameters())?;
        if (entry.nominal_mm == 10.) != entry.correction.is_some() {
            return Err("The reviewed M10 correction must be retained explicitly".into());
        }
    }
    let mut covered = std::collections::BTreeSet::new();
    for row in &value.coverage {
        let key = nominal_key(row.nominal_mm)?;
        if !covered.insert(key) || row.reason.is_empty() {
            return Err("Washer coverage needs unique explicit nominals and reasons".into());
        }
        let entry = value
            .entries
            .iter()
            .find(|e| e.nominal_mm == row.nominal_mm);
        match (row.status.as_str(), entry, row.entry_id.as_deref()) {
            ("supported", Some(entry), Some(id)) if id == entry.id => {}
            ("unavailable", None, None) if matches!(key, 4500 | 9000) => {}
            _ => return Err("Washer coverage disagrees with its sourced entries".into()),
        }
    }
    if covered
        != [
            2000, 2500, 3000, 3500, 4000, 4500, 5000, 6000, 7000, 8000, 9000, 10000,
        ]
        .into()
    {
        return Err("Washer coverage must retain every requested nominal".into());
    }
    Ok(value)
}
fn washer_identity(entry: &WasherEntry, role: u64) -> Result<EntityId, Error> {
    Ok(EntityId::parse(&format!(
        "cad23300-0000-4000-8000-{:012x}",
        nominal_key(entry.nominal_mm)? * 100 + role
    ))?)
}
fn washer_literal(entry: &WasherEntry) -> MechanicalPartParameters {
    MechanicalPartParameters::Washer {
        bore_diameter: length(entry.bore_mm),
        outside_diameter: length(entry.outside_mm),
        thickness: length(entry.thickness_mm),
        frame: Default::default(),
    }
}
pub fn build_washer(curation: &WasherCuration, index: usize) -> Result<ProjectFile, Error> {
    let entry = curation
        .entries
        .get(index)
        .ok_or("Missing curated washer")?;
    let name = format!("{} normal plain washer", entry.designation);
    let document: Document = serde_json::from_value(serde_json::json!({
        "schema_version":multicad_core::document::DOCUMENT_SCHEMA,
        "id":washer_identity(entry,0)?,"name":name,"entities":{},
    }))?;
    let mut session = Session::new(document)?;
    let mut exposed = Vec::new();
    let mut edits = Vec::new();
    for (role, name, value) in [
        (1, "bore", entry.bore_mm),
        (2, "outside", entry.outside_mm),
        (3, "thickness", entry.thickness_mm),
    ] {
        let mut parameter = Entity::new(
            name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        parameter.id = washer_identity(entry, role)?;
        exposed.push(parameter.id);
        edits.push(Mutation::Insert(parameter));
    }
    session.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Washer physical dimensions".into(),
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
        MechanicalPartParameters::Washer {
            bore_diameter: dimension("bore")?,
            outside_diameter: dimension("outside")?,
            thickness: dimension("thickness")?,
            frame: Default::default(),
        },
    );
    request.feature = washer_identity(entry, 4)?;
    request.body = washer_identity(entry, 5)?;
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: format!("toskcad.bolts.{}", entry.id),
        revision: format!(
            "{} / washer curation {}",
            curation.data_revision, curation.revision
        ),
        designation: format!("{} normal-series dimensional reference", entry.designation),
        source_url: curation.data_source.clone(),
        reference_geometry: washer_literal(entry),
        catalog_facts: Some(MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(entry.nominal_mm),
            series: Some(format!(
                "{} (BOLTS plainwasher1, curated revision {})",
                entry.series, curation.revision
            )),
            material: entry.material.clone(),
            grade: entry.grade.clone(),
        }),
    });
    let prepared = model::prepare_mechanical_part(&session, session.revision(), actor(), &request)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.prepared)?;
    let mut author =
        SymbolAuthorRequest::new(&name, request.body, SymbolKind::Dynamic, exposed, None);
    author.definition = washer_identity(entry, 6)?;
    author.symbol = washer_identity(entry, 7)?;
    author.revision = curation.revision.clone();
    let prepared = symbols::prepare_author(&session, session.revision(), actor(), &author)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.prepared)?;
    let preview =
        plant_library::render_symbol_preview(session.document(), author.definition, |_| {
            [0.62, 0.64, 0.68]
        })?;
    let info = FileInfo {
        name,
        description: format!(
            "Nominal screw size {} mm; physical bore {} mm, outside diameter {} mm, thickness {} mm. Editable annular solid with retained normal-series source dimensions. Tolerances and edge breaks are omitted; material and grade are unspecified.",
            entry.nominal_mm, entry.bore_mm, entry.outside_mm, entry.thickness_mm
        ),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = washer_identity(entry, 8)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Washer library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}
fn checked_relative(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 200
        || value.contains(['\\', ':'])
        || value
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err("Catalog source path must stay inside its folder".into());
    }
    Ok(())
}
fn main() -> Result<(), Error> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("Usage: mechanical_library OUTPUT_FOLDER")?;
    let output = Path::new(&output);
    if output.exists()
        && (std::fs::symlink_metadata(output)?.file_type().is_symlink()
            || std::fs::read_dir(output)?.next().is_some())
    {
        return Err(
            "Catalog generation needs an empty output folder; existing files are preserved".into(),
        );
    }
    std::fs::create_dir_all(output)?;
    match std::env::args().nth(2).as_deref() {
        Some("--machine-screws") => return machine_screw_catalog::generate(output),
        Some("--hex-nuts") => return nut_catalog::generate(output),
        None => {}
        _ => return Err("Optional catalog mode must be --machine-screws or --hex-nuts".into()),
    }
    let curation = curated_washers()?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("library.json"))?)?;
    if manifest["format"] != "toskcad.bundled-library" || manifest["version"] != 1 {
        return Err("Unsupported source Library manifest".into());
    }
    let mut entries = Vec::new();
    for entry in manifest["entries"]
        .as_array()
        .ok_or("Missing Library entries")?
    {
        let file = entry["file"]
            .as_str()
            .ok_or("Missing source Library filename")?;
        checked_relative(file)?;
        if file.contains('/') || !file.ends_with(".tcd") {
            return Err("Invalid project filename".into());
        }
        if curation.entries.iter().any(|e| e.file == file) {
            continue;
        }
        let bytes = std::fs::read(source.join(file))?;
        if project_file::ContentHash::of(&bytes).to_string()
            != entry["sha256"].as_str().ok_or("Missing project checksum")?
        {
            return Err(format!("Existing Library project checksum differs: {file}").into());
        }
        write_new(&output.join(file), &bytes)?;
        entries.push(entry.clone());
    }
    for (index, entry) in curation.entries.iter().enumerate() {
        let project = build_washer(&curation, index)?;
        let mut bytes = Vec::new();
        project_file::write(&project, &mut bytes, Default::default())?;
        write_new(&output.join(&entry.file), &bytes)?;
        entries.push(serde_json::json!({"file":entry.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()}));
        println!("{} · {} bytes", entry.file, bytes.len());
    }
    let mut requires: Vec<String> = serde_json::from_value(manifest["requires"].clone())?;
    for file in [
        "washers-curation.json",
        "sources/bolts/washer.blt",
        "sources/bolts/WASHER-NOTICES.md",
    ] {
        if !requires.iter().any(|r| r == file) {
            requires.push(file.into());
        }
    }
    for file in &requires {
        checked_relative(file)?;
        let path = source.join(file);
        if !std::fs::symlink_metadata(&path)?.is_file() {
            return Err("Catalog source notice must be a regular file".into());
        }
        let destination = output.join(file);
        std::fs::create_dir_all(destination.parent().ok_or("Missing source directory")?)?;
        write_new(&destination, &std::fs::read(path)?)?;
    }
    let manifest = serde_json::json!({"format":"toskcad.bundled-library","version":1,"revision":"5","entries":entries,"requires":requires});
    let mut bytes = serde_json::to_vec_pretty(&manifest)?;
    bytes.push(b'\n');
    write_new(&output.join("library.json"), &bytes)?;
    Ok(())
}
