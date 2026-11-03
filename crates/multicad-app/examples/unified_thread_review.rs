//! Repeatable CAD-212 native artifacts; creates no application window.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintRequest, PrintSource},
    model::{
        self, BodyInput, PrimitiveParameters, PrimitiveRequest, ThreadFamily, ThreadFit,
        ThreadMode, ThreadParameters, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, Solid};
use multicad_model::{Compiler, export::Exporter};
use std::{error::Error, path::PathBuf};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

/// Co-located 1/4-inch male and nut for native mating verification or UI review.
/// The returned requests identify their canonical features and bodies.
pub fn unified_fixture(
    family: ThreadFamily,
) -> Result<(Session, ThreadRequest, ThreadRequest), Box<dyn Error>> {
    let tpi = match family {
        ThreadFamily::Unc => 20.,
        ThreadFamily::Unf => 28.,
        _ => return Err("This fixture supports UNC and UNF only".into()),
    };
    let pitch = 25.4 / tpi;
    let mut s = Session::new(Document::new("Unified sourced native mating pair")?)?;
    let parameters = ThreadParameters {
        family,
        fit: ThreadFit::Unified2A,
        major_diameter: length(6.35),
        pitch: length(pitch),
        length: length(4. * pitch),
        ..Default::default()
    };
    let male = ThreadRequest::new("1/4 inch Unified 2A screw", parameters.clone());
    let prepared = model::prepare_thread(&s, s.revision(), actor(), &male)?;
    s.commit(prepared.prepared)?;
    let stock = PrimitiveRequest::new(
        "Unified nut blank",
        PrimitiveParameters::Cylinder {
            radius: length(5.),
            height: length(4. * pitch),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, s.revision(), actor(), &stock)?;
    s.commit(prepared.prepared)?;
    let female = ThreadRequest::receiving(
        "1/4 inch Unified 2B nut",
        ThreadMode::CutInternal,
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        ThreadParameters {
            fit: ThreadFit::Unified2B,
            ..parameters
        },
    )?;
    let prepared = model::prepare_thread(&s, s.revision(), actor(), &female)?;
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(prepared.prepared.document(), s.revision() + 1)?;
    s.commit(prepared.prepared)?;
    Ok((s, male, female))
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/unified-thread-review".into());
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // Existing review evidence must never be replaced by a rerun.
    std::fs::create_dir(&output)?;
    let output = output.canonicalize()?;
    let mut reports = vec![];
    for (name, family) in [("unc", ThreadFamily::Unc), ("unf", ThreadFamily::Unf)] {
        let (session, male, female) = unified_fixture(family)?;
        let native = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(session.document(), session.revision())?;
        let screw = native.solid(male.feature).ok_or("Missing screw")?;
        let nut = native.solid(female.feature).ok_or("Missing nut")?;
        if screw
            .boolean(nut, BooleanOperation::Intersect, Default::default())?
            .is_some()
        {
            return Err("Native mating pair contains overlapping material".into());
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
        let reopened = Session::new(reopened.document)?;
        let rebuilt = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(reopened.document(), reopened.revision())?;
        let mut parts = vec![];
        for (role, request) in [("screw", &male), ("nut", &female)] {
            let source = model::read_thread(session.document(), request.feature)?;
            let restored = model::read_thread(reopened.document(), request.feature)?;
            assert_eq!(source, restored);
            let resolved = source.parameters.resolve(
                &multicad_core::parameters::evaluate(session.document())?,
                source.mode,
            )?;
            let fit = resolved.fit.as_ref().ok_or("Missing source fit")?;
            let solid = native.solid(request.feature).ok_or("Missing native part")?;
            let volume = solid.volume_mm3()?;
            assert!(
                (rebuilt
                    .solid(request.feature)
                    .ok_or("Missing reopened part")?
                    .volume_mm3()?
                    - volume)
                    .abs()
                    < 1e-3
            );
            let step = output.join(format!("{name}-{role}.step"));
            solid.write_step(&step)?;
            let imported = Solid::read_step(&step, GeometryPolicy::default())?;
            assert!((imported.volume_mm3()? - volume).abs() < 1e-3);
            let print = PrintRequest {
                source: PrintSource::Parts {
                    parts: vec![request.body],
                },
                aids: AidPolicy::IncludeAll,
                quality: MeshQuality::default(),
                stl_per_part: true,
            };
            let exporter = Exporter::default();
            let preview = exporter.print_preview(
                session.document(),
                session.revision(),
                &print,
                &actor(),
                None,
            )?;
            assert!(preview.can_write());
            let receipt = exporter.write_print(
                session.document(),
                session.revision(),
                &print,
                &preview.key,
                &output.join(format!("{name}-{role}.3mf")),
                &actor(),
                None,
            )?;
            for written in &receipt.files {
                assert!(written.triangles > 100);
                assert!((written.mesh_volume_mm3 - volume).abs() < volume * 0.02);
            }
            parts.push(serde_json::json!({
                "role": role, "body": request.body, "feature": request.feature,
                "definition": source, "designation": fit.designation, "source": fit.source,
                "source_url": fit.source_url, "nominal_pitch_diameter_mm": resolved.nominal_pitch_diameter_mm,
                "actual_pitch_diameter_mm": resolved.actual_pitch_diameter_mm,
                "resolved_fit": fit,
                "volume_mm3": volume, "step": step, "print_receipt": receipt,
            }));
        }
        reports.push(
            serde_json::json!({"family":family,"project":project_path,"parts":parts,
            "native_mating_material_intersection":false,"independent_session_reopen":true}),
        );
    }
    let presets = model::unified_thread_presets();
    let report = serde_json::json!({"scope":"Bounded NBS H28 (1969) Unified UN flat-root engineering targets; no gage, physical-fit or current ASME compliance claim",
        "supported_presets":presets,"native_cases":reports,"os_window_interaction":false});
    std::fs::write(
        output.join("native-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", output.display());
    Ok(())
}
