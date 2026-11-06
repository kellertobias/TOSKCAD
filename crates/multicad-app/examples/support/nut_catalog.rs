//! Developer-only adapter for edition-specific licensed nut dimensional data.
use super::{Error, actor, checked_relative, plant_library, write_new};
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, MechanicalPartCatalogFacts, MechanicalPartParameters, MechanicalPartProvenance,
        MechanicalPartRequest, ThreadParameters, ThreadRepresentation, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use multicad_model::Compiler;
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path};

const SOURCE_REVISION: &str = "5c83300a57761f70db87f50c9a1f647c7278a4c2";
const SOURCE_HASH: &str = "55932f107596d255c008eac910e9c7a4dad3482160d2a1b4f4b56df11a59142b";
const NOMINALS: [u64; 12] = [
    2000, 2500, 3000, 3500, 4000, 4500, 5000, 6000, 7000, 8000, 9000, 10000,
];

#[derive(Clone, Debug, Deserialize)]
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
    pub coverage: Vec<Coverage>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub file: String,
    pub designation: String,
    pub family: String,
    pub nominal_mm: f64,
    pub pitch_mm: f64,
    pub across_flats_mm: f64,
    pub height_mm: f64,
    pub source_class: String,
    pub source_row: [f64; 4],
    pub verification_url: String,
    pub verification_reference: String,
    pub correction: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub family: String,
    pub nominal_mm: f64,
    pub status: String,
    pub entry_id: Option<String>,
    pub reason: String,
}
fn nominal_key(value: f64) -> Result<u64, Error> {
    let scaled = value * 1000.;
    if !scaled.is_finite()
        || !(1. ..=100_000.).contains(&scaled)
        || (scaled - scaled.round()).abs() > 1e-8
    {
        return Err("Nut nominal needs a positive exact micrometer identity".into());
    }
    Ok(scaled.round() as u64)
}
fn family(entry: &Entry) -> Result<(u64, &'static str, &'static str), Error> {
    match entry.family.as_str() {
        "din934-1987" => Ok((1, "hexagonnut2", "DIN 934:1987")),
        "iso4032-2012" => Ok((2, "hexagonnut1", "ISO 4032:2012")),
        _ => Err("Unsupported nut family or edition".into()),
    }
}
fn identity(entry: &Entry, role: u64) -> Result<EntityId, Error> {
    let key = (family(entry)?.0 * 1_000_000 + nominal_key(entry.nominal_mm)?) * 100 + role;
    Ok(EntityId::parse(&format!(
        "cad23400-0000-4000-8000-{key:012x}"
    ))?)
}
fn literal(entry: &Entry) -> MechanicalPartParameters {
    MechanicalPartParameters::HexNut {
        across_flats: length(entry.across_flats_mm),
        thread: Box::new(ThreadParameters {
            major_diameter: length(entry.nominal_mm),
            pitch: length(entry.pitch_mm),
            length: length(entry.height_mm),
            minimum_wall: length(0.5),
            representation: ThreadRepresentation::Modeled,
            representation_parameter: None,
            ..Default::default()
        }),
    }
}
fn validate_entry(entry: &Entry) -> Result<(), Error> {
    let (_, class, _) = family(entry)?;
    let key = nominal_key(entry.nominal_mm)?;
    let slug = entry.nominal_mm.to_string().replace('.', "-");
    let expected_pitch = match key {
        2000 => 0.4,
        2500 => 0.45,
        3000 => 0.5,
        3500 => 0.6,
        4000 => 0.7,
        5000 => 0.8,
        6000 | 7000 => 1.,
        8000 => 1.25,
        10000 => 1.5,
        _ => return Err("No selected source row for this nut nominal".into()),
    };
    if entry.family == "iso4032-2012" && key == 7000 {
        return Err("ISO 4032:2012 does not supply an M7 row".into());
    }
    if entry.id != format!("nut.{}.m{slug}", entry.family)
        || entry.file != format!("nut-{}-m{slug}.tcd", entry.family)
        || entry.designation != format!("M{}", entry.nominal_mm)
        || entry.source_class != class
        || entry.pitch_mm != expected_pitch
        || entry.source_row
            != [
                entry.nominal_mm,
                entry.across_flats_mm,
                entry.height_mm,
                entry.source_row[3],
            ]
        || entry.source_row.iter().any(|v| !v.is_finite() || *v <= 0.)
        || 2. * entry.across_flats_mm / 3_f64.sqrt() < entry.source_row[3]
        || !entry.verification_url.starts_with("https://")
        || entry.verification_reference.is_empty()
        || entry.correction.is_some()
    {
        return Err("Nut identity or retained source dimensions disagree".into());
    }
    let original = include_str!("../../../../library/sources/bolts/nut.blt");
    let block = original
        .split(&format!("  - id: {class}\n"))
        .nth(1)
        .ok_or("Missing original nut class")?
        .split("\n  - id:")
        .next()
        .ok_or("Missing nut table")?;
    let line = block
        .lines()
        .find(|line| {
            line.trim_start()
                .starts_with(&format!("{}:", entry.designation))
        })
        .ok_or("Missing original nut row")?;
    let values = line
        .split('[')
        .nth(1)
        .and_then(|v| v.split(']').next())
        .ok_or("Malformed original nut row")?;
    let row: Vec<f64> = values
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse())
        .collect::<Result<_, _>>()?;
    if row.as_slice() != entry.source_row {
        return Err("Selected nut row differs from the pinned licensed original".into());
    }
    let session = Session::new(Document::new("Nut reference validation")?)?;
    literal(entry).resolve(session.parameters())?;
    Ok(())
}
pub fn curated() -> Result<Curation, Error> {
    let c: Curation = serde_json::from_str(include_str!("../../../../library/nuts-curation.json"))?;
    if c.format != "toskcad.nut-curation"
        || c.version != 1
        || c.revision != "1"
        || c.data_revision != SOURCE_REVISION
        || c.data_license != "LGPL-2.1-or-later"
        || c.data_sha256 != SOURCE_HASH
        || project_file::ContentHash::of(include_bytes!(
            "../../../../library/sources/bolts/nut.blt"
        ))
        .to_string()
            != SOURCE_HASH
        || c.data_source
            != format!(
                "https://github.com/boltsparts/boltsparts/blob/{SOURCE_REVISION}/data/nut.blt"
            )
        || c.verified_on.is_empty()
        || c.scope.is_empty()
        || c.entries.len() != 19
        || c.coverage.len() != 24
    {
        return Err("Unsupported nut curation, source pin or coverage".into());
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    if project_file::ContentHash::of(&std::fs::read(source.join("sources/bolts/nut.blt"))?)
        .to_string()
        != SOURCE_HASH
        || std::fs::read(source.join("nuts-curation.json"))?.as_slice()
            != include_bytes!("../../../../library/nuts-curation.json").as_slice()
    {
        return Err("Runtime nut data differs from the compiled curation/source pin".into());
    }
    let mut ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for entry in &c.entries {
        validate_entry(entry)?;
        if !ids.insert(entry.id.as_str()) || !files.insert(entry.file.as_str()) {
            return Err("Duplicate nut preset identity".into());
        }
        for role in 0..=10 {
            if !identities.insert(identity(entry, role)?) {
                return Err("Nut source identities overlap".into());
            }
        }
    }
    let mut covered = BTreeSet::new();
    for row in &c.coverage {
        let key = nominal_key(row.nominal_mm)?;
        if !matches!(row.family.as_str(), "din934-1987" | "iso4032-2012")
            || !covered.insert((row.family.as_str(), key))
            || row.reason.is_empty()
        {
            return Err("Nut coverage must be unique and edition-specific".into());
        }
        let entry = c
            .entries
            .iter()
            .find(|e| e.family == row.family && e.nominal_mm == row.nominal_mm);
        match (row.status.as_str(), entry, row.entry_id.as_deref()) {
            ("supported", Some(entry), Some(id)) if entry.id == id => {}
            ("unavailable", None, None)
                if matches!(key, 4500 | 9000) || (row.family == "iso4032-2012" && key == 7000) => {}
            _ => return Err("Nut coverage disagrees with its selected rows".into()),
        }
    }
    let expected: BTreeSet<_> = ["din934-1987", "iso4032-2012"]
        .into_iter()
        .flat_map(|family| NOMINALS.map(|nominal| (family, nominal)))
        .collect();
    if covered != expected {
        return Err("Each requested nominal needs both family coverage statuses".into());
    }
    Ok(c)
}
pub fn build(c: &Curation, index: usize) -> Result<ProjectFile, Error> {
    let entry = c.entries.get(index).ok_or("Missing curated nut")?;
    validate_entry(entry)?;
    let name = format!("{} {} hex nut", entry.designation, family(entry)?.2);
    let document: Document = serde_json::from_value(serde_json::json!({
        "schema_version":multicad_core::document::DOCUMENT_SCHEMA,
        "id":identity(entry,0)?,"name":name,"entities":{},
    }))?;
    let mut session = Session::new(document)?;
    let mut exposed = Vec::new();
    let mut edits = Vec::new();
    for (role, parameter_name, value, unit) in [
        (1, "nominal_thread_diameter", entry.nominal_mm, "mm"),
        (2, "pitch", entry.pitch_mm, "mm"),
        (3, "across_flats", entry.across_flats_mm, "mm"),
        (4, "height", entry.height_mm, "mm"),
        (5, "thread_representation", 1., "count"),
    ] {
        let mut parameter = Entity::new(
            parameter_name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        );
        parameter.id = identity(entry, role)?;
        exposed.push(parameter.id);
        edits.push(Mutation::Insert(parameter));
    }
    session.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Nut source dimensions".into(),
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
        MechanicalPartParameters::HexNut {
            across_flats: dimension("across_flats")?,
            thread: Box::new(ThreadParameters {
                major_diameter: dimension("nominal_thread_diameter")?,
                pitch: dimension("pitch")?,
                length: dimension("height")?,
                minimum_wall: length(0.5),
                representation: ThreadRepresentation::Modeled,
                representation_parameter: Some(identity(entry, 5)?),
                ..Default::default()
            }),
        },
    );
    request.feature = identity(entry, 6)?;
    request.body = identity(entry, 7)?;
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: format!("toskcad.bolts.{}", entry.id),
        revision: format!("{} / nut curation {}", c.data_revision, c.revision),
        designation: format!(
            "{} {} dimensional reference",
            family(entry)?.2,
            entry.designation
        ),
        source_url: c.data_source.clone(),
        reference_geometry: literal(entry),
        catalog_facts: Some(MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(entry.nominal_mm),
            series: Some(family(entry)?.2.into()),
            material: None,
            grade: None,
        }),
    });
    let prepared = model::prepare_mechanical_part(&session, session.revision(), actor(), &request)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.prepared)?;
    let mut author =
        SymbolAuthorRequest::new(&name, request.body, SymbolKind::Dynamic, exposed, None);
    author.definition = identity(entry, 8)?;
    author.symbol = identity(entry, 9)?;
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
            "{} reference: nominal {} mm, pitch {} mm, across flats {} mm and height {} mm (source maxima). Editable dimensions and explicit modeled/cosmetic thread choice. Nominal-basic internal thread without fit-class certification. Sharp hexagon; tolerances, chamfers, washer faces and thread-entry details omitted. Material, grade, coating and stock unspecified.",
            family(entry)?.2,
            entry.nominal_mm,
            entry.pitch_mm,
            entry.across_flats_mm,
            entry.height_mm
        ),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = identity(entry, 10)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Nut Library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}
