//! Source-only associative controls. These tests never claim a selected face
//! exists geometrically; the native evaluator must establish that separately.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Definition, Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters::BoundExpression,
    references::{GeometryIntent, Origin, TopologyKind},
    sketch::Datum,
    sketch_regions,
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
struct Fixture {
    session: Session,
    request: ExtrudeRequest,
    plane: EntityId,
    face: FaceInput,
    offset: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let (sketch, edits) = Source::create(Datum::Xy, "Source-only profile").unwrap();
        let mut doc = Document::new("Associative controls")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let mut source = Source::read(&doc, sketch).unwrap().0;
        source.add_rectangle([0., 0.], [20., 10.]).unwrap();
        let regions = sketch_regions::extract(&source)
            .regions
            .iter()
            .map(|r| r.intent())
            .collect();
        doc = doc.edited(&source.replace(&doc, sketch).unwrap()).unwrap();
        let plane = Entity::new(
            "End plane",
            EntityData::Plane {
                origin_mm: [0., 0., 40.],
                normal: [0., 0., 1.],
                anchor: None,
            },
        );
        let producer = Entity::new(
            "Face producer source",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.face_source".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let offset = Entity::new(
            "offset",
            EntityData::Parameter {
                expression: "2".into(),
                unit: "cm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let face = FaceInput {
            source: producer.id,
            intent: GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins: BTreeSet::from([Origin {
                    source: producer.id,
                    role: "cap.end".into(),
                }]),
                boundary: None,
                attachment: None,
            },
        };
        doc = doc
            .edited(&[
                Mutation::Insert(plane.clone()),
                Mutation::Insert(producer),
                Mutation::Insert(offset.clone()),
            ])
            .unwrap();
        Self {
            session: Session::new(doc).unwrap(),
            request: ExtrudeRequest::new(sketch, regions, Default::default()),
            plane: plane.id,
            face,
            offset: offset.id,
        }
    }
    fn apply(&mut self, edits: Vec<Mutation>) {
        self.session
            .apply(Request {
                expected_revision: self.session.revision(),
                actor: actor(),
                label: "Fixture edit".into(),
                edits,
            })
            .unwrap();
    }
    fn face_extent(&self) -> Extent {
        Extent::OneSided {
            end: Termination::ToFace {
                reference: self.face.intent.id,
                offset: length(3.),
            },
        }
    }
    fn preview(&self) -> ModelPrepared {
        prepare_extrude(
            &self.session,
            self.session.revision(),
            actor(),
            &self.request,
        )
        .unwrap()
    }
}
#[test]
fn associative_controls_keep_target_identities_and_signed_offsets_without_distances() {
    let mut f = Fixture::new();
    let expression = BoundExpression::bind(f.session.document(), "=$offset", "mm", None).unwrap();
    f.request.parameters = ExtrudeParameters {
        direction: Direction {
            elevation: angle(135.),
            azimuth: angle(0.),
            reverse: true,
        },
        start_offset: Dimension::Literal {
            value: -1.,
            unit: "cm".into(),
        },
        extent: Extent::TwoSided {
            forward: Termination::ToPlane {
                plane: f.plane,
                offset: Dimension::Expression {
                    expression: expression.clone(),
                },
            },
            backward: Termination::ToFace {
                reference: f.face.intent.id,
                offset: length(-3.),
            },
        },
        ..Default::default()
    };
    f.request.faces = vec![f.face.clone()];
    let resolved = f
        .request
        .parameters
        .resolve_controls(f.session.parameters())
        .unwrap();
    assert!((resolved.direction[0] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
    assert!((resolved.direction[2] + std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
    assert_eq!(resolved.start_offset_mm, -10.);
    assert_eq!(resolved.taper_rad, 0.);
    assert_eq!(
        resolved.extent,
        ResolvedExtent::TwoSided {
            forward: ResolvedTermination::ToPlane {
                plane: f.plane,
                offset_mm: 20.
            },
            backward: ResolvedTermination::ToFace {
                reference: f.face.intent.id,
                offset_mm: -3.
            }
        }
    );
    assert!(
        f.request
            .parameters
            .resolve(f.session.parameters())
            .is_err(),
        "Associative controls silently fabricated a distance"
    );
    let before = f.session.document().clone();
    let preview = f.preview();
    assert_eq!(f.session.document(), &before);
    let source = read_extrude(preview.prepared.document(), f.request.outputs[0].feature).unwrap();
    assert_eq!(source.parameters, f.request.parameters);
    assert_eq!(
        preview.prepared.document().get(f.request.sketch),
        before.get(f.request.sketch)
    );
    let links = preview
        .prepared
        .document()
        .get(f.request.outputs[0].feature)
        .unwrap()
        .references();
    assert!(links.iter().any(|l| l.target == f.plane));
    assert!(links.iter().any(|l| l.target == f.face.source));
    assert!(links.iter().any(|l| l.target == f.offset));
}
#[test]
fn plane_and_face_source_preparations_are_cancelable_undoable_and_preserve_links_on_edit() {
    for face in [false, true] {
        let mut f = Fixture::new();
        f.request.parameters.extent = if face {
            f.face_extent()
        } else {
            Extent::OneSided {
                end: Termination::ToPlane {
                    plane: f.plane,
                    offset: length(-2.),
                },
            }
        };
        if face {
            f.request.faces = vec![f.face.clone()];
        }
        let before = f.session.document().clone();
        let first = f.preview();
        let second = f.preview();
        assert_eq!(first.prepared.document(), second.prepared.document());
        drop(first);
        assert_eq!(f.session.document(), &before);
        f.session.commit(second.prepared).unwrap();
        let feature = f.request.outputs[0].feature;
        let committed = f.session.document().clone();
        let mut changed = f.request.parameters.clone();
        changed.start_offset = length(7.);
        let preview = prepare_extrude_edit(&f.session, 1, actor(), feature, &changed).unwrap();
        assert_eq!(f.session.document(), &committed);
        if face {
            let link = preview
                .prepared
                .document()
                .get(feature)
                .unwrap()
                .references()
                .into_iter()
                .find(|l| l.geometry.is_some())
                .unwrap();
            assert_eq!(link.target, f.face.source);
            assert_eq!(link.geometry, Some(f.face.intent.clone()));
        }
        f.session.commit(preview.prepared).unwrap();
        f.session.undo(2, actor()).unwrap();
        assert_eq!(f.session.document(), &committed);
        f.session.undo(3, actor()).unwrap();
        assert_eq!(f.session.document(), &before);
    }
}
#[test]
fn edits_can_supply_new_face_inputs_and_retain_a_repaired_link_as_the_only_selector() {
    let mut f = Fixture::new();
    f.request.parameters.extent = Extent::OneSided {
        end: Termination::Distance {
            distance: length(5.),
        },
    };
    let first = f.preview();
    f.session.commit(first.prepared).unwrap();
    let feature = f.request.outputs[0].feature;
    let mut changed = f.request.parameters.clone();
    changed.extent = f.face_extent();
    assert!(
        prepare_extrude_edit(&f.session, 1, actor(), feature, &changed).is_err(),
        "A new reference ID was accepted without its selector"
    );
    let preview = prepare_extrude_edit_with_faces(
        &f.session,
        1,
        actor(),
        feature,
        &changed,
        std::slice::from_ref(&f.face),
    )
    .unwrap();
    f.session.commit(preview.prepared).unwrap();
    let replacement = Entity::new(
        "Replacement producer",
        f.session
            .document()
            .get(f.face.source)
            .unwrap()
            .data
            .clone(),
    );
    f.apply(vec![Mutation::Insert(replacement.clone())]);
    let mut entity = f.session.document().get(feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut entity.data else {
        panic!()
    };
    let link = inputs.iter_mut().find(|l| l.geometry.is_some()).unwrap();
    link.target = replacement.id;
    link.geometry.as_mut().unwrap().origins = BTreeSet::from([Origin {
        source: replacement.id,
        role: "cap.end".into(),
    }]);
    f.apply(vec![Mutation::Replace(entity)]);
    let repaired = f
        .session
        .document()
        .get(feature)
        .unwrap()
        .references()
        .into_iter()
        .find(|l| l.geometry.is_some())
        .unwrap();
    changed.start_offset = length(9.);
    let preview =
        prepare_extrude_edit(&f.session, f.session.revision(), actor(), feature, &changed).unwrap();
    assert_eq!(
        preview
            .prepared
            .document()
            .get(feature)
            .unwrap()
            .references()
            .into_iter()
            .find(|l| l.geometry.is_some())
            .unwrap(),
        repaired
    );
    assert_eq!(
        read_extrude(preview.prepared.document(), feature)
            .unwrap()
            .parameters,
        changed
    );
}
#[test]
fn missing_wrong_kind_suppressed_and_unmatched_termination_inputs_fail_preparation() {
    let mut f = Fixture::new();
    let original = f.session.document().clone();
    for id in [EntityId::new(), f.face.source, f.request.sketch] {
        f.request.parameters.extent = Extent::OneSided {
            end: Termination::ToPlane {
                plane: id,
                offset: length(0.),
            },
        };
        assert!(prepare_extrude(&f.session, 0, actor(), &f.request).is_err());
    }
    f.request.parameters.extent = f.face_extent();
    assert!(prepare_extrude(&f.session, 0, actor(), &f.request).is_err());
    for source in [EntityId::new(), f.plane, f.request.sketch] {
        f.request.faces = vec![FaceInput {
            source,
            ..f.face.clone()
        }];
        assert!(prepare_extrude(&f.session, 0, actor(), &f.request).is_err());
    }
    let mut wrong = f.face.clone();
    wrong.intent.kind = TopologyKind::Edge;
    f.request.faces = vec![wrong];
    assert!(prepare_extrude(&f.session, 0, actor(), &f.request).is_err());
    assert_eq!(f.session.document(), &original);
    let mut producer = original.get(f.face.source).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut producer.data else {
        panic!()
    };
    *suppressed = true;
    f.apply(vec![Mutation::Replace(producer)]);
    f.request.faces = vec![f.face.clone()];
    assert!(prepare_extrude(&f.session, 1, actor(), &f.request).is_err());
}
#[test]
fn missing_termination_sources_remain_readable_and_repairable_but_not_prepareable() {
    for face in [false, true] {
        let mut f = Fixture::new();
        f.request.parameters.extent = if face {
            f.face_extent()
        } else {
            Extent::OneSided {
                end: Termination::ToPlane {
                    plane: f.plane,
                    offset: length(0.),
                },
            }
        };
        if face {
            f.request.faces = vec![f.face.clone()];
        }
        let preview = f.preview();
        f.session.commit(preview.prepared).unwrap();
        let feature = f.request.outputs[0].feature;
        let removed = if face { f.face.source } else { f.plane };
        f.apply(vec![Mutation::Remove(removed)]);
        let original = f.session.document().clone();
        let source = read_extrude(&original, feature).unwrap();
        assert!(
            original
                .reference_diagnostics()
                .iter()
                .any(|d| d.owner == feature && d.target == removed)
        );
        assert!(
            prepare_extrude_edit(
                &f.session,
                f.session.revision(),
                actor(),
                feature,
                &source.parameters
            )
            .is_err()
        );
        assert_eq!(f.session.document(), &original);
    }
}
#[test]
fn formula_bindings_stay_stable_and_numeric_compatibility_checks_remain_strict() {
    let mut f = Fixture::new();
    let bound = BoundExpression::bind(f.session.document(), "=$offset", "mm", None).unwrap();
    f.request.parameters.extent = Extent::OneSided {
        end: Termination::ToPlane {
            plane: f.plane,
            offset: Dimension::Expression {
                expression: bound.clone(),
            },
        },
    };
    let preview = f.preview();
    f.session.commit(preview.prepared).unwrap();
    let feature = f.request.outputs[0].feature;
    let canonical = read_extrude(f.session.document(), feature).unwrap();
    let mut parameter = f.session.document().get(f.offset).unwrap().clone();
    parameter.name = "renamed_offset".into();
    f.apply(vec![Mutation::Replace(parameter)]);
    assert_eq!(
        canonical
            .parameters
            .resolve_controls(f.session.parameters())
            .unwrap()
            .extent,
        ResolvedExtent::OneSided {
            end: ResolvedTermination::ToPlane {
                plane: f.plane,
                offset_mm: 20.
            }
        }
    );
    assert_eq!(
        read_extrude(f.session.document(), feature).unwrap(),
        canonical
    );
    let mut invalid = canonical.parameters.clone();
    invalid.taper = angle(1.);
    assert!(invalid.resolve_controls(f.session.parameters()).is_err());
    invalid = canonical.parameters.clone();
    invalid.extent = Extent::TwoSided {
        forward: Termination::Distance {
            distance: length(9e8),
        },
        backward: Termination::Distance {
            distance: length(9e8),
        },
    };
    assert!(invalid.resolve_controls(f.session.parameters()).is_err());
    invalid.extent = Extent::Symmetric { total: length(20.) };
    let numeric = invalid.resolve(f.session.parameters()).unwrap();
    assert_eq!((numeric.forward_mm, numeric.backward_mm), (10., 10.));
}
