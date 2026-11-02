//! CAD-225 representative native aids; no application windows are created.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_model::Compiler;
use std::path::PathBuf;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "artifacts/fdm-aid-review-20261102".into());
    std::fs::create_dir_all(&output)?;
    let mut reports = vec![];
    for kind in ["tab", "bridge", "support"] {
        let material = Entity::new(
            "Removable aid",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [242, 153, 55, 255],
                section_hatch: None,
            },
        );
        let mut session = Session::new(
            Document::new(format!("FDM {kind} review"))?
                .edited(&[Mutation::Insert(material.clone())])?,
        )?;
        let block = PrimitiveRequest::new(
            "Finished block",
            PrimitiveParameters::Box {
                size: [length(40.), length(20.), length(10.)],
                frame: PrimitiveFrame {
                    origin: [
                        length(0.),
                        length(0.),
                        length(if kind == "support" { 15. } else { 0. }),
                    ],
                    ..Default::default()
                },
            },
        );
        let p = prepare_primitive(&session, session.revision(), actor(), &block)?;
        session.commit(p.prepared)?;
        let other = if kind == "bridge" {
            let p = PrimitiveRequest::new(
                "Second finished block",
                PrimitiveParameters::Box {
                    size: [10.; 3].map(length),
                    frame: PrimitiveFrame {
                        origin: [length(60.), length(0.), length(0.)],
                        ..Default::default()
                    },
                },
            );
            let prepared = prepare_primitive(&session, session.revision(), actor(), &p)?;
            session.commit(prepared.prepared)?;
            Some(p.body)
        } else {
            None
        };
        let (shape, intent) = match kind {
            "tab" => (
                AidShape::Tab {
                    origin: [0., 10., 0.].map(length),
                    angle: angle(180.),
                    length: length(8.),
                    width: length(6.),
                    thickness: length(0.4),
                    overlap: length(2.),
                },
                "Flat pad overlapping a base edge; break away after printing",
            ),
            "bridge" => (
                AidShape::Bridge {
                    start: [39., 10., 0.2].map(length),
                    end: [61., 5., 0.2].map(length),
                    width: length(2.),
                    thickness: length(0.4),
                    other,
                },
                "Thin bar joining two surface anchors; cut the bar away after printing",
            ),
            _ => (
                AidShape::Support {
                    origin: [5., 5.].map(length),
                    angle: angle(0.),
                    length: length(30.),
                    width: length(10.),
                    base: length(0.),
                    gap: length(0.2),
                },
                "Vertical block below an underside, with a 0.2 mm removal gap",
            ),
        };
        let mut request = AidRequest::new(
            format!("Review {kind}"),
            AidDefinition {
                output: BodyOutput {
                    body: Default::default(),
                },
                target: block.body,
                shape,
                process: None,
            },
        );
        request.material = Some(material.id);
        let prepared = prepare_aid(&session, session.revision(), actor(), &request)?;
        Compiler::default().compile(prepared.prepared.document(), session.revision() + 1)?;
        session.commit(prepared.prepared)?;
        let native = Compiler::default().compile(session.document(), session.revision())?;
        let reviews = native.fdm_aids(session.document())?;
        let review = reviews
            .iter()
            .find(|review| review.part == request.body)
            .ok_or("Missing native aid review")?;
        assert!(
            review.contact_issues.is_empty(),
            "{kind}: {:?}",
            review.contact_issues
        );
        let aid = native.solid(request.body).ok_or("Missing aid solid")?;
        let target = native.solid(block.body).ok_or("Missing finished solid")?;
        assert!((target.volume_mm3()? - 8000.).abs() < 1e-6);
        let step = output.join(format!("{kind}.step"));
        aid.write_step(&step)?;
        let readback = multicad_kernel::Solid::read_step(&step, Default::default())?;
        assert!((readback.volume_mm3()? - aid.volume_mm3()?).abs() < 1e-4);
        let path = output.join(format!("{kind}.tcd"));
        project_file::write(
            &ProjectFile::new(session.document().clone())?,
            &mut std::fs::File::create(&path)?,
            Default::default(),
        )?;
        let reopened = project_file::read(
            &mut std::fs::File::open(&path)?,
            Default::default(),
            &project_file::supported_capabilities(),
        )?
        .project;
        let reopened = Compiler::default().compile(&reopened.document, 0)?;
        assert!(
            (reopened
                .solid(request.body)
                .ok_or("Missing reopened aid")?
                .volume_mm3()?
                - aid.volume_mm3()?)
            .abs()
                < 1e-6
        );
        reports.push(serde_json::json!({"kind":kind,"project":path,"step":step,"feature":request.feature,"part":request.body,"definition":request.aid,"bounds_mm":review.bounds_mm,"volume_mm3":review.volume_mm3,"target_relation":format!("{:?}",review.target.relation),"target_distance_mm":review.target.distance_mm,"contact_issues":review.contact_issues,"removal_intent":intent,"finished_part_volume_mm3":target.volume_mm3()?}));
    }
    std::fs::write(
        output.join("native-report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    println!("{}", output.display());
    Ok(())
}