pub fn generate(output: &Path) -> Result<(), Error> {
    if output.exists()
        && (std::fs::symlink_metadata(output)?.file_type().is_symlink()
            || !output.is_dir()
            || std::fs::read_dir(output)?.next().is_some())
    {
        return Err("Nut catalog generation needs an empty output folder".into());
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
    for entry in manifest["entries"]
        .as_array()
        .ok_or("Missing Library entries")?
    {
        let file = entry["file"].as_str().ok_or("Missing Library filename")?;
        checked_relative(file)?;
        if file.contains('/') || !file.ends_with(".tcd") || !seen.insert(file) {
            return Err("Invalid or duplicate Library project filename".into());
        }
        if c.entries.iter().any(|e| e.file == file) {
            continue;
        }
        let path = source.join(file);
        if !std::fs::symlink_metadata(&path)?.is_file() {
            return Err("Existing Library project must be a regular file".into());
        }
        let bytes = std::fs::read(path)?;
        if project_file::ContentHash::of(&bytes).to_string()
            != entry["sha256"].as_str().ok_or("Missing checksum")?
        {
            return Err("Existing Library project checksum differs".into());
        }
        write_new(&output.join(file), &bytes)?;
        entries.push(entry.clone());
    }
    for (index, entry) in c.entries.iter().enumerate() {
        let project = build(&c, index)?;
        let mut bytes = Vec::new();
        project_file::write(&project, &mut bytes, Default::default())?;
        write_new(&output.join(&entry.file), &bytes)?;
        entries.push(serde_json::json!({"file":entry.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()}));
        println!("{} · {} bytes", entry.file, bytes.len());
    }
    let mut requires: Vec<String> = serde_json::from_value(manifest["requires"].clone())?;
    for file in [
        "nuts-curation.json",
        "sources/bolts/nut.blt",
        "sources/bolts/NUT-NOTICES.md",
        "sources/bolts/COPYING.LESSER",
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
        let destination = output.join(file);
        std::fs::create_dir_all(
            destination
                .parent()
                .ok_or("Missing required source parent")?,
        )?;
        write_new(&destination, &std::fs::read(path)?)?;
    }
    let value = serde_json::json!({"format":"toskcad.bundled-library","version":1,"revision":revision,"entries":entries,"requires":requires});
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    write_new(&output.join("library.json"), &bytes)?;
    Ok(())
}
