//! Developer-only curation adapter. Runtime geometry uses retained definitions.
use super::{Error, actor, checked_relative, plant_library, write_new};
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, MechanicalPartCatalogFacts, MechanicalPartParameters, MechanicalPartProvenance,
        MechanicalPartRequest, ScrewDrive, ScrewHead, ThreadParameters, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use multicad_model::Compiler;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    file: String,
    url: String,
    revision: String,
    sha256: String,
    license: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    nominal_mm: f64,
    socket_cap: Availability,
    external_hex: Availability,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Availability {
    status: String,
    entries: Vec<String>,
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Curation {
    format: String,
    version: u32,
    revision: String,
    verified_on: String,
    scope: String,
    data_sources: Vec<Source>,
    entries: Vec<Entry>,
    coverage: Vec<Coverage>,
    profile_revision: String,
    profile_sources: Vec<super::screw_head_catalog::Source>,
    profile_entries: Vec<super::screw_head_catalog::Entry>,
    profile_coverage: Vec<super::screw_head_catalog::Coverage>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    file: String,
    family: String,
    nominal_mm: f64,
    pitch_mm: f64,
    underhead_length_mm: f64,
    thread_length_mm: f64,
    shank_diameter_mm: f64,
    head_height_mm: f64,
    head_diameter_mm: Option<f64>,
    head_across_flats_mm: Option<f64>,
    drive_across_flats_mm: Option<f64>,
    drive_depth_mm: Option<f64>,
    source_product: String,
    verification_url: String,
    source_reference: String,
    head_surface: String,
    limits: BTreeMap<String, f64>,
    modifications: Vec<String>,
}
fn slug(v: f64) -> String {
    v.to_string().replace('.', "-")
}
fn scaled(v: f64) -> Result<u64, Error> {
    let n = v * 1000.;
    if !n.is_finite() || !(1. ..1_000_000.).contains(&n) || (n - n.round()).abs() > 1e-8 {
        return Err("Catalog dimensions need bounded positive micrometer keys".into());
    }
    Ok(n.round() as u64)
}
fn form(entry: &Entry) -> Result<&'static str, Error> {
    match entry.family.as_str() {
        "socket_cap" => Ok("socket"),
        "external_hex" => Ok("hex"),
        _ => Err("Unsupported screw head family".into()),
    }
}
fn identity(entry: &Entry, role: u64) -> Result<EntityId, Error> {
    let family = if form(entry)? == "socket" { 1 } else { 2 };
    let key = (family * 100_000_000_000
        + scaled(entry.nominal_mm)? * 1_000_000
        + scaled(entry.underhead_length_mm)?)
        * 100
        + role;
    Ok(EntityId::parse(&format!(
        "cad23500-0000-4000-8000-{key:012x}"
    ))?)
}
fn literal(entry: &Entry) -> Result<MechanicalPartParameters, Error> {
    let (head, drive) = match form(entry)? {
        "socket" => (
            ScrewHead::SocketCap {
                diameter: length(entry.head_diameter_mm.ok_or("Missing head diameter")?),
                height: length(entry.head_height_mm),
            },
            ScrewDrive::HexSocket {
                across_flats: length(entry.drive_across_flats_mm.ok_or("Missing socket AF")?),
                depth: length(entry.drive_depth_mm.ok_or("Missing socket depth")?),
            },
        ),
        _ => (
            ScrewHead::ExternalHex {
                across_flats: length(entry.head_across_flats_mm.ok_or("Missing head AF")?),
                height: length(entry.head_height_mm),
            },
            ScrewDrive::ExternalHex,
        ),
    };
    Ok(MechanicalPartParameters::MachineScrew {
        underhead_length: Some(length(entry.underhead_length_mm)),
        length_input: None,
        shank_diameter: length(entry.shank_diameter_mm),
        head,
        drive,
        thread: Box::new(ThreadParameters {
            major_diameter: length(entry.nominal_mm),
            pitch: length(entry.pitch_mm),
            length: length(entry.thread_length_mm),
            ..Default::default()
        }),
    })
}
fn curated() -> Result<Curation, Error> {
    let c: Curation = serde_json::from_str(include_str!(
        "../../../../library/machine-screws-curation.json"
    ))?;
    if c.format != "toskcad.machine-screw-curation"
        || c.version != 1
        || c.revision != "1"
        || c.entries.len() != 19
        || c.coverage.len() != 12
        || c.verified_on.is_empty()
        || c.scope.is_empty()
        || c.data_sources.len() != 2
        || c.profile_revision != "1"
        || c.profile_sources.len() != 11
        || c.profile_entries.len() != 52
        || c.profile_coverage.len() != 96
    {
        return Err("Unsupported machine screw curation or incomplete coverage".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let expected: BTreeMap<_, _> = [
        (
            "sources/bolts/hex_socket.blt",
            "7d1f57b64c0d8b574beaabaf4dedc8a11b6980371d9e9508c0bd5399a2cc1f53",
        ),
        (
            "sources/bolts/hex.blt",
            "59cd376c3d0474c953679292f2541248dc9c8d6dce5b491e072d9e5bc5646f10",
        ),
    ]
    .into();
    let mut sources = BTreeSet::new();
    for source in &c.data_sources {
        checked_relative(&source.file)?;
        if !sources.insert(source.file.as_str())
            || expected.get(source.file.as_str()) != Some(&source.sha256.as_str())
            || source.revision != "5c83300a57761f70db87f50c9a1f647c7278a4c2"
            || source.license != "LGPL-2.1-or-later"
            || !source.url.starts_with("https://")
            || project_file::ContentHash::of(&std::fs::read(root.join(&source.file))?).to_string()
                != source.sha256
        {
            return Err("Machine screw source hash/license/revision mismatch".into());
        }
    }
    let validation = Session::new(Document::new("Machine screw catalog validation")?)?;
    let mut ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    for entry in &c.entries {
        let kind = form(entry)?;
        if !ids.insert(entry.id.as_str())
            || !files.insert(entry.file.as_str())
            || entry.id
                != format!(
                    "screw.{kind}.m{}.l{}",
                    slug(entry.nominal_mm),
                    slug(entry.underhead_length_mm)
                )
            || entry.file
                != format!(
                    "screw-{kind}-m{}-l{}.tcd",
                    slug(entry.nominal_mm),
                    slug(entry.underhead_length_mm)
                )
            || entry.source_product.is_empty()
            || entry.source_reference.is_empty()
            || entry.head_surface.is_empty()
            || !entry.verification_url.starts_with("https://")
            || entry.modifications.is_empty()
            || entry.limits.values().any(|v| !v.is_finite() || *v <= 0.)
        {
            return Err("Invalid retained screw identity or dimensional source".into());
        }
        identity(entry, 0)?;
        literal(entry)?.resolve(validation.parameters())?;
        let in_bounds = |minimum: &str, maximum: &str, value: f64| {
            entry.limits.get(minimum).is_none_or(|min| value >= *min)
                && entry.limits.get(maximum).is_none_or(|max| value <= *max)
        };
        if !in_bounds(
            "head_height_min_mm",
            "head_height_max_mm",
            entry.head_height_mm,
        ) {
            return Err("Head height falls outside retained source bounds".into());
        }
        if kind == "socket" {
            if entry.head_across_flats_mm.is_some() {
                return Err("Socket head cannot retain external hex dimensions".into());
            }
            let af = entry.drive_across_flats_mm.ok_or("Missing socket AF")?;
            let depth = entry.drive_depth_mm.ok_or("Missing socket depth")?;
            if !in_bounds(
                "head_diameter_min_mm",
                "head_diameter_knurled_max_mm",
                entry.head_diameter_mm.ok_or("Missing head diameter")?,
            ) {
                return Err("Head outer-diameter envelope exceeds the retained limits".into());
            }
            let bound = |key| {
                entry
                    .limits
                    .get(key)
                    .copied()
                    .ok_or("Missing socket source limit")
            };
            if af < bound("socket_af_min_mm")?
                || af > bound("socket_af_max_mm")?
                || depth < bound("socket_depth_min_mm")?
                || entry.head_height_mm - depth < bound("floor_min_mm")?
                || entry.head_height_mm < bound("head_height_min_mm")?
                || entry.head_height_mm > bound("head_height_max_mm")?
            {
                return Err(
                    "Socket engineering target falls outside retained source bounds".into(),
                );
            }
        } else if entry.head_diameter_mm.is_some()
            || entry.drive_across_flats_mm.is_some()
            || entry.drive_depth_mm.is_some()
            || entry.thread_length_mm != entry.underhead_length_mm
        {
            return Err("Selected DIN933 entries are external hex and fully threaded".into());
        }
        if kind == "hex" {
            let af = entry.head_across_flats_mm.ok_or("Missing head AF")?;
            if !in_bounds("head_across_flats_min_mm", "head_across_flats_max_mm", af)
                || entry
                    .limits
                    .get("head_across_flats_reference_mm")
                    .is_some_and(|expected| (af - expected).abs() > 1e-9)
                || entry
                    .limits
                    .get("head_height_reference_mm")
                    .is_some_and(|expected| (entry.head_height_mm - expected).abs() > 1e-9)
                || entry
                    .limits
                    .get("head_corner_min_mm")
                    .or_else(|| entry.limits.get("head_corner_reference_mm"))
                    .is_some_and(|minimum| 2. * af / 3_f64.sqrt() < *minimum)
            {
                return Err(
                    "External hex target disagrees with its retained source dimensions".into(),
                );
            }
        }
    }
    let mut nominals = BTreeSet::new();
    for row in &c.coverage {
        if !nominals.insert(scaled(row.nominal_mm)?) {
            return Err("Duplicate screw coverage nominal".into());
        }
        for (family, status) in [
            ("socket_cap", &row.socket_cap),
            ("external_hex", &row.external_hex),
        ] {
            let actual: BTreeSet<_> = c
                .entries
                .iter()
                .filter(|e| e.nominal_mm == row.nominal_mm && e.family == family)
                .map(|e| e.id.as_str())
                .collect();
            let declared: BTreeSet<_> = status.entries.iter().map(String::as_str).collect();
            if status.reason.is_empty()
                || actual != declared
                || status.entries.len() != declared.len()
                || !matches!(
                    (status.status.as_str(), actual.is_empty()),
                    ("supported", false) | ("unavailable", true)
                )
            {
                return Err("Screw coverage disagrees with the explicit sourced rows".into());
            }
        }
    }
    if nominals
        != [
            2000, 2500, 3000, 3500, 4000, 4500, 5000, 6000, 7000, 8000, 9000, 10000,
        ]
        .into()
    {
        return Err("Every requested nominal needs coverage status".into());
    }
    Ok(c)
}
pub(super) fn validate_legacy_curation() -> Result<(), Error> {
    curated().map(|_| ())
}
fn build(c: &Curation, entry: &Entry) -> Result<ProjectFile, Error> {
    let kind = form(entry)?;
    let name = format!(
        "M{} × {} · {} screw reference",
        entry.nominal_mm,
        entry.underhead_length_mm,
        if kind == "socket" {
            "socket-cap"
        } else {
            "external-hex"
        }
    );
    let document: Document = serde_json::from_value(
        serde_json::json!({"schema_version":multicad_core::document::DOCUMENT_SCHEMA,"id":identity(entry,0)?,"name":name,"entities":{}}),
    )?;
    let mut s = Session::new(document)?;
    let mut parameters = vec![
        (1, "underhead_length", entry.underhead_length_mm),
        (2, "thread_length", entry.thread_length_mm),
        (3, "shank_diameter", entry.shank_diameter_mm),
        (4, "nominal_thread_diameter", entry.nominal_mm),
        (5, "pitch", entry.pitch_mm),
        (6, "head_height", entry.head_height_mm),
    ];
    if kind == "socket" {
        parameters.extend([
            (
                7,
                "head_diameter",
                entry.head_diameter_mm.ok_or("Missing head")?,
            ),
            (
                8,
                "socket_across_flats",
                entry.drive_across_flats_mm.ok_or("Missing drive")?,
            ),
            (
                9,
                "socket_depth",
                entry.drive_depth_mm.ok_or("Missing depth")?,
            ),
        ]);
    } else {
        parameters.push((
            7,
            "head_across_flats",
            entry.head_across_flats_mm.ok_or("Missing head AF")?,
        ));
    }
    let mut edits = Vec::new();
    let mut exposed = Vec::new();
    for (role, name, value) in parameters {
        let mut parameter = Entity::new(
            name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        parameter.id = identity(entry, role)?;
        exposed.push(parameter.id);
        edits.push(Mutation::Insert(parameter));
    }
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Screw source dimensions".into(),
        edits,
    })?;
    let dimension = |name| -> Result<Dimension, Error> {
        Ok(Dimension::Expression {
            expression: BoundExpression::bind(s.document(), &format!("=${name}"), "mm", None)?,
        })
    };
    let mut parameters = literal(entry)?;
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        shank_diameter,
        head,
        drive,
        thread,
        ..
    } = &mut parameters
    else {
        unreachable!()
    };
    *underhead_length = Some(dimension("underhead_length")?);
    *shank_diameter = dimension("shank_diameter")?;
    thread.major_diameter = dimension("nominal_thread_diameter")?;
    thread.pitch = dimension("pitch")?;
    thread.length = dimension("thread_length")?;
    match head {
        ScrewHead::SocketCap { diameter, height } => {
            *diameter = dimension("head_diameter")?;
            *height = dimension("head_height")?;
        }
        ScrewHead::ExternalHex {
            across_flats,
            height,
        } => {
            *across_flats = dimension("head_across_flats")?;
            *height = dimension("head_height")?;
        }
        _ => return Err("Legacy screw curation cannot construct new head variants".into()),
    }
    if let ScrewDrive::HexSocket {
        across_flats,
        depth,
    } = drive
    {
        *across_flats = dimension("socket_across_flats")?;
        *depth = dimension("socket_depth")?;
    }
    let mut request = MechanicalPartRequest::new(&name, parameters);
    request.feature = identity(entry, 10)?;
    request.body = identity(entry, 11)?;
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: format!("toskcad.bolts.{}", entry.id),
        revision: format!(
            "5c83300a57761f70db87f50c9a1f647c7278a4c2 / machine screw curation {}",
            c.revision
        ),
        designation: format!("{} · dimensional reference", entry.source_product),
        source_url: c
            .data_sources
            .iter()
            .find(|source| {
                source.file.ends_with(if kind == "socket" {
                    "hex_socket.blt"
                } else {
                    "/hex.blt"
                })
            })
            .ok_or("Missing source")?
            .url
            .clone(),
        reference_geometry: literal(entry)?,
        catalog_facts: Some(MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(entry.nominal_mm),
            series: Some(
                if kind == "socket" {
                    "HPC CHCSS 2025 dimensional envelope; surface unspecified"
                } else {
                    "DIN 933 dimensional reference; Norelem 07171"
                }
                .into(),
            ),
            material: None,
            grade: None,
        }),
    });
    let p = model::prepare_mechanical_part(&s, s.revision(), actor(), &request)?;
    Compiler::default().compile(p.prepared.document(), s.revision() + 1)?;
    s.commit(p.prepared)?;
    let mut author =
        SymbolAuthorRequest::new(&name, request.body, SymbolKind::Dynamic, exposed, None);
    author.definition = identity(entry, 12)?;
    author.symbol = identity(entry, 13)?;
    author.revision = c.revision.clone();
    let p = symbols::prepare_author(&s, s.revision(), actor(), &author)?;
    s.commit(p.prepared)?;
    let preview = plant_library::native_preview::render_symbol_preview_with_placement(
        s.document(),
        author.definition,
        multicad_core::document::Placement {
            translation_mm: [0.0; 3],
            rotation_xyzw: [1.0, 0.0, 0.0, 0.0],
        },
        |_| [0.62, 0.64, 0.68],
    )?;
    let info = FileInfo {
        name,
        description: format!(
            "Metric M{} × {} mm underhead length, {} mm thread length; {}. Editable source dimensions, nominal/basic thread without a fit class. Flat-ended hex socket where present. Bottom tooling, surface detail, fillets, chamfers and imperfect-thread transitions are omitted. No supplier, stock, strength or current-standard certification.",
            entry.nominal_mm, entry.underhead_length_mm, entry.thread_length_mm, entry.head_surface
        ),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(s.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            entity.id = identity(entry, 14)?;
        }
    }
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Screw Library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(s.document().clone())?)
}
pub(super) fn generate(output: &Path) -> Result<(), Error> {
    let c = curated()?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("library.json"))?)?;
    if manifest["format"] != "toskcad.bundled-library" || manifest["version"] != 1 {
        return Err("Unsupported Library manifest".into());
    }
    let mut entries = Vec::new();
    for entry in manifest["entries"]
        .as_array()
        .ok_or("Missing Library entries")?
    {
        let file = entry["file"].as_str().ok_or("Missing filename")?;
        checked_relative(file)?;
        if file.contains('/') || !file.ends_with(".tcd") {
            return Err("Invalid project filename".into());
        }
        if c.entries.iter().any(|e| e.file == file) {
            continue;
        }
        let bytes = std::fs::read(source.join(file))?;
        if project_file::ContentHash::of(&bytes).to_string()
            != entry["sha256"].as_str().ok_or("Missing checksum")?
        {
            return Err("Existing Library project differs".into());
        }
        write_new(&output.join(file), &bytes)?;
        entries.push(entry.clone());
    }
    for entry in &c.entries {
        let project = build(&c, entry)?;
        let mut bytes = Vec::new();
        project_file::write(&project, &mut bytes, Default::default())?;
        write_new(&output.join(&entry.file), &bytes)?;
        entries.push(serde_json::json!({"file":entry.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()}));
        println!("{} · {} bytes", entry.file, bytes.len());
    }
    let mut requires: Vec<String> = serde_json::from_value(manifest["requires"].clone())?;
    for file in [
        "machine-screws-curation.json",
        "sources/bolts/hex_socket.blt",
        "sources/bolts/hex.blt",
        "sources/bolts/MACHINE-SCREW-NOTICES.md",
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
        std::fs::create_dir_all(destination.parent().ok_or("Missing parent")?)?;
        write_new(&destination, &std::fs::read(path)?)?;
    }
    let value = serde_json::json!({"format":"toskcad.bundled-library","version":1,"revision":"6","entries":entries,"requires":requires});
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    write_new(&output.join("library.json"), &bytes)?;
    Ok(())
}
