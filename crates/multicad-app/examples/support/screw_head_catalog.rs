//! Developer-only adapter for retained sourced screw-profile presets.
use super::{Error, actor, checked_relative, plant_library, write_new};
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, MechanicalPartCatalogFacts, MechanicalPartParameters, MechanicalPartProvenance,
        MechanicalPartRequest, ScrewDrive, ScrewHead, ScrewLengthInput, ThreadParameters,
        ThreadRepresentation, length,
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
const FAMILIES: [&str; 8] = [
    "iso14583-2011",
    "iso14581-2013",
    "iso7045-2011-h",
    "iso7046-1-2011-h",
    "fila-iso14579",
    "iso7380-1-2011",
    "iso10642-2004",
    "accu-shb-m3-10-a2-r360",
];
const NOMINALS: [u64; 12] = [
    2000, 2500, 3000, 3500, 4000, 4500, 5000, 6000, 7000, 8000, 9000, 10000,
];
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curation {
    pub revision: String,
    pub sources: Vec<Source>,
    pub entries: Vec<Entry>,
    pub coverage: Vec<Coverage>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub url: String,
    pub sha256: String,
    pub edition: String,
    pub reference: String,
    pub rights: String,
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
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub file: String,
    pub family: String,
    pub nominal_mm: f64,
    pub pitch_mm: f64,
    pub length_mm: f64,
    pub length_convention: String,
    pub thread_runout_mm: f64,
    pub shank_diameter_mm: f64,
    pub head: ScrewHead,
    pub drive: ScrewDrive,
    pub source_facts: SourceFacts,
    pub profile_facts: Option<ProfileFacts>,
    pub source_reference: String,
    pub verification_url: String,
    pub modifications: Vec<String>,
    pub engineering_checks: EngineeringChecks,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringChecks {
    pub entry_radius_mm: f64,
    pub neck_radius_mm: f64,
    pub unthreaded_transition_mm: f64,
    pub thread_length_at_default_mm: f64,
    pub rim_height_mm: Option<f64>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFacts {
    pub id: String,
    pub family: String,
    pub nominal_mm: f64,
    pub pitch_mm: f64,
    pub selected_length_mm: f64,
    pub available_lengths_mm: Vec<f64>,
    pub length_scope: String,
    pub length_convention: String,
    pub availability_kind: String,
    pub source_head: HeadFacts,
    pub source_drive: DriveFacts,
    pub source_thread: ThreadFacts,
    pub verification: Verification,
    pub upstream_starting_point: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub url: String,
    pub reference: String,
    pub source_sha256: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadFacts {
    pub actual_diameter_min_mm: Option<f64>,
    pub actual_diameter_max_mm: Option<f64>,
    pub height_min_mm: Option<f64>,
    pub height_max_mm: Option<f64>,
    pub crown_radius_approx_mm: Option<f64>,
    pub neck_radius_min_mm: Option<f64>,
    pub head_included_angle_deg: Option<f64>,
    pub theoretical_diameter_max_mm: Option<f64>,
    pub neck_radius_max_mm: Option<f64>,
    pub head_included_angle_min_deg: Option<f64>,
    pub head_included_angle_max_deg: Option<f64>,
    pub reinforcement_shoulder: Option<String>,
    pub crown_radius_min_mm: Option<f64>,
    pub crown_radius_max_mm: Option<f64>,
    pub fully_threaded_underhead_radius_min_mm: Option<f64>,
    pub flat_land_diameter_reference_mm: Option<f64>,
    #[serde(rename = "flushness_F_max_mm")]
    pub flushness_f_max_mm: Option<f64>,
    pub edge: Option<String>,
    pub crown_radius_mm: Option<f64>,
    pub crown_radius_source: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveFacts {
    pub size: Option<f64>,
    #[serde(rename = "A_reference_mm")]
    pub a_reference_mm: Option<f64>,
    pub functional_penetration_min_mm: Option<f64>,
    pub functional_penetration_max_mm: Option<f64>,
    pub depth_kind: Option<String>,
    pub profile_source: Option<String>,
    pub m_reference_mm: Option<f64>,
    pub gauge_penetration_min_mm: Option<f64>,
    pub gauge_penetration_max_mm: Option<f64>,
    #[serde(rename = "H0_radius_exception_required")]
    pub h0_radius_exception_required: Option<bool>,
    pub nominal_key_mm: Option<f64>,
    pub across_flats_min_mm: Option<f64>,
    pub across_flats_max_mm: Option<f64>,
    pub straight_depth_min_mm: Option<f64>,
    pub remaining_dimension_w_min_mm: Option<f64>,
    pub mouth: Option<String>,
    pub bottom: Option<String>,
    pub bottom_included_angle_min_deg: Option<f64>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadFacts {
    pub a_max_mm: Option<f64>,
    pub b_min_mm: Option<f64>,
    pub short_length_rule: Option<String>,
    pub rule_scope: Option<String>,
    pub thread_length_b_mm: Option<f64>,
    pub pitch_basis: Option<String>,
    pub b_reference_mm: Option<f64>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ProfileFacts {
    Phillips(Box<PhillipsFacts>),
    Hexalobular(HexalobularFacts),
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhillipsFacts {
    pub size: u32,
    pub b_min_mm: f64,
    pub b_max_mm: f64,
    pub e_min_mm: f64,
    pub e_max_mm: f64,
    pub g_min_mm: f64,
    pub g_max_mm: f64,
    pub f_min_mm: f64,
    pub f_max_mm: f64,
    pub mouth_radius_nominal_mm: f64,
    pub t1_reference_mm: f64,
    pub t1_independently_computed_mm: f64,
    pub alpha_nominal_deg: Option<f64>,
    pub alpha_tolerance_min_deg: Option<f64>,
    pub beta_min_deg: f64,
    pub beta_max_deg: f64,
    pub outer_flank_min_deg: f64,
    pub outer_flank_max_deg: f64,
    pub floor_angle_min_deg: f64,
    pub floor_angle_max_deg: f64,
    pub outer_corner_angle_min_deg: f64,
    pub outer_corner_angle_max_deg: f64,
    pub inner_radius_instead_of_alpha_min_mm: Option<f64>,
    pub inner_radius_instead_of_alpha_max_mm: Option<f64>,
    pub engineering_target: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HexalobularFacts {
    pub size: u32,
    #[serde(rename = "A_basic_mm")]
    pub a_basic_mm: f64,
    #[serde(rename = "B_basic_mm")]
    pub b_basic_mm: f64,
    pub arc_radii: String,
}
fn microns(value: f64) -> Result<u64, Error> {
    let n = value * 1000.;
    if !n.is_finite() || !(1. ..=100_000.).contains(&n) || (n - n.round()).abs() > 1e-8 {
        return Err("Screw identity needs an exact positive micrometer key".into());
    }
    Ok(n.round() as u64)
}
fn identity(entry: &Entry, role: u64) -> Result<EntityId, Error> {
    let family: u64 = match entry.family.as_str() {
        "iso14583-2011" => 1,
        "iso14581-2013" => 2,
        "iso7045-2011-h" => 3,
        "iso7046-1-2011-h" => 4,
        "fila-iso14579" => 5,
        "iso7380-1-2011" => 6,
        "iso10642-2004" => 7,
        "accu-shb-m3-10-a2-r360" => 8,
        _ => return Err("Unknown sourced screw family".into()),
    };
    if role >= 100 {
        return Err("Screw identity role exceeds reserved range".into());
    }
    let key = (family * 100_000_000_000
        + microns(entry.nominal_mm)? * 1_000_000
        + microns(entry.length_mm)?)
        * 100
        + role;
    Ok(EntityId::parse(&format!(
        "cad23600-0000-4000-8000-{key:012x}"
    ))?)
}
fn scalar(d: &Dimension, unit: &str) -> Result<f64, Error> {
    match d {
        Dimension::Literal {
            value,
            unit: actual,
        } if actual == unit && value.is_finite() => Ok(*value),
        _ => Err("Catalog target must retain a finite literal with the exact declared unit".into()),
    }
}
fn literal(entry: &Entry) -> Result<MechanicalPartParameters, Error> {
    let height = match &entry.head {
        ScrewHead::SocketCap { height, .. } | ScrewHead::ExternalHex { height, .. } => {
            scalar(height, "mm")?
        }
        ScrewHead::Pan { profile } | ScrewHead::Button { profile } => {
            scalar(&profile.height, "mm")?
        }
        ScrewHead::Countersunk { profile } => scalar(&profile.height, "mm")?,
    };
    let (length_input, underhead) = match entry.length_convention.as_str() {
        "underhead" => (
            ScrewLengthInput::Underhead {
                value: length(entry.length_mm),
            },
            entry.length_mm,
        ),
        "overall" => (
            ScrewLengthInput::Overall {
                value: length(entry.length_mm),
            },
            entry.length_mm - height,
        ),
        _ => return Err("Unknown sourced screw length convention".into()),
    };
    Ok(MechanicalPartParameters::MachineScrew {
        underhead_length: None,
        length_input: Some(length_input),
        shank_diameter: length(entry.shank_diameter_mm),
        head: entry.head.clone(),
        drive: entry.drive.clone(),
        thread: Box::new(ThreadParameters {
            major_diameter: length(entry.nominal_mm),
            pitch: length(entry.pitch_mm),
            length: length(underhead - entry.thread_runout_mm),
            representation: ThreadRepresentation::Modeled,
            representation_parameter: None,
            ..Default::default()
        }),
    })
}
fn bounds(value: f64, min: Option<f64>, max: Option<f64>) -> bool {
    min.is_none_or(|m| value >= m) && max.is_none_or(|m| value <= m)
}
fn validate_entry(entry: &Entry) -> Result<(), Error> {
    let sf = &entry.source_facts;
    let h = &sf.source_head;
    let d = &sf.source_drive;
    let slug = |v: f64| v.to_string().replace('.', "-");
    let expected = format!(
        "screw.{}.m{}.l{}",
        entry.family,
        slug(entry.nominal_mm),
        slug(entry.length_mm)
    );
    let expected_pitch = match microns(entry.nominal_mm)? {
        2000 => 0.4,
        2500 => 0.45,
        3000 => 0.5,
        3500 => 0.6,
        4000 => 0.7,
        5000 => 0.8,
        6000 => 1.,
        8000 => 1.25,
        10000 => 1.5,
        _ => return Err("No selected screw nominal row".into()),
    };
    if !FAMILIES.contains(&entry.family.as_str())
        || entry.id != expected
        || entry.file != format!("{}.tcd", expected.replace('.', "-"))
        || entry.pitch_mm != expected_pitch
        || entry.nominal_mm != sf.nominal_mm
        || entry.pitch_mm != sf.pitch_mm
        || entry.length_mm != sf.selected_length_mm
        || sf.available_lengths_mm != [entry.length_mm]
        || entry.family != sf.family
        || entry.id != sf.id
        || entry.shank_diameter_mm != entry.nominal_mm
        || !entry.thread_runout_mm.is_finite()
        || entry.thread_runout_mm <= 0.
        || entry.source_reference.is_empty()
        || !entry.verification_url.starts_with("https://")
        || entry.verification_url != sf.verification.url
        || sf.verification.source_sha256.len() != 64
        || entry.modifications.is_empty()
        || entry.modifications.iter().any(String::is_empty)
        || sf.length_scope.is_empty()
        || sf.availability_kind.is_empty()
        || sf.verification.reference.is_empty()
    {
        return Err("Sourced screw identity, pitch, source or discrete length disagrees".into());
    }
    checked_relative(&entry.file)?;
    let expected_form = match entry.family.as_str() {
        "iso14583-2011" => matches!(
            (&entry.head, &entry.drive),
            (ScrewHead::Pan { .. }, ScrewDrive::Hexalobular { .. })
        ),
        "iso14581-2013" => matches!(
            (&entry.head, &entry.drive),
            (
                ScrewHead::Countersunk { .. },
                ScrewDrive::Hexalobular { .. }
            )
        ),
        "iso7045-2011-h" => matches!(
            (&entry.head, &entry.drive),
            (ScrewHead::Pan { .. }, ScrewDrive::PhillipsH { .. })
        ),
        "iso7046-1-2011-h" => matches!(
            (&entry.head, &entry.drive),
            (ScrewHead::Countersunk { .. }, ScrewDrive::PhillipsH { .. })
        ),
        "fila-iso14579" => matches!(
            (&entry.head, &entry.drive),
            (ScrewHead::SocketCap { .. }, ScrewDrive::Hexalobular { .. })
        ),
        "iso7380-1-2011" => matches!(
            (&entry.head, &entry.drive),
            (ScrewHead::Button { .. }, ScrewDrive::HexSocket { .. })
        ),
        "iso10642-2004" => matches!(
            (&entry.head, &entry.drive),
            (ScrewHead::Countersunk { .. }, ScrewDrive::HexSocket { .. })
        ),
        "accu-shb-m3-10-a2-r360" => matches!(
            (&entry.head, &entry.drive),
            (ScrewHead::Button { .. }, ScrewDrive::Hexalobular { .. })
        ),
        _ => false,
    };
    if !expected_form {
        return Err("Source family does not permit this head/drive combination".into());
    }
    if h.actual_diameter_min_mm.is_none()
        || (h.height_min_mm.is_none() && h.height_max_mm.is_none())
    {
        return Err("Source row omits its actual diameter or head-height bound".into());
    }
    if (entry.length_convention == "overall") != (sf.length_convention == "head_inclusive") {
        return Err("Sourced screw length convention changed".into());
    }
    if sf
        .source_thread
        .a_max_mm
        .is_some_and(|maximum| entry.thread_runout_mm > maximum)
    {
        return Err("Plain transition exceeds the selected short-thread source bound".into());
    }
    let (diameter, height, neck, crown, land) = match &entry.head {
        ScrewHead::Pan { profile } | ScrewHead::Button { profile } => (
            scalar(&profile.diameter, "mm")?,
            scalar(&profile.height, "mm")?,
            scalar(&profile.neck_radius, "mm")?,
            Some(scalar(&profile.crown_radius, "mm")?),
            Some(scalar(&profile.top_land_diameter, "mm")?),
        ),
        ScrewHead::Countersunk { profile } => {
            let a = scalar(&profile.included_angle, "deg")?;
            if !bounds(
                a,
                h.head_included_angle_min_deg,
                h.head_included_angle_max_deg,
            ) || scalar(&profile.edge_chamfer, "mm")? <= 0.
            {
                return Err("Countersunk angle/chamfer falls outside retained target".into());
            }
            (
                scalar(&profile.diameter, "mm")?,
                scalar(&profile.height, "mm")?,
                scalar(&profile.neck_radius, "mm")?,
                None,
                None,
            )
        }
        ScrewHead::SocketCap { diameter, height } => (
            scalar(diameter, "mm")?,
            scalar(height, "mm")?,
            0.,
            None,
            None,
        ),
        ScrewHead::ExternalHex { .. } => {
            return Err("Additional source matrix has no external hex rows".into());
        }
    };
    if !bounds(
        diameter,
        h.actual_diameter_min_mm,
        h.actual_diameter_max_mm.or(h.theoretical_diameter_max_mm),
    ) || !bounds(height, h.height_min_mm, h.height_max_mm)
        || !bounds(
            neck,
            h.fully_threaded_underhead_radius_min_mm
                .or(h.neck_radius_min_mm),
            h.neck_radius_max_mm,
        )
        || crown.is_some_and(|v| !bounds(v, h.crown_radius_min_mm, h.crown_radius_max_mm))
        || land.is_some_and(|v| {
            h.flat_land_diameter_reference_mm
                .is_some_and(|expected| (v - expected).abs() > 1e-12)
        })
    {
        return Err("Head target exceeds its retained source scalar limits".into());
    }
    match (&entry.drive, &entry.profile_facts) {
        (
            ScrewDrive::HexSocket {
                across_flats,
                depth,
            },
            None,
        ) => {
            let af = scalar(across_flats, "mm")?;
            let depth = scalar(depth, "mm")?;
            if !bounds(af, d.across_flats_min_mm, d.across_flats_max_mm)
                || d.straight_depth_min_mm
                    .is_none_or(|minimum| depth < minimum)
                || d.remaining_dimension_w_min_mm
                    .is_none_or(|minimum| height - depth < minimum)
            {
                return Err("Hex socket fails sourced AF/depth/remaining floor limits".into());
            }
        }
        (ScrewDrive::Hexalobular { profile }, Some(ProfileFacts::Hexalobular(f))) => {
            let model::HexalobularDriveParameters {
                major_diameter,
                minor_diameter,
                outer_lobe_radius,
                depth,
                bottom_angle,
            } = profile.as_ref();
            let a = scalar(major_diameter, "mm")?;
            let b = scalar(minor_diameter, "mm")?;
            if Some(f.size as f64) != d.size
                || a != f.a_basic_mm
                || b != f.b_basic_mm
                || !bounds(
                    scalar(depth, "mm")?,
                    d.functional_penetration_min_mm,
                    d.functional_penetration_max_mm,
                )
                || scalar(bottom_angle, "deg")? != 120.
                || f.arc_radii.is_empty()
            {
                return Err(
                    "Hexalobular target differs from basic contour/depth references".into(),
                );
            }
            model::HexalobularContour::new(a, b, scalar(outer_lobe_radius, "mm")?)?;
        }
        (ScrewDrive::PhillipsH { profile }, Some(ProfileFacts::Phillips(f))) => {
            use model::{PhillipsHSize, PhillipsInnerForm};
            let size = match profile.size {
                PhillipsHSize::H0 => 0,
                PhillipsHSize::H1 => 1,
                PhillipsHSize::H2 => 2,
                PhillipsHSize::H3 => 3,
                PhillipsHSize::H4 => 4,
            };
            if f.size != size
                || Some(size as f64) != d.size
                || d.m_reference_mm != Some(scalar(&profile.mouth_diameter, "mm")?)
                || !bounds(
                    scalar(&profile.b_width, "mm")?,
                    Some(f.b_min_mm),
                    Some(f.b_max_mm),
                )
                || !bounds(
                    scalar(&profile.g_diameter, "mm")?,
                    Some(f.g_min_mm),
                    Some(f.g_max_mm),
                )
                || scalar(&profile.mouth_radius, "mm")? != f.mouth_radius_nominal_mm
                || !bounds(
                    scalar(&profile.outer_flank_angle, "deg")?,
                    Some(f.outer_flank_min_deg),
                    Some(f.outer_flank_max_deg),
                )
                || !bounds(
                    scalar(&profile.floor_angle, "deg")?,
                    Some(f.floor_angle_min_deg),
                    Some(f.floor_angle_max_deg),
                )
                || !bounds(
                    scalar(&profile.tilt_angle, "deg")?,
                    Some(f.beta_min_deg),
                    Some(f.beta_max_deg),
                )
                || !bounds(
                    scalar(&profile.outer_corner_angle, "deg")?,
                    Some(f.outer_corner_angle_min_deg),
                    Some(f.outer_corner_angle_max_deg),
                )
            {
                return Err(
                    "Phillips target differs from its retained theoretical scalar ranges".into(),
                );
            }
            match &profile.inner {
                PhillipsInnerForm::Rounded { radius } if size == 0 => {
                    let r = scalar(radius, "mm")?;
                    let e = 2.
                        * r
                        * (scalar(&profile.outer_corner_angle, "deg")?.to_radians() / 2.).cos();
                    if !bounds(
                        r,
                        f.inner_radius_instead_of_alpha_min_mm,
                        f.inner_radius_instead_of_alpha_max_mm,
                    ) || !bounds(e, Some(f.e_min_mm), Some(f.e_max_mm))
                    {
                        return Err("H0 radius/derived e fails source limits".into());
                    }
                }
                PhillipsInnerForm::ShallowV { angle, e_width } if size != 0 => {
                    if !bounds(
                        scalar(angle, "deg")?,
                        f.alpha_tolerance_min_deg,
                        f.alpha_nominal_deg,
                    ) || !bounds(scalar(e_width, "mm")?, Some(f.e_min_mm), Some(f.e_max_mm))
                    {
                        return Err("H1-H4 inner form fails source limits".into());
                    }
                }
                _ => return Err("Phillips inner form does not match source size".into()),
            }
        }
        _ => return Err("Drive/source profile facts disagree".into()),
    }
    if entry.engineering_checks.neck_radius_mm != neck
        || entry.engineering_checks.unthreaded_transition_mm != entry.thread_runout_mm
    {
        return Err("Retained transition checks disagree with selected target".into());
    }
    let validation = Session::new(Document::new("Screw profile catalog validation")?)?;
    let model::ResolvedMechanicalPart::MachineScrew {
        head,
        drive,
        thread,
        shank_radius_mm,
        ..
    } = literal(entry)?.resolve(validation.parameters())?
    else {
        return Err("Expected resolved catalog screw".into());
    };
    if (drive.entry_radius_mm() - entry.engineering_checks.entry_radius_mm).abs() > 1e-10
        || (thread.length_mm - entry.engineering_checks.thread_length_at_default_mm).abs() > 1e-10
    {
        return Err("Declared default geometry checks do not match resolved source".into());
    }
    match head {
        model::ResolvedScrewHead::Countersunk {
            radius_mm,
            height_mm,
            included_angle_deg,
            ..
        } => {
            let rim = height_mm
                - (radius_mm - shank_radius_mm) / (included_angle_deg.to_radians() / 2.).tan();
            if entry
                .engineering_checks
                .rim_height_mm
                .is_none_or(|expected| (rim - expected).abs() > 1e-10)
            {
                return Err(
                    "Declared countersunk rim differs from retained cone dimensions".into(),
                );
            }
        }
        _ if entry.engineering_checks.rim_height_mm.is_some() => {
            return Err("Only countersunk source can declare a cone rim".into());
        }
        _ => {}
    }
    Ok(())
}
pub fn curated() -> Result<Curation, Error> {
    super::machine_screw_catalog::validate_legacy_curation()?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let bytes = std::fs::read(source.join("machine-screws-curation.json"))?;
    if bytes.as_slice()
        != include_bytes!("../../../../library/machine-screws-curation.json").as_slice()
    {
        return Err("Runtime screw curation differs from compiled data".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let c: Curation = serde_json::from_value(
        serde_json::json!({"revision":value["profile_revision"],"sources":value["profile_sources"],"entries":value["profile_entries"],"coverage":value["profile_coverage"]}),
    )?;
    if c.revision != "1" || c.entries.len() != 52 || c.coverage.len() != 96 || c.sources.len() != 11
    {
        return Err("Incomplete additional screw source matrix".into());
    }
    let mut source_ids = BTreeSet::new();
    for s in &c.sources {
        if !source_ids.insert(&s.id)
            || !s.url.starts_with("https://")
            || s.sha256.len() != 64
            || !s.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || s.edition.is_empty()
            || s.reference.is_empty()
            || s.rights.is_empty()
        {
            return Err("Invalid or duplicate primary source record".into());
        }
    }
    let mut ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for e in &c.entries {
        validate_entry(e)?;
        if !ids.insert(&e.id)
            || !files.insert(&e.file)
            || !c.sources.iter().any(|s| {
                s.url == e.verification_url && s.sha256 == e.source_facts.verification.source_sha256
            })
        {
            return Err("Duplicate screw row or missing primary source binding".into());
        }
        for role in 0..100 {
            if !identities.insert(identity(e, role)?) {
                return Err("Screw preset stable identities overlap".into());
            }
        }
    }
    let mut coverage = BTreeSet::new();
    for r in &c.coverage {
        let key = microns(r.nominal_mm)?;
        if !FAMILIES.contains(&r.family.as_str())
            || !NOMINALS.contains(&key)
            || !coverage.insert((&r.family, key))
            || r.reason.is_empty()
        {
            return Err("Invalid screw family/nominal coverage".into());
        }
        let entry = c
            .entries
            .iter()
            .find(|e| e.family == r.family && e.nominal_mm == r.nominal_mm);
        match (r.status.as_str(), entry, r.entry_id.as_deref()) {
            ("supported", Some(e), Some(id)) if id == e.id => {}
            ("unavailable", None, None) => {}
            _ => return Err("Coverage does not match actual sourced screw rows".into()),
        }
    }
    Ok(c)
}
fn map_dimensions(
    parameters: &mut MechanicalPartParameters,
    f: &mut impl FnMut(u64, &str, &mut Dimension) -> Result<(), Error>,
) -> Result<(), Error> {
    let MechanicalPartParameters::MachineScrew {
        length_input,
        shank_diameter,
        head,
        drive,
        thread,
        ..
    } = parameters
    else {
        return Err("Expected screw parameters".into());
    };
    match length_input.as_mut().ok_or("Expected typed screw length")? {
        ScrewLengthInput::Underhead { value } => f(1, "underhead_length", value)?,
        ScrewLengthInput::Overall { value } => f(1, "overall_length", value)?,
    };
    f(2, "nominal_thread_diameter", &mut thread.major_diameter)?;
    f(3, "pitch", &mut thread.pitch)?;
    f(5, "shank_diameter", shank_diameter)?;
    match head {
        ScrewHead::Pan { profile } | ScrewHead::Button { profile } => {
            let model::CrownedScrewHeadParameters {
                diameter,
                height,
                crown_radius,
                top_land_diameter,
                neck_radius,
            } = profile.as_mut();
            f(10, "head_height", height)?;
            f(11, "head_diameter", diameter)?;
            f(12, "neck_radius", neck_radius)?;
            f(13, "crown_radius", crown_radius)?;
            f(14, "top_land_diameter", top_land_diameter)?;
        }
        ScrewHead::Countersunk { profile } => {
            let model::CountersunkScrewHeadParameters {
                diameter,
                height,
                included_angle,
                edge_chamfer,
                neck_radius,
            } = profile.as_mut();
            f(10, "head_height", height)?;
            f(11, "head_diameter", diameter)?;
            f(12, "neck_radius", neck_radius)?;
            f(15, "head_included_angle", included_angle)?;
            f(16, "head_edge_chamfer", edge_chamfer)?;
        }
        ScrewHead::SocketCap { diameter, height } => {
            f(10, "head_height", height)?;
            f(11, "head_diameter", diameter)?;
        }
        ScrewHead::ExternalHex { .. } => return Err("Unexpected external-hex source target".into()),
    }
    match drive {
        ScrewDrive::HexSocket {
            across_flats,
            depth,
        } => {
            f(20, "socket_across_flats", across_flats)?;
            f(21, "drive_depth", depth)?;
        }
        ScrewDrive::Hexalobular { profile } => {
            let model::HexalobularDriveParameters {
                major_diameter,
                minor_diameter,
                outer_lobe_radius,
                depth,
                bottom_angle,
            } = profile.as_mut();
            f(21, "drive_depth", depth)?;
            f(22, "drive_major_diameter", major_diameter)?;
            f(23, "drive_minor_diameter", minor_diameter)?;
            f(24, "drive_outer_lobe_radius", outer_lobe_radius)?;
            f(25, "drive_bottom_angle", bottom_angle)?;
        }
        ScrewDrive::PhillipsH { profile } => {
            f(30, "drive_mouth_diameter", &mut profile.mouth_diameter)?;
            f(31, "drive_mouth_radius", &mut profile.mouth_radius)?;
            f(32, "drive_b_width", &mut profile.b_width)?;
            f(33, "drive_g_diameter", &mut profile.g_diameter)?;
            f(34, "drive_tilt_angle", &mut profile.tilt_angle)?;
            f(
                35,
                "drive_outer_flank_angle",
                &mut profile.outer_flank_angle,
            )?;
            f(36, "drive_floor_angle", &mut profile.floor_angle)?;
            f(
                37,
                "drive_outer_corner_angle",
                &mut profile.outer_corner_angle,
            )?;
            match &mut profile.inner {
                model::PhillipsInnerForm::Rounded { radius } => {
                    f(40, "drive_inner_radius", radius)?
                }
                model::PhillipsInnerForm::ShallowV { angle, e_width } => {
                    f(38, "drive_inner_angle", angle)?;
                    f(39, "drive_e_width", e_width)?;
                }
            }
        }
        ScrewDrive::ExternalHex => return Err("Unexpected external drive".into()),
    }
    Ok(())
}
pub fn build(c: &Curation, index: usize) -> Result<ProjectFile, Error> {
    let entry = c
        .entries
        .get(index)
        .ok_or("Missing curated screw profile")?;
    validate_entry(entry)?;
    let name = format!(
        "M{} × {} · {}",
        entry.nominal_mm, entry.length_mm, entry.family
    );
    let document: Document = serde_json::from_value(
        serde_json::json!({"schema_version":multicad_core::document::DOCUMENT_SCHEMA,"id":identity(entry,0)?,"name":name,"entities":{}}),
    )?;
    let mut session = Session::new(document)?;
    let mut parameters = literal(entry)?;
    let mut fields = Vec::new();
    map_dimensions(&mut parameters, &mut |role, name, dimension| {
        fields.push((
            role,
            name.to_owned(),
            scalar(dimension, dimension.unit())?,
            dimension.unit().to_owned(),
        ));
        Ok(())
    })?;
    fields.push((
        4,
        "thread_runout".into(),
        entry.thread_runout_mm,
        "mm".into(),
    ));
    fields.push((6, "thread_representation".into(), 1., "count".into()));
    let mut edits = Vec::new();
    let mut exposed = Vec::new();
    let mut names = BTreeSet::new();
    let mut roles = BTreeSet::new();
    for (role, name, value, unit) in fields {
        if !roles.insert(role) || !names.insert(name.clone()) {
            return Err("Duplicate screw parameter role/name".into());
        }
        let mut parameter = Entity::new(
            name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit,
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
        label: "Sourced screw dimensions".into(),
        edits,
    })?;
    map_dimensions(&mut parameters, &mut |_, name, dimension| {
        *dimension = Dimension::Expression {
            expression: BoundExpression::bind(
                session.document(),
                &format!("=${name}"),
                dimension.unit(),
                None,
            )?,
        };
        Ok(())
    })?;
    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut parameters else {
        unreachable!()
    };
    let thread_expression = if entry.length_convention == "overall" {
        "=$overall_length - $head_height - $thread_runout"
    } else {
        "=$underhead_length - $thread_runout"
    };
    thread.length = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), thread_expression, "mm", None)?,
    };
    thread.representation_parameter = Some(identity(entry, 6)?);
    let mut request = MechanicalPartRequest::new(&name, parameters);
    request.feature = identity(entry, 80)?;
    request.body = identity(entry, 81)?;
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: format!("toskcad.sourced.{}", entry.id),
        revision: format!("screw profile curation {}", c.revision),
        designation: format!(
            "{} M{} × {} dimensional reference",
            entry.family, entry.nominal_mm, entry.length_mm
        ),
        source_url: entry.verification_url.clone(),
        reference_geometry: literal(entry)?,
        catalog_facts: Some(MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(entry.nominal_mm),
            series: Some(format!(
                "{}; source dimensions with explicit engineering profile",
                entry.family
            )),
            material: None,
            grade: None,
        }),
    });
    let prepared = model::prepare_mechanical_part(&session, session.revision(), actor(), &request)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.prepared)?;
    let mut author =
        SymbolAuthorRequest::new(&name, request.body, SymbolKind::Dynamic, exposed, None);
    author.definition = identity(entry, 82)?;
    author.symbol = identity(entry, 83)?;
    author.revision = c.revision.clone();
    let prepared = symbols::prepare_author(&session, session.revision(), actor(), &author)?;
    session.commit(prepared.prepared)?;
    let preview = plant_library::native_preview::render_symbol_preview_with_placement(
        session.document(),
        author.definition,
        multicad_core::document::Placement {
            translation_mm: [0.; 3],
            rotation_xyzw: [1., 0., 0., 0.],
        },
        |_| [0.62, 0.64, 0.68],
    )?;
    let info = FileInfo {
        name,
        description: format!(
            "{} M{} × {} dimensional reference; {} length, positive retained thread transition and modeled/cosmetic choice. Independently authored circular/planar head and recess engineering targets; original source limits remain in the bundled curation. Nominal-basic thread, no fit class, gauge, physical driver fit, strength, material, coating or stock certification.",
            entry.family, entry.nominal_mm, entry.length_mm, entry.length_convention
        ),
        preview: Some(preview),
        capture_on_save: true,
    };
    let mut edits = project_file::info::edits(session.document(), &info)?;
    for edit in &mut edits {
        if let Mutation::Insert(e) = edit {
            e.id = identity(entry, 84)?;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Screw profile Library card".into(),
        edits,
    })?;
    Ok(ProjectFile::new(session.document().clone())?)
}
#[derive(Clone, Debug)]
pub struct RegenerationReceipt {
    bytes: Vec<u8>,
    pin: project_file::ContentHash,
}
fn profile_pin(entry: &Entry, project: &ProjectFile) -> Result<project_file::ContentHash, Error> {
    let definitions = symbols::definitions(&project.document)?;
    if project.document.id() != identity(entry, 0)?
        || definitions.len() != 1
        || definitions[0].0 != identity(entry, 82)?
        || definitions[0].1.symbol != identity(entry, 83)?
        || definitions[0].1.kind != SymbolKind::Dynamic
    {
        return Err("Existing sourced screw identity differs; explicit migration required".into());
    }
    Ok(definitions[0].1.pin()?)
}
pub fn checked_existing_profile(
    entry: &Entry,
    manifest_entry: &serde_json::Value,
    bytes: &[u8],
) -> Result<RegenerationReceipt, Error> {
    if manifest_entry["file"].as_str() != Some(entry.file.as_str())
        || manifest_entry["sha256"].as_str()
            != Some(project_file::ContentHash::of(bytes).to_string().as_str())
    {
        return Err("Existing sourced screw filename/checksum differs".into());
    }
    let project = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    if manifest_entry["document"].as_str() != Some(project.document.id().to_string().as_str()) {
        return Err("Existing sourced screw manifest identity differs".into());
    }
    Ok(RegenerationReceipt {
        bytes: bytes.to_vec(),
        pin: profile_pin(entry, &project)?,
    })
}
pub fn verify_unchanged_profile(
    entry: &Entry,
    existing: &RegenerationReceipt,
    rebuilt: &ProjectFile,
    bytes: &[u8],
) -> Result<(), Error> {
    if profile_pin(entry, rebuilt)? != existing.pin || bytes != existing.bytes {
        return Err(
            "Rebuilt sourced screw pin or bytes differ; explicit migration required".into(),
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
        return Err("Screw profile generation needs an empty output directory".into());
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
                checked_existing_profile(curated, entry, &bytes)?,
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
            verify_unchanged_profile(entry, existing, &project, &bytes)?;
        }
        write_new(&output.join(&entry.file), &bytes)?;
        entries.push(serde_json::json!({"file":entry.file,"document":project.document.id(),"sha256":project_file::ContentHash::of(&bytes).to_string()}));
        println!("{} · {} bytes", entry.file, bytes.len());
    }
    let mut requires: Vec<String> = serde_json::from_value(manifest["requires"].clone())?;
    for file in [
        "machine-screws-curation.json",
        "sources/bolts/hex_socket.blt",
        "sources/bolts/COPYING.LESSER",
        "sources/bolts/SCREW-PROFILE-NOTICES.md",
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
