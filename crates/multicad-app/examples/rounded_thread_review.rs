//! Developer-only rounded-thread artifacts from canonical source and native solids.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{
        self, BodyInput, PrimitiveParameters, PrimitiveRequest, ThreadFamily, ThreadFit,
        ThreadMode, ThreadParameters, ThreadRequest, WhitworthThreadPreset, length,
    },
    project_file::{self, ContentHash, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, Solid};
use multicad_model::{
    Compiler,
    export::{Exporter, mesh},
};
use serde::Serialize;
use std::{error::Error, io::Write, path::PathBuf};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn fixture(preset: &WhitworthThreadPreset) -> Result<(Session, ThreadRequest, ThreadRequest)> {
    let mut session = Session::new(Document::new(format!(
        "{} rounded mating pair",
        preset.designation
    ))?)?;
    let pipe = preset.family == ThreadFamily::Bspp;
    let parameters = ThreadParameters {
        family: preset.family,
        fit: if pipe {
            ThreadFit::BsppClassBExternal
        } else {
            ThreadFit::WhitworthMediumExternal
        },
        major_diameter: length(preset.diameter_mm),
        pitch: length(preset.pitch_mm),
        length: length(3. * preset.pitch_mm),
        ..Default::default()
    };
    let male = ThreadRequest::new("Rounded external", parameters.clone());
    let prepared = model::prepare_thread(&session, session.revision(), actor(), &male)?;
    session.commit(prepared.prepared)?;
    let stock = PrimitiveRequest::new(
        "Rounded receiving blank",
        PrimitiveParameters::Cylinder {
            radius: length(preset.diameter_mm / 2. + 2.),
            height: length(3. * preset.pitch_mm),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &stock)?;
    session.commit(prepared.prepared)?;
    let female = ThreadRequest::receiving(
        "Rounded internal",
        ThreadMode::CutInternal,
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        ThreadParameters {
            fit: if pipe {
                ThreadFit::BsppInternal
            } else {
                ThreadFit::WhitworthMediumInternal
            },
            ..parameters
        },
    )?;
    let prepared = model::prepare_thread(&session, session.revision(), actor(), &female)?;
    session.commit(prepared.prepared)?;
    Ok((session, male, female))
}
#[derive(Serialize)]
struct Metrology {
    volume_mm3: f64,
    surface_area_mm2: f64,
    bounds_mm: [[f64; 3]; 2],
    volume_budget_mm3: f64,
}
fn measure(solid: &Solid, radius: f64, height: f64) -> Result<Metrology> {
    if solid.solid_count()? != 1 {
        return Err("Rounded artifact must contain exactly one solid".into());
    }
    let volume = solid.volume_mm3()?;
    let area = solid.faces()?.iter().map(|face| face.area_mm2).sum::<f64>();
    let bounds = solid.bounds()?;
    let epsilon = GeometryPolicy::default().linear_tolerance_mm;
    if !area.is_finite() || area <= 0. {
        return Err("Invalid native surface area".into());
    }
    let expected = [[-radius, -radius, 0.], [radius, radius, height]];
    for (actual, expected) in [bounds.min, bounds.max].into_iter().zip(expected) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            if (actual - expected).abs() > 16. * epsilon {
                return Err(
                    format!("Native extent {actual} differs from authored {expected}").into(),
                );
            }
        }
    }
    Ok(Metrology {
        volume_mm3: volume,
        surface_area_mm2: area,
        bounds_mm: [bounds.min, bounds.max],
        volume_budget_mm3: epsilon * area + 1e-7 * volume.abs(),
    })
}
fn compare(a: &Metrology, b: &Metrology, label: &str) -> Result<f64> {
    let delta = (a.volume_mm3 - b.volume_mm3).abs();
    if delta > a.volume_budget_mm3 + b.volume_budget_mm3 {
        return Err(format!(
            "{label} volume delta {delta} exceeds policy budget {}",
            a.volume_budget_mm3 + b.volume_budget_mm3
        )
        .into());
    }
    Ok(delta)
}
fn main() -> Result<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/rounded-thread-review".into());
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(&output)?;
    let output = output.canonicalize()?;
    let presets = model::whitworth_thread_presets();
    if presets.len() != 6 {
        return Err("This review requires all six sourced rounded presets".into());
    }
    let mut cases = vec![];
    let exporter = Exporter {
        compiler: compiler(),
    };
    for (index, preset) in presets.iter().enumerate() {
        let family = format!("{:?}", preset.family).to_lowercase();
        let name = format!("{family}-{index}");
        let (session, male, female) = fixture(preset)?;
        let native = compiler().compile(session.document(), session.revision())?;
        let external = native.solid(male.feature).ok_or("Missing external solid")?;
        let internal = native
            .solid(female.feature)
            .ok_or("Missing internal solid")?;
        if external
            .boolean(
                internal,
                BooleanOperation::Intersect,
                GeometryPolicy::default(),
            )?
            .is_some()
        {
            return Err(format!("{} has overlapping mating material", preset.designation).into());
        }
        let project_path = output.join(format!("{name}-mating-pair.tcd"));
        project_file::write(
            &ProjectFile::new(session.document().clone())?,
            &mut std::fs::File::create_new(&project_path)?,
            Default::default(),
        )?;
        let reopened = project_file::read(
            &mut std::fs::File::open(&project_path)?,
            Default::default(),
            &project_file::supported_capabilities(),
        )?
        .project;
        if reopened.document != *session.document() {
            return Err("Saved canonical source changed on reopen".into());
        }
        let reopened = Session::new(reopened.document)?;
        if reopened.revision() != 0 {
            return Err("Reopened artifact must own an independent session".into());
        }
        let rebuilt = compiler().compile(reopened.document(), reopened.revision())?;
        if rebuilt
            .solid(male.feature)
            .ok_or("Missing reopened external")?
            .boolean(
                rebuilt
                    .solid(female.feature)
                    .ok_or("Missing reopened internal")?,
                BooleanOperation::Intersect,
                GeometryPolicy::default(),
            )?
            .is_some()
        {
            return Err("Reopened pair has overlapping mating material".into());
        }
        let mut parts = vec![];
        for (role, request) in [("external", &male), ("internal", &female)] {
            let source = model::read_thread(session.document(), request.feature)?;
            let restored = model::read_thread(reopened.document(), request.feature)?;
            if source != restored || source.output.body != request.body {
                return Err("Reopened thread source or identity changed".into());
            }
            let resolved = source
                .parameters
                .resolve(session.parameters(), source.mode)?;
            let fit = resolved.fit.as_ref().ok_or("Missing sourced fit")?;
            let solid = native
                .solid(request.feature)
                .ok_or("Missing native thread")?;
            let height = 3. * preset.pitch_mm;
            let radius = if role == "external" {
                resolved.major_radius_mm
            } else {
                preset.diameter_mm / 2. + 2.
            };
            let measured = measure(solid, radius, height)?;
            let reopened_measurement = measure(
                rebuilt
                    .solid(request.feature)
                    .ok_or("Missing reopened thread")?,
                radius,
                height,
            )?;
            let reopen_delta = compare(&measured, &reopened_measurement, "Reopen")?;
            let step_path = output.join(format!("{name}-{role}.step"));
            if step_path.exists() {
                return Err("Refuse overwriting a STEP artifact".into());
            }
            solid.write_step(&step_path)?;
            let step = Solid::read_step(&step_path, GeometryPolicy::default())?;
            let step_measurement = measure(&step, radius, height)?;
            let step_delta = compare(&measured, &step_measurement, "STEP")?;
            let print = PrintRequest {
                source: PrintSource::Parts {
                    parts: vec![request.body],
                },
                aids: AidPolicy::IncludeAll,
                quality: MeshQuality {
                    chord_deviation_mm: 0.005,
                    angular_deviation_deg: 5.,
                },
                stl_per_part: true,
            };
            let preview = exporter.print_preview(
                session.document(),
                session.revision(),
                &print,
                &actor(),
                None,
            )?;
            if !preview.can_write() {
                return Err(format!("Print package refused: {:?}", preview.blocked).into());
            }
            let stem = format!("{name}-{role}");
            for file in &preview.files {
                if output.join(file.filename(&stem)).exists() {
                    return Err("Refuse overwriting a print artifact".into());
                }
            }
            let receipt = exporter.write_print(
                session.document(),
                session.revision(),
                &print,
                &preview.key,
                &output.join(format!("{stem}.3mf")),
                &actor(),
                None,
            )?;
            if receipt.files.len() != 2 {
                return Err("Expected one actual STL and one 3MF for each thread".into());
            }
            let mut mesh_checks = vec![];
            for file in &receipt.files {
                let bytes = std::fs::read(&file.path)?;
                if bytes.len() as u64 != file.bytes || ContentHash::of(&bytes) != file.content_hash
                {
                    return Err("Mesh receipt hash/size mismatch".into());
                }
                let read = match file.kind {
                    PrintFileKind::ThreeMf => mesh::read_three_mf(&bytes)?,
                    PrintFileKind::Stl => mesh::read_stl(&bytes)?,
                };
                // Binary STL stores facets, without object/build records.
                let expected_counts = match file.kind {
                    PrintFileKind::ThreeMf => (1, 1),
                    PrintFileKind::Stl => (0, 0),
                };
                if read.unit_scale != 1.
                    || (read.objects, read.items) != expected_counts
                    || read.triangles != file.triangles
                    || read.triangles == 0
                {
                    return Err("Invalid actual mesh units, object/build count or triangles".into());
                }
                if !read
                    .world
                    .iter()
                    .flat_map(|(a, b, c)| [a, b, c])
                    .flatten()
                    .all(|n| n.is_finite())
                {
                    return Err("Nonfinite exported mesh coordinate".into());
                }
                let mesh_bounds = read
                    .bounds_mm()
                    .ok_or("Exported mesh has no finite bounds")?;
                let coordinate_scale = measured
                    .bounds_mm
                    .into_iter()
                    .flatten()
                    .map(f64::abs)
                    .fold(1., f64::max);
                // Chord error plus conservative f32 STL serialization error
                // and the native support-query enclosure margin. 3MF uses
                // the same bound even when its coordinates retain more digits.
                let bounds_budget = print.quality.chord_deviation_mm
                    + 2. * f64::from(f32::EPSILON) * coordinate_scale
                    + 16. * GeometryPolicy::default().linear_tolerance_mm;
                for (actual, expected) in mesh_bounds
                    .into_iter()
                    .flatten()
                    .zip(measured.bounds_mm.into_iter().flatten())
                {
                    if (actual - expected).abs() > bounds_budget {
                        return Err(format!("Mesh bounds {actual} differ from native {expected} beyond chord/serialization budget {bounds_budget}").into());
                    }
                }
                let volume = read.volume_mm3();
                let budget = print.quality.chord_deviation_mm * measured.surface_area_mm2
                    + 1e-7 * measured.volume_mm3.abs();
                if (volume - measured.volume_mm3).abs() > budget
                    || (volume - file.mesh_volume_mm3).abs() > 1e-6
                {
                    return Err("Mesh volume exceeds chord/surface-area budget or disagrees with actual receipt".into());
                }
                mesh_checks.push(serde_json::json!({"path":file.path,"native_volume_difference_mm3":volume-measured.volume_mm3,"budget_mm3":budget,"actual_mesh_volume_mm3":volume,"bounds_mm":mesh_bounds,"bounds_budget_mm":bounds_budget}));
            }
            let topology = solid.topology_snapshot(request.feature, session.revision())?;
            if topology
                .elements
                .iter()
                .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
                .any(|e| e.origins.is_empty())
            {
                return Err("Incomplete native face ancestry".into());
            }
            parts.push(serde_json::json!({"role":role,"feature":request.feature,"body":request.body,"definition":source,
                "designation":preset.designation,"source":preset.source,"source_url":preset.source_url,"profile_source_url":preset.profile_source_url,
                "pitch_mm":resolved.pitch_mm,"lead_mm":resolved.lead_mm,"depth_mm":resolved.profile.depth_mm,"rounding_radius_mm":resolved.profile.rounding_radius_mm,
                "nominal_pitch_diameter_mm":resolved.nominal_pitch_diameter_mm,"actual_pitch_diameter_mm":resolved.actual_pitch_diameter_mm,
                "actual_major_diameter_mm":2.*resolved.major_radius_mm,"actual_minor_diameter_mm":2.*(resolved.major_radius_mm-resolved.profile.depth_mm),"resolved_fit":fit,
                "native":measured,"reopened":reopened_measurement,"reopen_volume_difference_mm3":reopen_delta,"step":step_path,"step_metrology":step_measurement,"step_volume_difference_mm3":step_delta,
                "print_receipt":receipt,"mesh_checks":mesh_checks}));
        }
        let external_radius = male
            .parameters
            .resolve(session.parameters(), ThreadMode::NewExternal)?
            .major_radius_mm;
        let internal_radius = female
            .parameters
            .resolve(session.parameters(), ThreadMode::CutInternal)?
            .major_radius_mm;
        cases.push(serde_json::json!({"family":preset.family,"designation":preset.designation,"project":project_path,"parts":parts,
            "native_mating_material_intersection":false,"reopened_mating_material_intersection":false,"independent_session_reopen":true,
            "radial_profile_target_separation_mm":internal_radius-external_radius,"clearance_interpretation":"Radial separation of same-phase full-form targets; no minimum-distance, physical-fit or gage certification"}));
    }
    for (extension, expected) in [("tcd", 6), ("step", 12), ("stl", 12), ("3mf", 12)] {
        let count = std::fs::read_dir(&output)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == extension)
            })
            .count();
        if count != expected {
            return Err(format!("Expected {expected} {extension} artifacts, found {count}").into());
        }
    }
    let report = serde_json::json!({"scope":"Six sourced true rounded BSW/BSF/BSPP engineering targets; no current-standard, pressure-seal, physical-fit or calibrated-gage claim",
        "supported_presets":presets,"native_cases":cases,"os_window_interaction":false,
        "metrology":"Native/STEP/reopen volume budget per solid: native linear epsilon times actual face area plus unchanged 1e-7 maximum integration-relative guard; comparisons use the sum. Mesh volume budget: chord deviation times actual face area plus numeric relative term. Mesh bounds budget: chord deviation plus twice f32 epsilon times coordinate scale plus 16 native linear epsilons."});
    let mut file = std::fs::File::create_new(output.join("native-report.json"))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    println!("{}", output.display());
    Ok(())
}
