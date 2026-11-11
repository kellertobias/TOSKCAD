use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, GeometryIntent, Resolution, TopologyKind},
    sketch::{self, CreationPlane, PlaneOptions},
    sketch_regions,
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn bound(s: &Session, name: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), &format!("=${name}"), "mm", None).unwrap(),
    }
}
fn reopen(s: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    Session::new(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap()
}
fn compile(s: &Session) -> Result<multicad_model::Model, multicad_model::ModelError> {
    Compiler::default().compile_with_backend(
        s.document(),
        s.revision(),
        GeometryBackend::Rust,
        None,
    )
}
#[test]
fn attached_sketch_extrusion_keeps_its_finite_patch_or_requires_repair_after_saved_edits() {
    for keep_left in [false, true] {
        for select_left in [false, true] {
            let mut document = Document::new("Derived Boolean reference history").unwrap();
            for (name, value) in [("slot_x", "25"), ("slot_y", "-20"), ("slot_width", "10")] {
                document
                    .insert(Entity::new(
                        name,
                        EntityData::Parameter {
                            expression: value.into(),
                            unit: "mm".into(),
                            exposed: true,
                            inputs: vec![],
                        },
                    ))
                    .unwrap();
            }
            // The angled planar cutter forces the generic planar Cut provider.
            let mut session = Session::new(document).unwrap();
            let stock = PrimitiveRequest::new(
                "Stock",
                PrimitiveParameters::Box {
                    size: [length(30.), length(20.), length(10.)],
                    frame: PrimitiveFrame::default(),
                },
            );
            session
                .commit(
                    prepare_primitive(&session, session.revision(), actor(), &stock)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            let tool = PrimitiveRequest::new(
                "Slot tool",
                PrimitiveParameters::Box {
                    size: [bound(&session, "slot_width"), length(60.), length(5.)],
                    frame: PrimitiveFrame {
                        origin: [
                            bound(&session, "slot_x"),
                            bound(&session, "slot_y"),
                            length(5.),
                        ],
                        u: [0.8, 0.6, 0.],
                        v: [-0.6, 0.8, 0.],
                    },
                },
            );
            session
                .commit(
                    prepare_primitive(&session, session.revision(), actor(), &tool)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            let cut = BooleanRequest::new(
                Operation::Cut,
                BodyInput {
                    body: stock.body,
                    producer: stock.feature,
                },
                vec![BodyInput {
                    body: tool.body,
                    producer: tool.feature,
                }],
            );
            session
                .commit(
                    prepare_boolean(&session, session.revision(), actor(), &cut)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            let evaluated = compile(&session).unwrap();
            let solid = evaluated.solid(stock.body).unwrap();
            let face = solid
                .faces()
                .unwrap()
                .into_iter()
                .filter(|f| (f.center_mm[2] - 10.).abs() < 1e-7)
                .min_by(|a, b| {
                    if select_left {
                        a.center_mm[0].total_cmp(&b.center_mm[0])
                    } else {
                        b.center_mm[0].total_cmp(&a.center_mm[0])
                    }
                })
                .unwrap();
            let snapshot = &evaluated.snapshot().shapes[&cut.feature].topology;
            let element = snapshot
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == face.index)
                .unwrap();
            let intent = GeometryIntent::capture(snapshot, element.key).unwrap();
            assert!(
                intent
                    .origins
                    .iter()
                    .any(|o| o.role.starts_with("derived-fragment.v1.")),
                "test must exercise derived split ancestry"
            );
            let anchor = element.frames[0].origin.clone();
            let attached = sketch::prepare_new(
                &session,
                session.revision(),
                actor(),
                "Patch-dependent boss",
                CreationPlane::Face {
                    snapshot,
                    key: element.key,
                    anchor: Some(&anchor),
                },
                PlaneOptions::default(),
            )
            .unwrap();
            let (sketch, frame) = (attached.sketch, attached.frame);
            session.commit(attached.prepared).unwrap();
            let support = compile(&session).unwrap();
            let mut strokes = Source::read_for_evaluation(
                session.document(),
                sketch,
                session.revision(),
                Some(&support.snapshot().references),
            )
            .unwrap()
            .0;
            let (x, y) = if select_left { (2., 2.) } else { (27., 16.) };
            strokes
                .add_rectangle(
                    frame.coordinates([x, y, 10.]),
                    frame.coordinates([x + 2., y + 2., 10.]),
                )
                .unwrap();
            let boss_region = sketch_regions::extract(&strokes).regions[0].intent();
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Draw patch boss".into(),
                    edits: strokes.replace(session.document(), sketch).unwrap(),
                })
                .unwrap();
            let boss = ExtrudeRequest::new(
                sketch,
                vec![boss_region],
                ExtrudeParameters {
                    extent: Extent::OneSided {
                        end: Termination::Distance {
                            distance: length(1.),
                        },
                    },
                    ..Default::default()
                },
            );
            session
                .commit(
                    prepare_extrude(&session, session.revision(), actor(), &boss)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            let valid = compile(&session).unwrap();
            assert!(
                (valid
                    .solid(boss.outputs[0].body)
                    .unwrap()
                    .volume_mm3()
                    .unwrap()
                    - 4.)
                    .abs()
                    < 1e-7
            );
            let valid_document = session.document().clone();
            let edits = [
                ("slot_x", if keep_left { "25" } else { "-7" }),
                ("slot_y", if keep_left { "-20" } else { "-44" }),
                ("slot_width", "50"),
            ]
            .map(|(name, value)| {
                let mut e = session
                    .document()
                    .entities()
                    .find(|e| e.name == name)
                    .unwrap()
                    .clone();
                let EntityData::Parameter { expression, .. } = &mut e.data else {
                    panic!()
                };
                *expression = value.into();
                Mutation::Replace(e)
            });
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Extend slot across one patch".into(),
                    edits: edits.into(),
                })
                .unwrap();
            let changed = session.document().clone();
            let opened = reopen(&session);
            if keep_left == select_left {
                let result = compile(&opened).unwrap();
                let topology = &result.snapshot().shapes[&cut.feature].topology;
                let Resolution::Resolved { key, .. } =
                    references::resolve(&intent, topology, opened.revision())
                else {
                    panic!("untouched patch lost")
                };
                let actual = result
                    .solid(stock.body)
                    .unwrap()
                    .faces()
                    .unwrap()
                    .into_iter()
                    .find(|f| f.index == key.ordinal)
                    .unwrap();
                for (a, b) in actual.center_mm.into_iter().zip(face.center_mm) {
                    assert!((a - b).abs() < 1e-7);
                }
                let boss = result.solid(boss.outputs[0].body).unwrap();
                assert!((boss.volume_mm3().unwrap() - 4.).abs() < 1e-7);
                for (a, b) in boss.bounds().unwrap().min.into_iter().zip([x, y, 10.]) {
                    assert!((a - b).abs() < 1e-7);
                }
            } else {
                assert!(
                    compile(&opened).is_err(),
                    "deleted support must not silently attach the boss elsewhere"
                );
            }
            session.undo(session.revision(), actor()).unwrap();
            assert_eq!(session.document(), &valid_document);
            assert!(compile(&reopen(&session)).is_ok());
            session.redo(session.revision(), actor()).unwrap();
            assert_eq!(session.document(), &changed);
            assert_eq!(compile(&reopen(&session)).is_ok(), keep_left == select_left);
        }
    }
}
