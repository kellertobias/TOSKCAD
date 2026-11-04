//! Developer-only native sourced round-thread manufacturing review artifacts.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{
        self, BodyInput, MetricFitPreset, PrimitiveParameters, PrimitiveRequest, ThreadHand,
        ThreadMode, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, Solid};
use multicad_model::{
    Compiler,
    export::{Exporter, mesh},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn pair(row: &MetricFitPreset, hand: ThreadHand) -> (Session, ThreadRequest, ThreadRequest) {
    let mut s = Session::new(Document::new("Metric fit mating geometry").unwrap()).unwrap();
    let mut p = model::ThreadParameters {
        major_diameter: length(row.diameter_mm),
        pitch: length(row.pitch_mm),
        fit: model::ThreadFit::Metric6gExternal,
        end_treatments: Some(model::ThreadEndTreatments {
            start: model::ThreadStartTreatment::Chamfer45 {
                radial_depth: length(0.8 * row.pitch_mm),
            },
            end: model::ThreadEndTreatment::CylindricalRelief {
                length: length(0.6 * row.pitch_mm),
                radial_depth: length(0.1 * row.pitch_mm),
            },
        }),
        ..Default::default()
    };
    p.hand = hand;
    p.length = length(1.1 * row.diameter_mm);
    p.radial_allowance = length(0.002);
    p.axial_allowance = length(0.002);
    let male = ThreadRequest::new("6g external", p.clone());
    let prepared = model::prepare_thread(&s, 0, actor(), &male).unwrap();
    s.commit(prepared.prepared).unwrap();
    let blank = PrimitiveRequest::new(
        "Metric receiver blank",
        PrimitiveParameters::Cylinder {
            radius: length(row.diameter_mm / 2. + 2.5),
            height: p.length.clone(),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(prepared.prepared).unwrap();
    p.fit = model::ThreadFit::Metric6HInternal;
    let female = ThreadRequest::receiving(
        "6H internal",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        p,
    )
    .unwrap();
    let prepared = model::prepare_thread(&s, s.revision(), actor(), &female).unwrap();
    s.commit(prepared.prepared).unwrap();
    (s, male, female)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use multicad_core::project_file::ContentHash;
    use serde_json::json;
    let output = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "artifacts/metric-fit-review-20261104".into()),
    );
    std::fs::create_dir(&output)?;
    let output = std::fs::canonicalize(output)?;
    let mut cases = vec![];
    let rows = model::metric_fit_presets();
    let cases_to_build = [
        (&rows[0], ThreadHand::Right, "m3-rh-treated"),
        (&rows[0], ThreadHand::Left, "m3-lh-treated"),
        (&rows[1], ThreadHand::Right, "m6-rh-treated"),
        (&rows[1], ThreadHand::Left, "m6-lh-treated"),
    ];
    for (row, hand, name) in cases_to_build {
        let (session, male, female) = pair(row, hand);
        let project = output.join(format!("{name}.tcd"));
        project_file::write(
            &ProjectFile::new(session.document().clone())?,
            &mut std::fs::File::create(&project)?,
            Default::default(),
        )?;
        let read = project_file::read(
            &mut std::fs::File::open(&project)?,
            Default::default(),
            &project_file::supported_capabilities(),
        )?;
        let reopened = Session::new(read.project.document)?;
        let native = compiler().compile(session.document(), session.revision())?;
        let rebuilt = compiler().compile(reopened.document(), reopened.revision())?;
        let a = native
            .solid(male.feature)
            .ok_or("Missing metric fit external")?;
        let b = native
            .solid(female.feature)
            .ok_or("Missing metric fit internal")?;
        if a.boolean(b, BooleanOperation::Intersect, Default::default())?
            .is_some()
        {
            return Err("Mating pair material overlap".into());
        }
        let mut parts = vec![];
        for (role, request) in [("external", &male), ("internal", &female)] {
            let source = model::read_thread(session.document(), request.feature)?;
            if source != model::read_thread(reopened.document(), request.feature)? {
                return Err("Independent source reopen changed".into());
            }
            let resolved = source
                .parameters
                .resolve(session.parameters(), source.mode)?;
            let solid = native.solid(request.feature).ok_or("Missing native part")?;
            let volume = solid.volume_mm3()?;
            let area = solid.faces()?.iter().map(|f| f.area_mm2).sum::<f64>();
            let step = output.join(format!("{name}-{role}.step"));
            solid.write_step(&step)?;
            let back = Solid::read_step(&step, Default::default())?;
            let step_delta = (back.volume_mm3()? - volume).abs();
            let reopen_delta = (rebuilt
                .solid(request.feature)
                .ok_or("Missing reopened part")?
                .volume_mm3()?
                - volume)
                .abs();
            let budget = multicad_kernel::GeometryPolicy::default().linear_tolerance_mm * area
                + 1e-7 * volume.abs();
            if solid.solid_count()? != 1
                || back.solid_count()? != 1
                || step_delta > budget
                || reopen_delta > budget
            {
                return Err(format!("Native STEP/reopen metrology failed {name}/{role}: native_solids={}, step_solids={}, volume={volume}, step_volume={}, step_delta={step_delta}, reopen_delta={reopen_delta}, budget={budget}",solid.solid_count()?,back.solid_count()?,back.volume_mm3()?).into());
            }
            let bounds = solid.bounds()?;
            parts.push(json!({"role":role,"feature":request.feature,"body":request.body,"metric_fit":resolved.metric_fit,"fit":resolved.fit,"end_treatments":resolved.end_treatments,"construction":{"major_radius_mm":resolved.major_radius_mm,"depth_mm":resolved.profile.depth_mm,"crest_width_mm":resolved.profile.crest_width_mm,"pitch_mm":resolved.pitch_mm,"lead_mm":resolved.lead_mm,"starts":resolved.starts,"hand":resolved.hand},"source":source,"volume_mm3":volume,"area_mm2":area,"bounds_mm":[bounds.min,bounds.max],"step_volume_delta_mm3":step_delta,"reopen_volume_delta_mm3":reopen_delta,"volume_budget_mm3":budget}));
        }
        let request = PrintRequest {
            source: PrintSource::Parts {
                parts: vec![male.body, female.body],
            },
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality {
                chord_deviation_mm: 0.003,
                angular_deviation_deg: 5.,
            },
            stl_per_part: true,
        };
        let exporter = Exporter::default();
        let preview = exporter.print_preview(
            session.document(),
            session.revision(),
            &request,
            &actor(),
            None,
        )?;
        if !preview.can_write() {
            return Err(format!("Print blocked: {:?}", preview.blocked).into());
        }
        let receipt = exporter.write_print(
            session.document(),
            session.revision(),
            &request,
            &preview.key,
            &output.join(format!("{name}.3mf")),
            &actor(),
            None,
        )?;
        for file in &receipt.files {
            let bytes = std::fs::read(&file.path)?;
            let raw = match file.kind {
                PrintFileKind::ThreeMf => mesh::read_three_mf(&bytes)?,
                PrintFileKind::Stl => mesh::read_stl(&bytes)?,
            };
            if raw.triangles != file.triangles
                || (raw.volume_mm3() - file.mesh_volume_mm3).abs() > 1e-6
            {
                return Err("Raw print metrology mismatch".into());
            }
        }
        cases.push(json!({"designation":row.designation,"family":"iso_metric","hand":hand,"starts":1,"project":project,"mating_material_intersection_empty":true,"parts":parts,"print":receipt}));
    }
    let mut files = vec![];
    for entry in std::fs::read_dir(&output)? {
        let path = entry?.path();
        if path.is_file() {
            let bytes = std::fs::read(&path)?;
            files.push(json!({"path":path,"bytes":bytes.len(),"sha256":ContentHash::of(&bytes)}));
        }
    }
    let report = json!({"format":"multicad.metric-fit-review/1","scope":"Native BRep, independent source reopen, STEP and raw print exports; historical6g/6H dimensional targets and custom engineering end treatments; no OS interaction, gage, strength or physical-fit certification","sources":[model::METRIC_FIT_SOURCE],"native_fingerprint":multicad_kernel::thread_prototypes::source_fingerprint(),"cases":cases,"files":files});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", output.join("report.json").display());
    Ok(())
}
