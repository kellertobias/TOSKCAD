use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::{
        self, BodyInput, GearFamily, GearHand, GearHub, GearPairRequest, GearParameters,
        GearRequest, InstanceCopyRequest, InstanceCopySource, NumericTransform, PrimitiveFrame,
        angle, length,
    },
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_model::{Compiler, GeometryBackend};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn fixture() -> (Session, GearRequest) {
    let session = Session::new(Document::new("Placed spur gear").unwrap()).unwrap();
    let request = GearRequest::new(
        "Spur gear",
        GearParameters {
            face_width: length(5.),
            frame: PrimitiveFrame {
                origin: [length(10.), length(20.), length(30.)],
                u: [0., 1., 0.],
                v: [0.8, 0., 0.6],
            },
            ..Default::default()
        },
    );
    (session, request)
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} differs from {expected} by more than {tolerance}"
    );
}

fn near_point(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
    for (a, b) in actual.into_iter().zip(expected) {
        near(a, b, tolerance);
    }
}

#[test]
fn rust_spur_gears_preserve_involute_geometry_placement_and_face_origins() {
    for (teeth, bore_mm) in [(24, 8.), (40, 0.)] {
        let (mut session, mut request) = fixture();
        request.parameters.teeth = Dimension::Literal {
            value: f64::from(teeth),
            unit: "count".into(),
        };
        request.parameters.bore_diameter = length(bore_mm);
        let draft = model::prepare_gear(&session, 0, actor(), &request).unwrap();
        assert!(session.document().get(request.feature).is_none());
        session.commit(draft.prepared).unwrap();
        let occurrence = InstanceCopyRequest::new(
            InstanceCopySource::Part {
                source: BodyInput {
                    body: request.body,
                    producer: request.feature,
                },
            },
            NumericTransform {
                translation: [7., -13., 9.].map(length),
                angle: angle(90.),
                ..Default::default()
            },
        );
        let draft = model::prepare_instance_copies(
            &session,
            session.revision(),
            actor(),
            std::slice::from_ref(&occurrence),
        )
        .unwrap();
        session.commit(draft.prepared).unwrap();

        let compiler = Compiler::default();
        let build = |backend| {
            compiler
                .compile_with_backend(session.document(), session.revision(), backend, None)
                .unwrap_or_else(|e| panic!("{backend:?}: {e}"))
        };
        let rust = build(GeometryBackend::Rust);
        let feature = request.feature;
        let solid = rust.solid(feature).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert_eq!(rust.snapshot().bodies[&request.body], feature);
        let shape = &rust.snapshot().shapes[&feature];
        assert!(!shape.mesh.indices.is_empty());
        let resolved = request
            .parameters
            .resolve_geometry(session.parameters())
            .unwrap();
        let model::ResolvedGearGeometry::Rotary(resolved) = resolved else {
            panic!("Expected external spur geometry");
        };
        let tip = resolved.tip_radius_mm;
        let [min, max] = [
            [10. - 0.8 * tip, 20. - tip, 30. - 0.6 * tip - 0.8 * 5.],
            [10. + 0.8 * tip + 0.6 * 5., 20. + tip, 30. + 0.6 * tip],
        ];
        let bounds = solid.bounds().unwrap();
        near_point(bounds.min, min, 1e-8);
        near_point(bounds.max, max, 1e-8);
        let placed = &rust.snapshot().occurrences[&occurrence.instance];
        let placed_solid = rust.solid(occurrence.instance).unwrap();
        assert_eq!(placed.producer, feature);
        near(
            placed_solid.volume_mm3().unwrap(),
            solid.volume_mm3().unwrap(),
            1e-9,
        );
        let placed_min = [7. - max[1], min[0] - 13., min[2] + 9.];
        let placed_max = [7. - min[1], max[0] - 13., max[2] + 9.];
        near_point(placed.bounds.min, placed_min, 1e-8);
        near_point(placed.bounds.max, placed_max, 1e-8);

        let face = |role: &str| {
            shape.topology.elements.iter().find(|element| {
                element.key.kind == TopologyKind::Face
                    && element
                        .origins
                        .iter()
                        .any(|origin| origin.source == feature && origin.role == role)
            })
        };
        assert!(face("gear/end.start").is_some());
        assert!(face("gear/end.end").is_some());
        assert_eq!(face("gear/bore").is_some(), bore_mm > 0.);
        let has_continuation = (resolved.root_radius_mm
            * (resolved.root_radius_mm + 2. * resolved.root_fillet_radius_mm))
            .sqrt()
            < resolved.base_radius_mm;
        for tooth in 0..teeth {
            let role = |region| format!("gear/whole/tooth/{tooth}/{region}");
            for region in [
                "fillet.left",
                "involute.left",
                "tip",
                "involute.right",
                "fillet.right",
                "root",
            ] {
                let role = role(region);
                assert!(face(&role).is_some(), "Missing {role}");
            }
            for region in ["continuation.left", "continuation.right"] {
                assert_eq!(face(&role(region)).is_some(), has_continuation);
            }
        }
        let mut involutes = 0;
        let rotate = |[x, y, z]: [f64; 3]| [-y, x, z];
        for (source, world) in shape
            .topology
            .elements
            .iter()
            .zip(&placed.topology.elements)
        {
            let Some(
                geometry @ AnalyticGeometry::InvoluteArc {
                    frame,
                    base_radius_mm,
                    parameters,
                },
            ) = source.geometry
            else {
                continue;
            };
            involutes += 1;
            geometry.validate().unwrap();
            assert!(
                shape
                    .edges
                    .iter()
                    .any(|edge| { edge.index == source.key.ordinal && edge.points_mm.len() >= 2 })
            );
            assert_eq!(source.key, world.key);
            let Some(AnalyticGeometry::InvoluteArc {
                frame: actual_frame,
                base_radius_mm: actual_radius,
                parameters: actual_interval,
            }) = world.geometry
            else {
                panic!("Placed involute descriptor was lost");
            };
            assert_eq!(actual_interval, parameters);
            assert_eq!(actual_radius, base_radius_mm);
            let [x, y, z] = rotate(frame.origin_mm);
            near_point(actual_frame.origin_mm, [x + 7., y - 13., z + 9.], 1e-9);
            near_point(actual_frame.u, rotate(frame.u), 1e-9);
            near_point(actual_frame.v, rotate(frame.v), 1e-9);
        }
        assert_eq!(involutes, 4 * teeth);
        let mesh = solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        assert!(!mesh.mesh.indices.is_empty());

        #[cfg(feature = "occt")]
        {
            let native = build(GeometryBackend::OpenCascade);
            let reference = native.solid(feature).unwrap();
            // The OCCT comparator evaluates tolerance-bounded spline flanks.
            near(
                solid.volume_mm3().unwrap(),
                reference.volume_mm3().unwrap(),
                1e-4,
            );
            let native_bounds = reference.bounds().unwrap();
            near_point(bounds.min, native_bounds.min, 1e-5);
            near_point(bounds.max, native_bounds.max, 1e-5);
        }
    }
}

