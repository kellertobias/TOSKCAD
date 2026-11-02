//! Exact architectural fixture for CAD-247 review. Creates no application window.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
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
        .unwrap_or_else(|| "artifacts/compound-cutout-review-20261102".into());
    std::fs::create_dir_all(&output)?;
    let mut session = Session::new(Document::new("Compound cutout native review")?)?;
    let mut receivers = vec![];
    let mut ground = None;
    for (name, z) in [("Ground", 0.), ("Upper", 3000.)] {
        let (level, prepared) = prepare_level(
            &session,
            session.revision(),
            actor(),
            name,
            &LevelDefinition {
                elevation: length(z),
                height: length(3000.),
                slab_thickness: length(200.),
            },
        )?;
        session.commit(prepared)?;
        ground.get_or_insert(level);
        let prepared = prepare_slab(
            &session,
            session.revision(),
            actor(),
            &format!("{name} floor"),
            &SlabDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                level,
                kind: SlabKind::Slab,
                outline: [[0., 0.], [6000., 0.], [6000., 4000.], [0., 4000.]]
                    .map(|p| p.map(length))
                    .to_vec(),
                thickness: None,
                holes: vec![],
            },
            None,
        )?;
        receivers.push(CompoundCutoutReceiver::new(BodyInput {
            body: prepared.outputs[0].body,
            producer: prepared.outputs[0].feature,
        }));
        session.commit(prepared.prepared)?;
    }
    let wall_type = NewType::new(
        "Block 200",
        NewTypeKind::Wall(WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: None,
        }),
    );
    let wall = WallRequest::new(
        "Receiving wall",
        WallDefinition {
            layer_placement: None,
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: wall_type.id,
            path: [[1000., 1750.], [5000., 1750.]]
                .map(|p| p.map(length))
                .to_vec(),
            closed: false,
            level: ground,
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    );
    let prepared = prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[wall_type],
        &WallChange::Create(&wall),
    )?;
    session.commit(prepared.prepared)?;
    receivers.push(CompoundCutoutReceiver::new(BodyInput {
        body: wall.body,
        producer: wall.feature,
    }));
    let source = CompoundCutoutRequest::new(
        "L-shaped shaft and wall chase",
        CompoundCutoutDefinition {
            boxes: [([2000., 2000.], 1000., 400.), ([1700., 2300.], 400., 1000.)]
                .into_iter()
                .map(|(center, width, depth)| {
                    CompoundCutoutBox::new(PlanRectangle {
                        center: center.map(length),
                        width: length(width),
                        depth: length(depth),
                        rotation_deg: 0.,
                    })
                })
                .collect(),
            receivers,
            extent: CompoundCutoutExtent::ThroughReceivers,
            levels: None,
        },
    );
    let prepared = prepare_compound_cutout(
        &session,
        session.revision(),
        actor(),
        &CompoundCutoutChange::Set(Box::new(source.clone())),
    )?;
    // Validate every receiver before the fixture transaction is committed.
    Compiler::default().compile(prepared.prepared.document(), session.revision() + 1)?;
    session.commit(prepared.prepared)?;
    let native = Compiler::default().compile(session.document(), session.revision())?;
    assert_eq!(native.snapshot().bodies.len(), 3);
    let mut proofs = vec![];
    for (i, receiver) in source.definition.receivers.iter().enumerate() {
        let solid = native
            .solid(receiver.target.body)
            .ok_or("Missing native receiver")?;
        let expected = if i < 2 {
            6000. * 4000. * 200. - (1000. * 400. + 400. * 1000. - 400. * 400.) * 200.
        } else {
            4000. * 200. * 2500. - 1000. * 50. * 2500.
        };
        assert!((solid.volume_mm3()? - expected).abs() < 1e-3);
        let path = output.join(format!("receiver-{}.step", i + 1));
        solid.write_step(&path)?;
        let readback = multicad_kernel::Solid::read_step(&path, Default::default())?;
        assert!((readback.volume_mm3()? - expected).abs() < 1e-3);
        proofs.push(serde_json::json!({"part":receiver.target.body,"feature":receiver.feature,"volume_mm3":expected,"step":path}));
    }
    let path = output.join("compound-cutout.tcd");
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut std::fs::File::create(&path)?,
        Default::default(),
    )?;
    let report = serde_json::json!({"project":path,"document":session.document().id(),"source":source.feature,"definition":source.definition,"native_receivers":proofs,"evidence":"Exact native source and STEP readback. No OS-window interaction."});
    std::fs::write(
        output.join("native-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", path.display());
    Ok(())
}
