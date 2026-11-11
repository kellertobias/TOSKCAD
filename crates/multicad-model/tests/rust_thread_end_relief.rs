use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Link, Mutation},
    model::{
        self, ThreadEndTreatment, ThreadEndTreatments, ThreadFit, ThreadHand, ThreadParameters,
        ThreadRequest, ThreadStartTreatment, length,
    },
    project_file::{self, ProjectFile},
    references::{Attachment, GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(session: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(
        session.document(),
        session.revision(),
        GeometryBackend::Rust,
        None,
    )
    .unwrap()
}
fn ends(width: f64) -> ThreadEndTreatments {
    ThreadEndTreatments {
        start: ThreadStartTreatment::FlatCut,
        end: ThreadEndTreatment::CylindricalRelief {
            length: length(width),
            radial_depth: length(0.1),
        },
    }
}
#[test]
fn canonical_metric_relief_preserves_source_saved_edits_history_and_far_cap() {
    for hand in [ThreadHand::Right, ThreadHand::Left] {
        let mut s = Session::new(Document::new("Metric terminal relief").unwrap()).unwrap();
        let parameters = ThreadParameters {
            major_diameter: length(6.),
            pitch: length(1.),
            length: length(6.6),
            fit: ThreadFit::Metric6gExternal,
            hand,
            end_treatments: Some(ends(0.6)),
            ..Default::default()
        };
        let request = ThreadRequest::new("M6 relieved shaft", parameters.clone());
        let prepared = model::prepare_thread(&s, 0, actor(), &request).unwrap();
        let initial = s.document().clone();
        let candidate = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(prepared.prepared.document(), 1, GeometryBackend::Rust, None)
        .unwrap();
        assert_eq!(s.document(), &initial);
        let shape = candidate.solid(request.feature).unwrap();
        let volume = shape.volume_mm3().unwrap();
        let snapshot = &candidate.snapshot().shapes[&request.feature].topology;
        let cap = snapshot
            .elements
            .iter()
            .find(|e| {
                e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == "cap.end")
            })
            .unwrap();
        let mut intent = GeometryIntent::capture(snapshot, cap.key).unwrap();
        intent.boundary = None;
        assert_eq!(cap.frames[0].frame.origin_mm[2], 6.6);
        s.commit(prepared.prepared).unwrap();
        let anchor = cap.frames[0].origin.clone();
        let mut attached = intent.clone();
        attached.attachment = Some(Attachment {
            anchor,
            offset_mm: [0.2, -0.3, 0.4],
            angle_rad: 0.37,
        });
        s.apply(Request {
            expected_revision: 1,
            actor: actor(),
            label: "Attach terminal far-cap datum".into(),
            edits: vec![Mutation::Insert(Entity::new(
                "Far-cap datum",
                EntityData::Plane {
                    origin_mm: [0.; 3],
                    normal: [0., 0., 1.],
                    anchor: Some(Link {
                        target: request.feature,
                        role: "face_attachment".into(),
                        geometry: Some(attached),
                    }),
                },
            ))],
        })
        .unwrap();
        compile(&s);
        let first = s.document().clone();
        let edited = ThreadParameters {
            end_treatments: Some(ends(1.1)),
            ..parameters
        };
        let prepared =
            model::prepare_thread_edit(&s, 2, actor(), request.feature, &edited).unwrap();
        let edited_model = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(prepared.prepared.document(), 3, GeometryBackend::Rust, None)
        .unwrap();
        assert!(
            edited_model
                .solid(request.feature)
                .unwrap()
                .volume_mm3()
                .unwrap()
                < volume
        );
        assert!(matches!(
            resolve(
                &intent,
                &edited_model.snapshot().shapes[&request.feature].topology,
                3
            ),
            Resolution::Resolved { .. }
        ));
        s.commit(prepared.prepared).unwrap();
        let saved = s.document().clone();
        s.undo(3, actor()).unwrap();
        assert_eq!(s.document(), &first);
        s.redo(4, actor()).unwrap();
        assert_eq!(s.document(), &saved);
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(saved).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let source = model::read_thread(&reopened.document, request.feature).unwrap();
        assert_eq!(source.parameters, edited);
        let reopened = Session::new(reopened.document).unwrap();
        let built = compile(&reopened);
        let solid = built.solid(request.feature).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relieved.step");
        solid.write_step(&path).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(
            (solid.verify_helical_step(&text).unwrap().unwrap() - solid.volume_mm3().unwrap())
                .abs()
                < 1e-5
        );
        let imported = multicad_kernel::Solid::import_rust_step(
            &text,
            multicad_core::document::EntityId::new(),
            Default::default(),
        )
        .unwrap();
        assert!((imported.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-5);
        let mut invalid = edited.clone();
        invalid.end_treatments = Some(ends(6.6));
        assert!(model::prepare_thread_edit(&s, 5, actor(), request.feature, &invalid).is_err());
        assert_eq!(s.document(), reopened.document());
    }
}
#[test]
fn explicit_flat_flat_is_noop_and_entry_chamfer_remains_an_honest_failure() {
    let mut s = Session::new(Document::new("Explicit flat cuts").unwrap()).unwrap();
    let mut p = ThreadParameters {
        end_treatments: Some(ThreadEndTreatments {
            start: ThreadStartTreatment::FlatCut,
            end: ThreadEndTreatment::FlatCut,
        }),
        ..Default::default()
    };
    let request = ThreadRequest::new("Flat shaft", p.clone());
    let prepared = model::prepare_thread(&s, 0, actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    let first = compile(&s);
    assert_eq!(
        first.solid(request.feature).unwrap().faces().unwrap().len(),
        6
    );
    p.end_treatments = Some(ThreadEndTreatments {
        start: ThreadStartTreatment::Chamfer45 {
            radial_depth: length(0.2),
        },
        end: ThreadEndTreatment::FlatCut,
    });
    let prepared = model::prepare_thread_edit(&s, 1, actor(), request.feature, &p).unwrap();
    assert!(
        Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(prepared.prepared.document(), 2, GeometryBackend::Rust, None)
        .is_err()
    );
    assert_eq!(s.revision(), 1);
    assert_eq!(
        compile(&s)
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        first.solid(request.feature).unwrap().volume_mm3().unwrap()
    );
}

#[cfg(feature = "occt")]
#[test]
fn canonical_terminal_relief_matches_independent_native_construction_and_step_occupancy() {
    use multicad_core::model::PrimitiveFrame;
    use multicad_kernel::{BooleanOperation, RigidTransform, Solid};
    let directory = tempfile::tempdir().unwrap();
    for (diameter, pitch) in [(3., 0.5), (6., 1.)] {
        for hand in [ThreadHand::Right, ThreadHand::Left] {
            for posed in [false, true] {
                let frame = if posed {
                    PrimitiveFrame {
                        origin: [length(17.), length(-3.), length(21.)],
                        u: [0.8, 0.6, 0.],
                        v: [-0.36, 0.48, 0.8],
                    }
                } else {
                    PrimitiveFrame::default()
                };
                let mut session =
                    Session::new(Document::new("Native terminal relief comparison").unwrap())
                        .unwrap();
                let parameters = ThreadParameters {
                    major_diameter: length(diameter),
                    pitch: length(pitch),
                    length: length(1.1 * diameter),
                    hand,
                    fit: ThreadFit::Metric6gExternal,
                    frame,
                    end_treatments: Some(ThreadEndTreatments {
                        start: ThreadStartTreatment::FlatCut,
                        end: ThreadEndTreatment::CylindricalRelief {
                            length: length(0.6 * pitch),
                            radial_depth: length(0.1 * pitch),
                        },
                    }),
                    ..Default::default()
                };
                let resolved = parameters
                    .resolve(
                        session.parameters(),
                        multicad_core::model::ThreadMode::NewExternal,
                    )
                    .unwrap();
                let request = ThreadRequest::new("Native-reference relieved shaft", parameters);
                let prepared = model::prepare_thread(&session, 0, actor(), &request).unwrap();
                session.commit(prepared.prepared).unwrap();
                let rust = compile(&session);
                let native = Compiler {
                    omit_display: true,
                    ..Default::default()
                }
                .compile_with_backend(session.document(), 1, GeometryBackend::OpenCascade, None)
                .unwrap();
                let a = rust.solid(request.feature).unwrap();
                let b = native.solid(request.feature).unwrap();
                assert!(a.uses_rust_geometry());
                assert!(!b.uses_rust_geometry());
                let volume = b.volume_mm3().unwrap();
                assert!((a.volume_mm3().unwrap() - volume).abs() < volume * 1e-6);
                let area = |s: &Solid| {
                    s.faces()
                        .unwrap()
                        .iter()
                        .map(|face| face.area_mm2)
                        .sum::<f64>()
                };
                assert!((area(a) - area(b)).abs() < area(b) * 2e-6);
                let ab = a.bounds().unwrap();
                let bb = b.bounds().unwrap();
                for (x, y) in ab
                    .min
                    .into_iter()
                    .chain(ab.max)
                    .zip(bb.min.into_iter().chain(bb.max))
                {
                    assert!((x - y).abs() < 2e-6, "{x} {y}");
                }
                let path = directory.path().join("relief.step");
                a.write_step(&path).unwrap();
                let decoded = Solid::read_step(&path, Default::default()).unwrap();
                assert!(!decoded.uses_rust_geometry());
                assert!((decoded.volume_mm3().unwrap() - volume).abs() < volume * 1e-6);
                let neck = resolved.major_radius_mm - resolved.profile.depth_mm - 0.1 * pitch;
                let pose = RigidTransform {
                    rotation: std::array::from_fn(|i| {
                        [
                            resolved.frame.u[i],
                            resolved.frame.v[i],
                            resolved.frame.n[i],
                        ]
                    }),
                    translation_mm: resolved.frame.origin,
                };
                for (radius, z, inside) in [
                    (neck - 0.02 * pitch, resolved.length_mm - 0.3 * pitch, true),
                    (neck + 0.02 * pitch, resolved.length_mm - 0.3 * pitch, false),
                    (resolved.major_radius_mm + 0.1 * pitch, 0.4 * pitch, false),
                    (0., 0.4 * pitch, true),
                ] {
                    let point = [radius, 0., z];
                    let world = std::array::from_fn(|i| {
                        resolved.frame.origin[i]
                            + resolved.frame.u[i] * point[0]
                            + resolved.frame.n[i] * point[2]
                    });
                    assert_eq!(a.rust_thread_contains(world).unwrap(), inside);
                    let side = 0.001 * pitch;
                    let probe = Solid::box_solid([side; 3], Default::default())
                        .unwrap()
                        .transformed(RigidTransform {
                            translation_mm: point.map(|v| v - side / 2.),
                            ..Default::default()
                        })
                        .unwrap()
                        .transformed(pose)
                        .unwrap();
                    for s in [b, &decoded] {
                        let occupied = s
                            .boolean(&probe, BooleanOperation::Intersect, Default::default())
                            .unwrap()
                            .map_or(0., |s| s.volume_mm3().unwrap());
                        assert!(
                            if inside {
                                occupied > 0.99 * side.powi(3)
                            } else {
                                occupied < 1e-4 * side.powi(3)
                            },
                            "{occupied} inside{inside}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn parameter_bound_relief_width_remains_a_bound_source_across_history_and_reopen() {
    use multicad_core::{constraint_sketch::Dimension, parameters::BoundExpression};
    let mut doc = Document::new("Bound relief width").unwrap();
    let parameter = Entity::new(
        "reliefWidth",
        EntityData::Parameter {
            expression: "0.6".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    doc.insert(parameter).unwrap();
    let mut session = Session::new(doc).unwrap();
    let width = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$reliefWidth", "mm", None).unwrap(),
    };
    let parameters = ThreadParameters {
        major_diameter: length(6.),
        pitch: length(1.),
        length: length(6.6),
        fit: ThreadFit::Metric6gExternal,
        end_treatments: Some(ThreadEndTreatments {
            start: ThreadStartTreatment::FlatCut,
            end: ThreadEndTreatment::CylindricalRelief {
                length: width,
                radial_depth: length(0.1),
            },
        }),
        ..Default::default()
    };
    let request = ThreadRequest::new("Bound relieved shaft", parameters.clone());
    let prepared = model::prepare_thread(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let initial = session.document().clone();
    let volume = compile(&session)
        .solid(request.feature)
        .unwrap()
        .volume_mm3()
        .unwrap();
    let mut edited = session.document().get(parameter_id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut edited.data else {
        panic!()
    };
    *expression = "1.1".into();
    session
        .apply(Request {
            expected_revision: 1,
            actor: actor(),
            label: "Edit relief width parameter".into(),
            edits: vec![Mutation::Replace(edited)],
        })
        .unwrap();
    assert!(
        compile(&session)
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            < volume
    );
    let saved = session.document().clone();
    assert_eq!(
        model::read_thread(session.document(), request.feature)
            .unwrap()
            .parameters,
        parameters
    );
    session.undo(2, actor()).unwrap();
    assert_eq!(session.document(), &initial);
    session.redo(3, actor()).unwrap();
    assert_eq!(session.document(), &saved);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(
        model::read_thread(&reopened, request.feature)
            .unwrap()
            .parameters,
        parameters
    );
    compile(&Session::new(reopened).unwrap());
}