#[test]
fn rust_spur_reports_unavailable_geometry_before_recipe_commit() {
    let (mut session, request) = fixture();
    let draft = model::prepare_gear(&session, 0, actor(), &request).unwrap();
    session.commit(draft.prepared).unwrap();
    let baseline = session.document().clone();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let committed = || {
        compiler
            .compile_with_backend(
                session.document(),
                session.revision(),
                GeometryBackend::Rust,
                None,
            )
            .unwrap()
    };
    let before = committed();
    let parameters = GearParameters {
        family: GearFamily::Helical,
        hand: GearHand::Right,
        helix_angle: angle(15.),
        ..request.parameters
    };
    let draft =
        model::prepare_gear_edit(&session, 1, actor(), request.feature, &parameters).unwrap();
    let error = compiler
        .compile_with_backend(draft.prepared.document(), 2, GeometryBackend::Rust, None)
        .err()
        .unwrap();
    assert_eq!(error.entity, Some(request.feature));
    assert!(
        error
            .message
            .starts_with("Rust gear geometry currently requires"),
        "{error}"
    );
    assert_eq!(session.revision(), 1);
    assert_eq!(session.document(), &baseline);
    let retained = committed();
    assert_eq!(retained.snapshot().bodies[&request.body], request.feature);
    let a = before.solid(request.feature).unwrap();
    let b = retained.solid(request.feature).unwrap();
    assert_eq!(b.solid_count().unwrap(), 1);
    assert_eq!(b.bounds().unwrap(), a.bounds().unwrap());
    near(b.volume_mm3().unwrap(), a.volume_mm3().unwrap(), 1e-9);
}

