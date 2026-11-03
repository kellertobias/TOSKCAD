//! Developer-only native R/Rc manufacturing review artifacts.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{
        self, BodyInput, PipeGaugeConvention, PipeThreadForm, PipeThreadIntent, PipeThreadSize,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, ThreadMode, ThreadRequest, length,
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
fn pair(size: PipeThreadSize) -> (Session, ThreadRequest, ThreadRequest) {
    let mut s = Session::new(Document::new("R/Rc gauge-aligned pair").unwrap()).unwrap();
    let intent = |form| PipeThreadIntent {
        form,
        size,
        gauge: PipeGaugeConvention::Iso7Nominal1994,
    };
    let mut external = intent(PipeThreadForm::R).parameters();
    external.radial_allowance = length(0.005);
    external.length = length(intent(PipeThreadForm::R).default_length_mm() + 0.005 * 32.);
    let male = ThreadRequest::new("R external", external);
    let p = model::prepare_thread(&s, s.revision(), actor(), &male).unwrap();
    s.commit(p.prepared).unwrap();
    let mut internal = intent(PipeThreadForm::Rc).parameters();
    internal.radial_allowance = length(0.005);
    // Opposing axes: align gauge planes and the same helical phase, rather
    // than merely intersecting two coincident parallel-thread fixtures.
    let origin =
        intent(PipeThreadForm::R).gauge_offset_mm() + intent(PipeThreadForm::Rc).gauge_offset_mm();
    let phi = std::f64::consts::TAU * origin / intent(PipeThreadForm::R).preset().pitch_mm;
    internal.frame = PrimitiveFrame {
        origin: [length(0.), length(0.), length(origin)],
        u: [phi.cos(), phi.sin(), 0.],
        v: [phi.sin(), -phi.cos(), 0.],
    };
    let blank = PrimitiveRequest::new(
        "Rc fitting blank",
        PrimitiveParameters::Cylinder {
            radius: length(intent(PipeThreadForm::Rc).base_major_mm() / 2. + 2.),
            height: length(intent(PipeThreadForm::Rc).default_length_mm()),
            frame: internal.frame.clone(),
        },
    );
    let p = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(p.prepared).unwrap();
    let female = ThreadRequest::receiving(
        "Rc internal",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        internal,
    )
    .unwrap();
    let p = model::prepare_thread(&s, s.revision(), actor(), &female).unwrap();
    s.commit(p.prepared).unwrap();
    (s, male, female)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use multicad_core::project_file::ContentHash;
    use serde_json::json;
    let output = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "artifacts/pipe-thread-review-20261103".into()),
    );
    std::fs::create_dir(&output)?;
    let output = std::fs::canonicalize(output)?;
    let mut cases = vec![];
    for size in [PipeThreadSize::OneEighth, PipeThreadSize::OneQuarter] {
        let (session, male, female) = pair(size);
        let name = format!("{size:?}");
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
        let a = native.solid(male.feature).ok_or("Missing R")?;
        let b = native.solid(female.feature).ok_or("Missing Rc")?;
        if a.boolean(b, BooleanOperation::Intersect, Default::default())?
            .is_some()
        {
            return Err("Mating pair material overlap".into());
        }
        let mut parts = vec![];
        for (role, request) in [("R", &male), ("Rc", &female)] {
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
                return Err("Native STEP/reopen metrology failed".into());
            }
            let bounds = solid.bounds()?;
            parts.push(json!({"role":role,"feature":request.feature,"body":request.body,"pipe":resolved.pipe,"source":source,"volume_mm3":volume,"area_mm2":area,"bounds_mm":[bounds.min,bounds.max],"step_volume_delta_mm3":step_delta,"reopen_volume_delta_mm3":reopen_delta,"volume_budget_mm3":budget}));
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
        cases.push(json!({"size":size,"project":project,"mating_material_intersection_empty":true,"parts":parts,"print":receipt}));
    }
    let mut files = vec![];
    for entry in std::fs::read_dir(&output)? {
        let path = entry?.path();
        if path.is_file() {
            let bytes = std::fs::read(&path)?;
            files.push(json!({"path":path,"bytes":bytes.len(),"sha256":ContentHash::of(&bytes)}));
        }
    }
    let report = json!({"format":"multicad.pipe-thread-review/1","scope":"Native BRep, independent source reopen, STEP and raw print exports; no OS interaction, physical fit or pressure-seal certification","source":model::BSPT_THREAD_SOURCE,"native_fingerprint":multicad_kernel::thread_prototypes::source_fingerprint(),"cases":cases,"files":files});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", output.join("report.json").display());
    Ok(())
}
