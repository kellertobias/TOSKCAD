//! Developer-only native internal-ring source and manufacturing fixtures.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{self, length, *},
    project_file::{self, ContentHash, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{
    Compiler,
    export::{Exporter, mesh},
};
use serde_json::json;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "artifacts/internal-gear-review-20261104".into()),
    );
    std::fs::create_dir(&output)?;
    let output = std::fs::canonicalize(output)?;
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let mut cases = vec![];
    for (teeth, ring_first, name) in [
        (48., false, "24-48-ring-second"),
        (60., true, "60-24-ring-first"),
    ] {
        let mut session = Session::new(Document::new(name)?)?;
        let shared = GearParameters {
            module: length(1.),
            face_width: length(3.),
            fabrication_allowance: length(0.02),
            ..Default::default()
        };
        let ring = GearRequest::new(
            "Ring",
            GearParameters {
                family: GearFamily::InternalRing,
                teeth: Dimension::Literal {
                    value: teeth,
                    unit: "count".into(),
                },
                bore_diameter: length(0.),
                outer_diameter: Some(length(teeth + 8.)),
                ..shared.clone()
            },
        );
        let pinion = GearRequest::new("Pinion", shared);
        let mut req = GearPairRequest::new(
            name,
            if ring_first {
                [ring, pinion]
            } else {
                [pinion, ring]
            },
        );
        req.parameters.drive_angle = angle(13.);
        req.parameters.frame.origin = [length(10.), length(20.), length(3.)];
        let prepared = prepare_gear_pair(&session, 0, actor(), &req)?;
        compiler.compile(prepared.prepared.document(), 1)?;
        session.commit(prepared.prepared)?;
        let project = output.join(format!("{name}.tcd"));
        project_file::write(
            &ProjectFile::new(session.document().clone())?,
            &mut std::fs::File::create(&project)?,
            Default::default(),
        )?;
        let reopened = project_file::read(
            &mut std::fs::File::open(&project)?,
            Default::default(),
            &project_file::supported_capabilities(),
        )?;
        if reopened.project.document != *session.document() {
            return Err("Canonical source changed during reopen".into());
        }
        let native = compiler.compile(&reopened.project.document, session.revision())?;
        let resolved = model::resolve_gear_pair_parameters(
            session.document(),
            session.parameters(),
            &req.parameters,
        )?;
        let mut parts = vec![];
        for (i, gear) in req.gears.iter().enumerate() {
            let source = read_gear(session.document(), gear.feature)?;
            let r = source.parameters.resolve(session.parameters())?;
            let solid = native.solid(gear.feature).ok_or("Missing native member")?;
            let role = if r.family == GearFamily::InternalRing {
                "ring"
            } else {
                "pinion"
            };
            let path = output.join(format!("{name}-{role}.step"));
            solid.write_step(&path)?;
            let back = Solid::read_step(&path, GeometryPolicy::default())?;
            let volume = solid.volume_mm3()?;
            let area = solid.faces()?.iter().map(|f| f.area_mm2).sum::<f64>();
            let delta = (back.volume_mm3()? - volume).abs();
            let budget = 1e-7 * volume + 2e-7 * area;
            if solid.solid_count()? != 1 || back.solid_count()? != 1 || delta > budget {
                return Err("STEP native readback failed".into());
            }
            let pose = model::resolve_instance_pose(
                session.document(),
                req.outputs.instances[i],
                session.parameters(),
            )?;
            let bounds = solid.bounds()?;
            parts.push(json!({"role":role,"feature":gear.feature,"body":gear.body,"source":source,"member":i,"pose":pose,
                "volume_mm3":volume,"area_mm2":area,"volume_budget_mm3":budget,"step_volume_delta_mm3":delta,"step":path,
                "bounds_mm":[bounds.min,bounds.max],"construction":{"teeth":r.teeth,"pitch_radius_mm":r.pitch_radius_mm,"base_radius_mm":r.base_radius_mm,"tip_radius_mm":r.tip_radius_mm,"root_radius_mm":r.root_radius_mm,"root_fillet_radius_mm":r.root_fillet_radius_mm,"outer_radius_mm":r.outer_radius_mm,"physical_tooth_thickness_mm":r.physical_tooth_thickness_mm,"face_width_mm":r.face_width_mm}}));
        }
        let request = PrintRequest {
            source: PrintSource::Parts {
                parts: vec![req.placement.instance],
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
            return Err(format!("Print blocked {:?}", preview.blocked).into());
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
                return Err("Raw print receipt mismatch".into());
            }
        }
        cases.push(json!({"name":name,"project":project,"pair":req.pair,"occurrence":req.placement.instance,"center_distance_mm":resolved.center_distance_mm,"phases_rad":resolved.phases_rad,
            "native_pair_check":native.gear_pair_checks()[&req.pair],"parts":parts,"print_preview":preview,"print":receipt}));
    }
    let mut files = vec![];
    for entry in std::fs::read_dir(&output)? {
        let path = entry?.path();
        if path.is_file() {
            let bytes = std::fs::read(&path)?;
            files.push(json!({"path":path,"bytes":bytes.len(),"sha256":ContentHash::of(&bytes)}));
        }
    }
    let report = json!({"format":"multicad.internal-gear-review/1","scope":"Bounded native 20-degree unshifted internal ring/pinion geometry, nine sampled phases, editable source and manufacturing readback; no continuous motion, cutter/strength/fit certification or OS interaction", "sources":[model::GEAR_DIMENSIONS_SOURCE_URL,"https://khkgears.us/media/beqhhrye/gear-solutions_khk_internal-ring-gears-design-and-considerations.pdf"],"native_fingerprint":multicad_kernel::thread_prototypes::source_fingerprint(),"cases":cases,"files":files});
    let path = output.join("report.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", path.display());
    Ok(())
}
