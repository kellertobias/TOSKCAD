//! Developer-only curation of source-dimensioned, nonmanufacturing wood envelopes.
use super::{Error, actor, checked_relative, plant_library, write_new};
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, MechanicalPartCatalogFacts, MechanicalPartParameters, MechanicalPartProvenance,
        MechanicalPartRequest, ScrewDrive, ScrewHead, ScrewLengthInput, WoodPointIntent,
        WoodScrewParameters, WoodScrewRepresentation, WoodVReference, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use multicad_model::Compiler;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Curation {
    pub format: String,
    pub version: u32,
    pub revision: String,
    pub verified_on: String,
    pub scope: String,
    pub representation: String,
    pub modeling_targets: ModelingTargets,
    pub sources: Vec<Source>,
    pub entries: Vec<Entry>,
    pub coverage: Vec<Coverage>,
    pub excluded_families: Vec<ExcludedFamily>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelingTargets {
    pub head_height: String,
    pub across_flats: String,
    pub underhead_length: String,
    pub stock_diameter: String,
    pub distal_end: String,
    pub point: String,
    pub manufacturing: String,
    pub thread_extent: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub url: String,
    pub sha256: String,
    pub reference: String,
    pub role: String,
    pub rights: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub file: String,
    pub designation: String,
    pub family: String,
    pub nominal_mm: f64,
    pub underhead_length_mm: f64,
    pub length_min_mm: f64,
    pub length_max_mm: f64,
    pub head: Head,
    pub v_reference: VReference,
    pub minimum_thread_extent_ratio: f64,
    pub source_weight_g: f64,
    pub dimensional_source: String,
    pub profile_reference_source: String,
    pub minimum_extent_source: String,
    pub correction: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Head {
    pub height_nominal_mm: f64,
    pub height_min_mm: f64,
    pub height_max_mm: f64,
    pub across_flats_min_mm: f64,
    pub across_flats_max_mm: f64,
    pub diagonal_reference_mm: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VReference {
    pub pitch_mm: f64,
    pub core_diameter_mm: f64,
    pub included_angle_deg: f64,
    pub included_angle_tolerance_deg: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub family: String,
    pub nominal_mm: f64,
    pub status: String,
    pub entry_ids: Vec<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludedFamily {
    pub family: String,
    pub status: String,
    pub reason: String,
}

fn validate(c: &Curation) -> Result<(), Error> {
    let pinned: Curation = serde_json::from_str(include_str!(
        "../../../../library/wood-screws-curation.json"
    ))?;
    if serde_json::to_value(c)? != serde_json::to_value(pinned)?
        || c.format != "toskcad.wood-screw-curation"
        || c.version != 1
        || c.revision != "1"
        || c.entries.len() != 44
        || c.coverage.len() != 12
        || c.representation != "simplified_envelope"
    {
        return Err(
            "Wood curation differs from the pinned dimensional source; explicit migration required"
                .into(),
        );
    }
    let mut ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for e in &c.entries {
        if !ids.insert(&e.id)
            || !files.insert(&e.file)
            || e.family != "bitork-din571"
            || e.designation.starts_with('M')
            || e.minimum_thread_extent_ratio != 0.6
            || e.correction.is_some()
        {
            return Err("Invalid or duplicate wood preset".into());
        }
        for role in 0..=84 {
            if !identities.insert(identity(e, role)?) {
                return Err("Overlapping wood identities".into());
            }
        }
    }
    Ok(())
}
pub fn curated() -> Result<Curation, Error> {
    let c: Curation = serde_json::from_str(include_str!(
        "../../../../library/wood-screws-curation.json"
    ))?;
    validate(&c)?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    if std::fs::read(source.join("wood-screws-curation.json"))?.as_slice()
        != include_bytes!("../../../../library/wood-screws-curation.json").as_slice()
    {
        return Err("Runtime wood curation differs from compiled source".into());
    }
    Ok(c)
}
fn exact_key(value: f64) -> Result<u64, Error> {
    let scaled = value * 1000.;
    if !scaled.is_finite() || !(1. ..=1_000_000.).contains(&scaled) || scaled != scaled.round() {
        return Err("Wood identities require exact positive micrometer dimensions".into());
    }
    Ok(scaled as u64)
}
fn identity(e: &Entry, role: u64) -> Result<EntityId, Error> {
    let key =
        (exact_key(e.nominal_mm)? * 1_000_000 + exact_key(e.underhead_length_mm)?) * 100 + role;
    Ok(EntityId::parse(&format!(
        "cad23700-0000-4000-8000-{key:012x}"
    ))?)
}
fn angle(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "deg".into(),
    }
}
fn literal(e: &Entry) -> MechanicalPartParameters {
    MechanicalPartParameters::WoodScrew {
        parameters: Box::new(WoodScrewParameters {
            length_input: ScrewLengthInput::Underhead {
                value: length(e.underhead_length_mm),
            },
            nominal_diameter: length(e.nominal_mm),
            head: ScrewHead::ExternalHex {
                across_flats: length(e.head.across_flats_max_mm),
                height: length(e.head.height_nominal_mm),
            },
            drive: ScrewDrive::ExternalHex,
            reference_profile: WoodVReference {
                pitch: length(e.v_reference.pitch_mm),
                core_diameter: length(e.v_reference.core_diameter_mm),
                included_angle: angle(e.v_reference.included_angle_deg),
            },
            minimum_tip_inclusive_thread_extent: length(
                e.underhead_length_mm * e.minimum_thread_extent_ratio,
            ),
            point: WoodPointIntent::ManufacturerUnspecified,
            representation: WoodScrewRepresentation::SimplifiedEnvelope,
            frame: Default::default(),
        }),
    }
}
pub fn build(c: &Curation, index: usize) -> Result<ProjectFile, Error> {
    validate(c)?;
    let e = c.entries.get(index).ok_or("Missing wood preset")?;
    let name = format!(
        "{} wood screw · DIN 571 reference · simplified envelope",
        e.designation
    );
    let document: Document = serde_json::from_value(
        serde_json::json!({"schema_version":multicad_core::document::DOCUMENT_SCHEMA,"id":identity(e,0)?,"name":name,"entities":{}}),
    )?;
    let mut session = Session::new(document)?;
    let mut edits = Vec::new();
    let mut exposed = Vec::new();
    for (role, name, value, unit) in [
        (1, "nominal_wood_diameter", e.nominal_mm, "mm"),
        (2, "underhead_length", e.underhead_length_mm, "mm"),
        (3, "across_flats", e.head.across_flats_max_mm, "mm"),
        (4, "head_height", e.head.height_nominal_mm, "mm"),
        (5, "reference_wood_pitch", e.v_reference.pitch_mm, "mm"),
        (
            6,
            "reference_wood_core_diameter",
            e.v_reference.core_diameter_mm,
            "mm",
        ),
        (
            7,
            "reference_wood_included_angle",
            e.v_reference.included_angle_deg,
            "deg",
        ),
    ] {
        let mut p = Entity::new(
            name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        );
        p.id = identity(e, role)?;
        exposed.push(p.id);
        edits.push(Mutation::Insert(p));
    }
    session.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Wood envelope source dimensions".into(),
        edits,
    })?;
    let bound = |name: &str, unit: &str| -> Result<Dimension, Error> {
        Ok(Dimension::Expression {
            expression: BoundExpression::bind(
                session.document(),
                &format!("=${name}"),
                unit,
                None,
            )?,
        })
    };
    let mut parameters = literal(e);
    let MechanicalPartParameters::WoodScrew { parameters: wood } = &mut parameters else {
        unreachable!()
    };
    wood.length_input = ScrewLengthInput::Underhead {
        value: bound("underhead_length", "mm")?,
    };
    wood.nominal_diameter = bound("nominal_wood_diameter", "mm")?;
    wood.head = ScrewHead::ExternalHex {
        across_flats: bound("across_flats", "mm")?,
        height: bound("head_height", "mm")?,
    };
    wood.reference_profile = WoodVReference {
        pitch: bound("reference_wood_pitch", "mm")?,
        core_diameter: bound("reference_wood_core_diameter", "mm")?,
        included_angle: bound("reference_wood_included_angle", "deg")?,
    };
    wood.minimum_tip_inclusive_thread_extent = Dimension::Expression {
        expression: BoundExpression::bind(
            session.document(),
            "=$underhead_length * 0.6",
            "mm",
            None,
        )?,
    };
    let mut request = MechanicalPartRequest::new(&name, parameters);
    request.feature = identity(e, 80)?;
    request.body = identity(e, 81)?;
    request.provenance=Some(MechanicalPartProvenance {
        catalog_id:format!("toskcad.sourced.{}",e.id),revision:format!("wood screw curation {}",c.revision),
        designation:format!("Bitork DIN 571 {} dimensional reference; simplified envelope",e.designation),
        source_url:c.sources.iter().find(|s|s.id==e.dimensional_source).ok_or("Missing wood dimensional source")?.url.clone(),
        reference_geometry:literal(e),catalog_facts:Some(MechanicalPartCatalogFacts { nominal_fastener_diameter_mm:Some(e.nominal_mm),series:Some("DIN 571-related wood screw; simplified envelope, unspecified point and thread detail".into()),material:None,grade:None }),
    });
    let prepared = model::prepare_mechanical_part(&session, session.revision(), actor(), &request)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.prepared)?;
    let mut author =
        SymbolAuthorRequest::new(&name, request.body, SymbolKind::Dynamic, exposed, None);
    author.definition = identity(e, 82)?;
    author.symbol = identity(e, 83)?;
    author.revision = c.revision.clone();
    let prepared = symbols::prepare_author(&session, session.revision(), actor(), &author)?;
    session.commit(prepared.prepared)?;
    let preview =
        plant_library::render_symbol_preview(session.document(), author.definition, |_| {
            [0.62, 0.64, 0.68]
        })?;
    let info = FileInfo {
        name,
        description: format!(
            "Bitork DIN 571-related {} wood screw dimensional reference. Underhead length; nominal head height and maximum across flats. Simplified stock envelope with flat distal end enclosing an unspecified manufacturer point; no recovered wood helix, point or run-in. Separate wood V-reference and minimum tip-inclusive extent are intent only. Manufacturing unavailable. Material, grade, coating and stock unspecified.",
            e.designation
        ),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = identity(e, 84)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Wood envelope Library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}
#[derive(Clone, Debug)]
pub struct RegenerationReceipt {
    bytes: Vec<u8>,
    pin: project_file::ContentHash,
}
fn wood_pin(entry: &Entry, project: &ProjectFile) -> Result<project_file::ContentHash, Error> {
    let definitions = symbols::definitions(&project.document)?;
    if project.document.id() != identity(entry, 0)?
        || definitions.len() != 1
        || definitions[0].0 != identity(entry, 82)?
        || definitions[0].1.symbol != identity(entry, 83)?
        || definitions[0].1.kind != SymbolKind::Dynamic
    {
        return Err("Existing wood envelope identity differs; explicit migration required".into());
    }
    Ok(definitions[0].1.pin()?)
}
pub fn checked_existing_wood(
    entry: &Entry,
    manifest_entry: &serde_json::Value,
    bytes: &[u8],
) -> Result<RegenerationReceipt, Error> {
    if manifest_entry["file"].as_str() != Some(entry.file.as_str())
        || manifest_entry["sha256"].as_str()
            != Some(project_file::ContentHash::of(bytes).to_string().as_str())
    {
        return Err("Existing wood envelope filename/checksum differs".into());
    }
    let project = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    if manifest_entry["document"].as_str() != Some(project.document.id().to_string().as_str()) {
        return Err("Existing wood envelope manifest identity differs".into());
    }
    Ok(RegenerationReceipt {
        bytes: bytes.to_vec(),
        pin: wood_pin(entry, &project)?,
    })
}
pub fn verify_unchanged_wood(
    entry: &Entry,
    existing: &RegenerationReceipt,
    rebuilt: &ProjectFile,
    bytes: &[u8],
) -> Result<(), Error> {
    if wood_pin(entry, rebuilt)? != existing.pin || bytes != existing.bytes {
        return Err(
            "Rebuilt wood envelope pin or bytes differ; explicit migration required".into(),
        );
    }
    Ok(())
}
pub fn generate(output: &Path) -> Result<(), Error> {
    if output.exists()
        && (std::fs::symlink_metadata(output)?.file_type().is_symlink()
            || !output.is_dir()
            || std::fs::read_dir(output)?.next().is_some())
    {
        return Err("Wood catalog generation needs an empty output directory".into());
    }
    std::fs::create_dir_all(output)?;
    let c = curated()?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("library.json"))?)?;
    if manifest["format"] != "toskcad.bundled-library" || manifest["version"] != 1 {
        return Err("Unsupported Library manifest".into());
    }
    let revision = manifest["revision"]
        .as_str()
        .ok_or("Missing Library revision")?
        .parse::<u64>()?
        .checked_add(1)
        .ok_or("Library revision overflow")?
        .to_string();
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    let mut selected = BTreeMap::new();
    for entry in manifest["entries"]
        .as_array()
        .ok_or("Missing Library entries")?
    {
        let file = entry["file"].as_str().ok_or("Missing project filename")?;
        checked_relative(file)?;
        if file.contains('/') || !file.ends_with(".tcd") || !seen.insert(file) {
            return Err("Invalid or duplicate Library filename".into());
        }
        let path = source.join(file);
        if !std::fs::symlink_metadata(&path)?.is_file() {
            return Err("Library project must be a regular file".into());
        }
        let bytes = std::fs::read(path)?;
        if project_file::ContentHash::of(&bytes).to_string()
            != entry["sha256"].as_str().ok_or("Missing Library checksum")?
        {
            return Err("Existing Library project checksum differs".into());
        }
        if let Some(curated) = c.entries.iter().find(|e| e.file == file) {
            selected.insert(
                file.to_owned(),
                checked_existing_wood(curated, entry, &bytes)?,
            );
            continue;
        }
        write_new(&output.join(file), &bytes)?;
        entries.push(entry.clone());
    }
    for (index, entry) in c.entries.iter().enumerate() {
        let project = build(&c, index)?;
        let mut bytes = Vec::new();
        project_file::write(&project, &mut bytes, Default::default())?;
        if let Some(existing) = selected.get(&entry.file) {
            verify_unchanged_wood(entry, existing, &project, &bytes)?;
        }
        write_new(&output.join(&entry.file), &bytes)?;
        entries.push(serde_json::json!({"file":entry.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()}));
        println!("{} · {} bytes", entry.file, bytes.len());
    }
    let mut requires: Vec<String> = serde_json::from_value(manifest["requires"].clone())?;
    for file in [
        "wood-screws-curation.json",
        "sources/wood-screws/source-ledger.json",
        "sources/wood-screws/WOOD-SCREW-NOTICES.md",
    ] {
        if !requires.iter().any(|r| r == file) {
            requires.push(file.into());
        }
    }
    for file in &requires {
        checked_relative(file)?;
        let path = source.join(file);
        if !std::fs::symlink_metadata(&path)?.is_file() {
            return Err("Required source must be a regular file".into());
        }
        let dest = output.join(file);
        std::fs::create_dir_all(dest.parent().ok_or("Missing source parent")?)?;
        write_new(&dest, &std::fs::read(path)?)?;
    }
    let value = serde_json::json!({"format":"toskcad.bundled-library","version":1,"revision":revision,"entries":entries,"requires":requires});
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    write_new(&output.join("library.json"), &bytes)?;
    Ok(())
}
