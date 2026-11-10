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
    let parameters = GearParameters {
        family: GearFamily::Helical,
        hand: GearHand::Right,
        helix_angle: angle(15.),
        ..request.parameters
    };
    let draft =
        model::prepare_gear_edit(&session, 1, actor(), request.feature, &parameters).unwrap();
    let error = Compiler::default()
        .compile_with_backend(draft.prepared.document(), 2, GeometryBackend::Rust, None)
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("external spur gear or a straight rack"),
        "{error}"
    );
    assert_eq!(session.document(), &baseline);
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
        &rust.snapshot().shapes[&feature],
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    near(verified, volume, 1e-4);
}

#[test]
fn rust_spur_pairs_require_exact_interference_proof_before_commit() {
    let (session, pinion) = fixture();
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
    let compiler = Compiler::default();
    let document = draft.prepared.document();
    let scoped = compiler
        .compile_sources_with_backend(document, 1, &[pinion_feature], GeometryBackend::Rust, None)
        .unwrap();
    assert!(scoped.solid(pinion_feature).unwrap().uses_rust_geometry());
    let error = compiler
        .compile_with_backend(document, 1, GeometryBackend::Rust, None)
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("exact interference validation"),
        "{error}"
    );
    assert_eq!(session.document(), &baseline);
    assert_eq!(session.revision(), 0);
}