#[test]
fn rust_spur_hub_extends_the_bore_and_keeps_native_face_roles() {
    let (mut session, mut request) = fixture();
    request.parameters.frame = PrimitiveFrame::default();
    let feature = request.feature;
    let draft = model::prepare_gear(&session, 0, actor(), &request).unwrap();
    session.commit(draft.prepared).unwrap();
    let compiler = Compiler::default();
    let plain = compiler
        .compile_with_backend(session.document(), 1, GeometryBackend::Rust, None)
        .unwrap();
    request.parameters.hub = Some(GearHub {
        diameter: length(18.),
        axial_start: length(-3.),
        axial_end: length(8.),
    });
    let draft =
        model::prepare_gear_edit(&session, 1, actor(), feature, &request.parameters).unwrap();
    session.commit(draft.prepared).unwrap();
    let build = |backend| {
        compiler
            .compile_with_backend(session.document(), 2, backend, None)
            .unwrap()
    };
    let rust = build(GeometryBackend::Rust);
    let solid = rust.solid(feature).unwrap();
    let volume = solid.volume_mm3().unwrap();
    let added = 6. * std::f64::consts::PI * (81. - 16.);
    near(
        volume - plain.solid(feature).unwrap().volume_mm3().unwrap(),
        added,
        1e-8,
    );
    let bounds = solid.bounds().unwrap();
    near(bounds.min[2], -3., 1e-8);
    near(bounds.max[2], 8., 1e-8);
    let topology = &rust.snapshot().shapes[&feature].topology;
    let role = |role: &'static str| {
        topology.elements.iter().filter(move |element| {
            element.key.kind == TopologyKind::Face
                && element
                    .origins
                    .iter()
                    .any(|origin| origin.source == feature && origin.role == role)
        })
    };
    assert_eq!(role("gear/hub.side").count(), 2);
    assert_eq!(role("gear/hub.end.start").count(), 1);
    assert_eq!(role("gear/hub.end.end").count(), 1);
    let bore = role("gear/bore").next().unwrap().key.ordinal;
    let faces = solid.faces().unwrap();
    near(
        faces[bore as usize].area_mm2,
        std::f64::consts::TAU * 4. * 11.,
        1e-8,
    );
    solid
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    #[cfg(feature = "occt")]
    {
        let native = build(GeometryBackend::OpenCascade);
        let reference = native.solid(feature).unwrap();
        near(volume, reference.volume_mm3().unwrap(), 1e-4);
        near(
            faces.iter().map(|face| face.area_mm2).sum(),
            reference
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum(),
            1e-4,
        );
    }
    assert!(solid.supports_step_export());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("spur-hub.step");
    solid.write_step(&path).unwrap();
    let verified = multicad_model::export::verify_rust_step_shape(
        &path,
        rust.solid(feature).unwrap(),
        &rust.snapshot().shapes[&feature],
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    near(verified, volume, 1e-4);
}

