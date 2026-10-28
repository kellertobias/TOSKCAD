//! Developer-only independent review of standard-layer planar DXF bundles.
//! Usage: <source.multicad> <bundle-directory> <new-report.json>.
use dxf::{Drawing, entities::EntityType, enums::Units};
use multicad_core::{
    exchange::{Layers, RecipeOutput, WrittenFile, read},
    project_file::{self, ContentHash},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

type ReviewResult<T> = Result<T, Box<dyn std::error::Error>>;
const EPSILON_MM: f64 = 1e-6;

fn require(ok: bool, message: &str) -> ReviewResult<()> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn near(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && (a - b).abs() <= EPSILON_MM
}
fn include(bounds: &mut [[f64; 2]; 2], p: [f64; 2]) {
    for (axis, v) in p.into_iter().enumerate() {
        bounds[0][axis] = bounds[0][axis].min(v);
        bounds[1][axis] = bounds[1][axis].max(v);
    }
}

fn inspect(
    file: &Path,
    stored: &WrittenFile,
    units: &Units,
    layers: &Layers,
) -> ReviewResult<Value> {
    let bytes = fs::read(file)?;
    require(
        bytes.len() as u64 == stored.bytes,
        "Receipt byte count mismatch",
    )?;
    let hash = ContentHash::of(&bytes);
    require(hash == stored.content_hash, "Receipt SHA-256 mismatch")?;
    let drawing = Drawing::load_file(file)?;
    require(
        drawing.header.default_drawing_units == *units,
        "DXF units differ from saved recipe",
    )?;
    let mm_per_unit = match units {
        Units::Millimeters => 1.,
        Units::Centimeters => 10.,
        Units::Meters => 1000.,
        Units::Inches => 25.4,
        _ => return Err("Unsupported review unit".into()),
    };
    let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    let mut loops = vec![];
    let mut centers = vec![];
    let mut labels = vec![];
    let mut circles = vec![];
    let mut outer = 0;
    let mut inner = 0;
    let mut area_mm2 = 0.;
    for entity in drawing.entities() {
        match &entity.specific {
            EntityType::LwPolyline(poly) => {
                require(
                    poly.is_closed() && poly.vertices.len() >= 2,
                    "Open or empty cut polyline",
                )?;
                let mut signed_area = 0.;
                let mut circle: Option<([f64; 2], f64)> = None;
                let mut circular = true;
                let mut sweep_sum = 0.;
                let mut bulges = vec![];
                for (i, a) in poly.vertices.iter().enumerate() {
                    let b = &poly.vertices[(i + 1) % poly.vertices.len()];
                    let a = [a.x * mm_per_unit, a.y * mm_per_unit, a.bulge];
                    let b = [b.x * mm_per_unit, b.y * mm_per_unit];
                    require(
                        a.into_iter().chain(b).all(f64::is_finite),
                        "Non-finite cut geometry",
                    )?;
                    let dx = b[0] - a[0];
                    let dy = b[1] - a[1];
                    let chord = dx.hypot(dy);
                    require(chord > 1e-7, "Duplicate or zero-length cut segment")?;
                    include(&mut bounds, [a[0], a[1]]);
                    signed_area += (a[0] * b[1] - a[1] * b[0]) * 0.5;
                    bulges.push(a[2]);
                    if a[2].abs() < 1e-12 {
                        circular = false;
                        continue;
                    }
                    // Recover the exact arc independently from the imported bulge.
                    let theta = 4. * a[2].atan();
                    let factor = (1. - a[2] * a[2]) / (4. * a[2]);
                    let center = [
                        (a[0] + b[0]) * 0.5 - dy * factor,
                        (a[1] + b[1]) * 0.5 + dx * factor,
                    ];
                    let radius = chord * (1. + a[2] * a[2]) / (4. * a[2].abs());
                    signed_area += radius * radius * (theta - theta.sin()) * 0.5;
                    sweep_sum += theta;
                    if let Some((old, r)) = circle {
                        circular &=
                            near(center[0], old[0]) && near(center[1], old[1]) && near(radius, r);
                    } else {
                        circle = Some((center, radius));
                    }
                    let start = (a[1] - center[1]).atan2(a[0] - center[0]);
                    for cardinal in 0..4 {
                        let angle = cardinal as f64 * std::f64::consts::FRAC_PI_2;
                        let distance = if theta > 0. {
                            angle - start
                        } else {
                            start - angle
                        }
                        .rem_euclid(std::f64::consts::TAU);
                        if distance <= theta.abs() + 1e-10 {
                            include(
                                &mut bounds,
                                [
                                    center[0] + radius * angle.cos(),
                                    center[1] + radius * angle.sin(),
                                ],
                            );
                        }
                    }
                }
                let is_outer = entity.common.layer == layers.cut;
                let is_inner = entity.common.layer == format!("{}_INNER", layers.cut);
                require(
                    (is_outer && signed_area > 0.) || (is_inner && signed_area < 0.),
                    "Wrong cut layer or contour winding",
                )?;
                outer += usize::from(is_outer);
                inner += usize::from(is_inner);
                area_mm2 += signed_area;
                if circular
                    && (sweep_sum.abs() - std::f64::consts::TAU).abs() < 1e-10
                    && let Some((center, radius)) = circle
                {
                    require(
                        is_inner,
                        "Unexpected circular outer boundary in this panel review",
                    )?;
                    circles.push((center, radius));
                }
                loops.push(json!({"layer":entity.common.layer,"closed":true,"vertices":poly.vertices.len(),"signed_area_mm2":signed_area,"bulges":bulges}));
            }
            EntityType::ModelPoint(point) => {
                require(
                    entity.common.layer == layers.construction,
                    "Center point on cut/incorrect layer",
                )?;
                require(near(point.location.z, 0.), "Non-planar center point")?;
                centers.push([
                    point.location.x * mm_per_unit,
                    point.location.y * mm_per_unit,
                ]);
            }
            EntityType::Text(text) => {
                require(
                    entity.common.layer == layers.annotation,
                    "Text on cut/incorrect layer",
                )?;
                require(!text.value.is_empty(), "Empty part label")?;
                labels.push(text.value.clone());
            }
            _ => return Err("Unexpected entity family in planar DXF".into()),
        }
    }
    require(
        centers.len() == circles.len(),
        "Circular opening centers missing or duplicated",
    )?;
    for (center, _) in &circles {
        require(
            centers
                .iter()
                .filter(|p| near(p[0], center[0]) && near(p[1], center[1]))
                .count()
                == 1,
            "Center point does not match exactly one circular opening",
        )?;
    }
    require(labels.len() == 1, "Expected one non-cut part label")?;
    let expected = stored
        .artifact
        .planar
        .as_ref()
        .ok_or("Receipt has no planar evidence")?;
    require(
        (outer, inner) == (expected.outer_loops, expected.inner_loops),
        "Receipt loop counts mismatch",
    )?;
    require(
        near(area_mm2, expected.area_mm2),
        "Receipt analytic area mismatch",
    )?;
    require(
        bounds
            .iter()
            .flatten()
            .zip(expected.bounds_mm.iter().flatten())
            .all(|(a, b)| near(*a, *b)),
        "Receipt exact bounds mismatch",
    )?;
    Ok(
        json!({"filename":stored.artifact.output.filename,"bytes":bytes.len(),"sha256":hash.to_string(),"receipt_hash_match":true,"units":format!("{units:?}"),"mm_per_unit":mm_per_unit,"bounds_mm":bounds,"area_mm2":area_mm2,"outer_loops":outer,"inner_loops":inner,"loops":loops,"circular_openings_mm":circles,"centers_mm":centers,"labels":labels}),
    )
}

fn main() -> ReviewResult<()> {
    let arguments = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    require(
        arguments.len() == 3,
        "Usage: planar_dxf_file_review <source.multicad> <bundle-directory> <new-report.json>",
    )?;
    let source = project_file::read(
        &mut File::open(&arguments[0])?,
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    let source_hash = ContentHash::document(&source.project.document)?;
    let bundle = &arguments[1];
    let manifest_bytes = fs::read(bundle.join("multicad-export.json"))?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)?;
    require(manifest["schema"] == 1, "Unsupported receipt schema")?;
    let saved_hash: ContentHash = serde_json::from_value(manifest["source_hash"].clone())?;
    require(
        source_hash == saved_hash,
        "Saved source hash differs from UI receipt",
    )?;
    let set = read(
        &source.project.document,
        serde_json::from_value(manifest["set"].clone())?,
    )?;
    let files: Vec<WrittenFile> = serde_json::from_value(manifest["files"].clone())?;
    require(
        !files.is_empty() && files.len() <= 128,
        "Invalid receipt file count",
    )?;
    let mut names = BTreeSet::new();
    let mut reports = vec![];
    for file in &files {
        let name = &file.artifact.output.filename;
        require(
            Path::new(name).components().count() == 1
                && name.ends_with(".dxf")
                && names.insert(name),
            "Duplicate or unsafe DXF filename",
        )?;
        let recipe = set
            .recipes
            .iter()
            .find(|r| r.id == file.artifact.output.recipe)
            .ok_or("Unknown receipt recipe")?;
        let RecipeOutput::PlanarDxf { units, layers, .. } = &recipe.output else {
            return Err("Receipt recipe is not planar DXF".into());
        };
        let units = match units {
            multicad_core::exchange::LengthUnit::Millimeter => Units::Millimeters,
            multicad_core::exchange::LengthUnit::Centimeter => Units::Centimeters,
            multicad_core::exchange::LengthUnit::Meter => Units::Meters,
            multicad_core::exchange::LengthUnit::Inch => Units::Inches,
        };
        reports.push(inspect(&bundle.join(name), file, &units, layers)?);
    }
    let report = json!({"reader":"Rust dxf 0.6.2, developer-only","source":arguments[0],"source_hash":source_hash.to_string(),"bundle":bundle,"manifest_sha256":ContentHash::of(&manifest_bytes).to_string(),"actor":manifest["actor"],"files":reports});
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&arguments[2])?;
    output.write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "Independent reader verified {} actual DXF files; report {}",
        files.len(),
        arguments[2].display()
    );
    Ok(())
}