#[test]
fn rust_spur_pair_certifies_authored_motion_and_reopens_with_the_same_proof() {
    use multicad_core::project_file::{self, FileLimits, ProjectFile};
    use multicad_kernel::{
        BooleanOperation, RigidTransform, rust_kernel::spur_gear::clearance::ResultKind,
    };

    let (mut session, pinion) = fixture();
    let wheel = GearRequest::new(
        "Spur wheel",
        GearParameters {
            teeth: Dimension::Literal {
                value: 36.,
                unit: "count".into(),
            },
            ..pinion.parameters.clone()
        },
    );
    let pinion_feature = pinion.feature;
    let baseline = session.document().clone();
    let request = GearPairRequest::new("Spur gear pair", [pinion, wheel]);
    let draft = model::prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let document = draft.prepared.document();
    let scoped = compiler
        .compile_sources_with_backend(document, 1, &[pinion_feature], GeometryBackend::Rust, None)
        .unwrap();
    assert!(scoped.solid(pinion_feature).unwrap().uses_rust_geometry());
    assert!(scoped.gear_pair_checks().is_empty());

    let rust = compiler
        .compile_with_backend(document, 1, GeometryBackend::Rust, None)
        .unwrap();
    let report = &rust.gear_pair_checks()[&request.pair];
    assert_eq!(report.center_distance_mm, 60.);
    assert_eq!(report.cycle_drive_angles_rad.len(), 9);
    assert!(report.current_pose_checked);
    assert_eq!(report.maximum_overlap_volume_mm3, 0.);
    assert_eq!(session.document(), &baseline);
    assert_eq!(session.revision(), 0);

    let [a, b] = request.outputs.instances;
    let phase = -std::f64::consts::PI / 36.;
    let (sin, cos) = phase.sin_cos();
    let wrong = RigidTransform {
        rotation: [[cos, -sin, 0.], [sin, cos, 0.], [0., 0., 1.]],
        translation_mm: [60. * (1. - cos), -60. * sin, 0.],
    };
    let wrong_rust = rust.solid(b).unwrap().transformed(wrong).unwrap();
    assert_eq!(
        rust.solid(a)
            .unwrap()
            .certify_rust_spur_pair(&wrong_rust, compiler.policy, None)
            .unwrap(),
        ResultKind::CommonInterior
    );
    if multicad_kernel::OCCT_LINKED {
        let native = compiler
            .compile_with_backend(document, 1, GeometryBackend::OpenCascade, None)
            .unwrap();
        let oracle = &native.gear_pair_checks()[&request.pair];
        assert_eq!(oracle.cycle_drive_angles_rad, report.cycle_drive_angles_rad);
        assert_eq!(oracle.maximum_overlap_volume_mm3, 0.);
        let wrong_native = native.solid(b).unwrap().transformed(wrong).unwrap();
        let overlap = native
            .solid(a)
            .unwrap()
            .boolean(&wrong_native, BooleanOperation::Intersect, compiler.policy)
            .unwrap()
            .unwrap()
            .volume_mm3()
            .unwrap();
        assert!(overlap > 1., "{overlap}");
    }

    session.commit(draft.prepared).unwrap();
    let mut parameters = request.parameters;
    parameters.drive_angle = angle(4.);
    let edited = model::prepare_gear_pair_edit(
        &session,
        session.revision(),
        actor(),
        request.pair,
        &parameters,
    )
    .unwrap();
    let moved = compiler
        .compile_with_backend(
            edited.prepared.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    near(
        moved.gear_pair_checks()[&request.pair].current_drive_angle_rad,
        4_f64.to_radians(),
        1e-12,
    );
    session.commit(edited.prepared).unwrap();

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let readback = compiler
        .compile_with_backend(&reopened.project.document, 0, GeometryBackend::Rust, None)
        .unwrap();
    let proof = &readback.gear_pair_checks()[&request.pair];
    assert_eq!(proof.cycle_drive_angles_rad, report.cycle_drive_angles_rad);
    near(proof.current_drive_angle_rad, 4_f64.to_radians(), 1e-12);
    assert!(proof.current_pose_checked);
}

#[test]
fn rust_herringbone_sections_rotate_the_tooth_profile_and_preserve_the_bore() {
    use multicad_core::{
        drawings::{VectorCurve, VectorRole},
        materials::HatchPattern,
    };
    use multicad_kernel::{
        DrawingHatch, DrawingProjection, DrawingSolid, PlaneFrame, project_drawing,
    };

    let mut session = Session::new(Document::new("Herringbone drawing").unwrap()).unwrap();
    let request = GearRequest::new(
        "Chevron",
        GearParameters {
            family: GearFamily::Herringbone,
            hand: GearHand::Right,
            helix_angle: angle(25.),
            face_width: length(8.),
            bore_diameter: length(8.),
            ..Default::default()
        },
    );
    let prepared = model::prepare_gear(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(
        session.document(),
        session.revision(),
        GeometryBackend::Rust,
        None,
    )
    .unwrap();
    let solid = model.solid(request.feature).unwrap();
    let resolved = request
        .parameters
        .resolve_geometry(session.parameters())
        .unwrap();
    let model::ResolvedGearGeometry::Rotary(gear) = resolved else {
        panic!("Expected a gear");
    };
    let cut = |depth, frame, window, hatch| {
        project_drawing(
            &[DrawingSolid { solid, hatch }],
            DrawingProjection {
                frame,
                section_offset_mm: Some(depth),
                section_only: true,
                section_u_mm: window,
                chord_tolerance_mm: 0.003,
                ..Default::default()
            },
        )
        .unwrap()
    };

    let end = cut(0., PlaneFrame::default(), None, None);
    let middle = cut(4., PlaneFrame::default(), None, None);
    assert_eq!(end.len(), middle.len());
    assert!(middle.iter().all(|edge| edge.role == VectorRole::Section));
    assert!(middle.iter().any(|edge| matches!(
        edge.curve,
        VectorCurve::Ellipse {
            center: [0., 0.],
            major_mm: 4.,
            minor_mm: 4.,
            ..
        }
    )));
    let mut flank_count = 0;
    let (sin, cos) = (gear.twist_rad_per_mm * 4.).sin_cos();
    for (from, to) in end.iter().zip(&middle) {
        if let (VectorCurve::BSpline { poles: a, .. }, VectorCurve::BSpline { poles: b, .. }) =
            (&from.curve, &to.curve)
        {
            flank_count += 1;
            for (&[x, y], &[u, v]) in a.iter().zip(b) {
                near(u, x * cos - y * sin, 1e-8);
                near(v, x * sin + y * cos, 1e-8);
            }
        }
    }
    assert_eq!(flank_count, 2 * gear.teeth as usize);
    let mirrored = cut(
        -4.,
        PlaneFrame {
            v: [0., -1., 0.],
            ..Default::default()
        },
        None,
        None,
    );
    assert_eq!(mirrored.len(), middle.len());
    assert!(cut(9., PlaneFrame::default(), None, None).is_empty());

    let projection =
        |options| project_drawing(&[DrawingSolid { solid, hatch: None }], options).unwrap();
    let tip_span = |edges: &[multicad_kernel::ProjectedVector]| {
        edges
            .iter()
            .filter_map(|edge| match edge.curve {
                VectorCurve::Ellipse {
                    major_mm,
                    start_rad,
                    end_rad,
                    ..
                } if edge.role != VectorRole::Hidden
                    && (major_mm - gear.tip_radius_mm).abs() < 1e-8 =>
                {
                    Some(end_rad - start_rad)
                }
                _ => None,
            })
            .sum::<f64>()
    };
    let cap_span = tip_span(&end);
    let projected = projection(DrawingProjection {
        hidden_lines: true,
        ..Default::default()
    });
    let sweep = gear.twist_rad_per_mm.abs();
    let expected = |length: f64| {
        (cap_span + f64::from(gear.teeth) * sweep * length).min(std::f64::consts::TAU)
    };
    near(tip_span(&projected), expected(4.), 1e-8);
    assert!(tip_span(&projected) > cap_span);
    assert!(projected.iter().any(|edge| edge.role == VectorRole::Hidden));
    assert!(
        projected.iter().any(|edge| edge.role == VectorRole::Visible
            && matches!(edge.curve, VectorCurve::Line { .. }))
    );
    let retained = projection(DrawingProjection {
        section_offset_mm: Some(6.),
        section_depth_mm: Some(1.),
        ..Default::default()
    });
    near(tip_span(&retained), expected(1.), 1e-8);
    assert!(retained.iter().any(|edge| edge.role == VectorRole::Section));
    assert!(retained.iter().any(|edge| edge.role == VectorRole::Visible));
    assert!(retained.iter().all(|edge| edge.role != VectorRole::Hidden));
    let reversed = projection(DrawingProjection {
        frame: PlaneFrame {
            v: [0., -1., 0.],
            ..Default::default()
        },
        ..Default::default()
    });
    near(tip_span(&reversed), expected(4.), 1e-8);
    assert!(
        projection(DrawingProjection {
            section_offset_mm: Some(10.),
            section_depth_mm: Some(1.),
            ..Default::default()
        })
        .is_empty()
    );
    assert!(
        project_drawing(
            &[DrawingSolid { solid, hatch: None }],
            DrawingProjection {
                maximum_curves: 1,
                ..Default::default()
            }
        )
        .is_err()
    );

    let hatch = DrawingHatch {
        angle_rad: 0.,
        spacing_mm: 1.,
        line_width_mm: 0.15,
        pattern: HatchPattern::Lines,
    };
    let clipped = cut(4., PlaneFrame::default(), Some([-1., 20.]), Some(hatch));
    assert!(
        clipped
            .iter()
            .flat_map(|edge| &edge.preview_mm)
            .all(|[x, _]| *x >= -1.000001 && *x <= 20.000001)
    );
    let strokes: Vec<_> = clipped
        .iter()
        .filter(|edge| edge.role == VectorRole::Hatch)
        .collect();
    assert!(!strokes.is_empty());
    for edge in strokes {
        let VectorCurve::Line { a, b } = edge.curve else {
            panic!("Expected a hatch line");
        };
        let [x, y]: [f64; 2] = std::array::from_fn(|i| a[i].midpoint(b[i]));
        assert!(x.hypot(y) >= 3.9999);
    }
}
