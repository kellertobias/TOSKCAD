use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Resolution, resolve},
};
use multicad_kernel::{
    rust_kernel::{
        Rational,
        planar_boolean::{ConvexPiece, Union},
        step,
    },
    *,
};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
}
fn compare(a: &Solid, b: &Solid) {
    near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
    near(
        a.faces().unwrap().iter().map(|face| face.area_mm2).sum(),
        b.faces().unwrap().iter().map(|face| face.area_mm2).sum(),
    );
    let (a, b) = (a.bounds().unwrap(), b.bounds().unwrap());
    for (a, b) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(a, b);
    }
}
fn tetra(frame: PlaneFrame) -> Solid {
    let [a, b, c, d] = [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
    let cell = ConvexPiece::new(
        vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]],
        1e-7,
    )
    .unwrap();
    let boundary = Union::new(&[cell], 1e-7).unwrap();
    let text = step::write_planar_union(&boundary, frame.origin, frame.u, frame.v).unwrap();
    Solid::import_rust_step(&text, EntityId::new(), Default::default()).unwrap()
}
fn box_at(x: f64, width: f64, frame: PlaneFrame) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [width, 1., 1.].map(|v| Rational::new((v * 4.).round() as i64, 4).unwrap()),
        PlaneFrame {
            origin: [x, 0., 0.],
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap()
    .transformed(RigidTransform {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
        translation_mm: frame.origin,
    })
    .unwrap()
}
fn apply(solids: &[&Solid], op: BooleanOperation) -> Option<Solid> {
    Solid::boolean_planar_certified(solids, op, EntityId::new(), Default::default(), None)
        .expect("certified provider selected")
        .unwrap()
}
fn roundtrip(solid: &Solid) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("common.step");
    solid.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let read = Solid::import_rust_step(&text, EntityId::new(), Default::default()).unwrap();
    compare(&read, solid);
    assert_eq!(read.solid_count().unwrap(), solid.solid_count().unwrap());
    solid
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    #[cfg(feature = "occt")]
    {
        let native = Solid::read_step(&path, Default::default()).unwrap();
        compare(&native, solid);
    }
}

#[test]
fn generic_intersection_preserves_authoritative_boundary_through_later_operations_and_scale() {
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [11., -7., 23.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        let stock = tetra(frame);
        let first = box_at(1., 4., frame);
        let second = box_at(1.5, 2.5, frame);
        let owner = EntityId::new();
        let before = stock.topology_snapshot(owner, 0).unwrap();
        let common = apply(&[&stock, &first], BooleanOperation::Intersect).unwrap();
        near(common.volume_mm3().unwrap(), 2.);
        let all = apply(&[&stock, &first, &second], BooleanOperation::Intersect).unwrap();
        near(all.volume_mm3().unwrap(), 1.5);
        let again = apply(&[&common, &second], BooleanOperation::Intersect).unwrap();
        near(again.volume_mm3().unwrap(), 1.5);
        let slab = box_at(1.5, 0.25, frame);
        let parted = apply(&[&all, &slab], BooleanOperation::Subtract).unwrap();
        near(parted.volume_mm3().unwrap(), 1.25);
        let restored = apply(&[&parted, &slab], BooleanOperation::Union).unwrap();
        near(restored.volume_mm3().unwrap(), 1.5);
        let trim = box_at(1.75, 2.25, frame);
        let repartitioned = apply(&[&restored, &trim], BooleanOperation::Intersect).unwrap();
        near(repartitioned.volume_mm3().unwrap(), 1.25);
        near(
            restored
                .rust_planar_intersection_volume(&trim, Default::default())
                .unwrap(),
            1.25,
        );
        let notch = box_at(1., 0.25, frame);
        let cut = common
            .rust_subtract_planar_tools(&[&notch], EntityId::new(), Default::default())
            .unwrap()
            .unwrap();
        near(cut.volume_mm3().unwrap(), 1.75);
        let extension = box_at(0.75, 0.25, frame);
        let join =
            Solid::union_planar_pieces(&[&common, &extension], EntityId::new(), Default::default())
                .unwrap()
                .unwrap();
        near(join.volume_mm3().unwrap(), 2.25);
        // A certified result is also a reusable tool, with its own exposed
        // boundary labels rather than the hidden recovery-cell partitions.
        let tool_cut = stock
            .rust_subtract_planar_tools(&[&common], EntityId::new(), Default::default())
            .unwrap()
            .unwrap();
        near(tool_cut.volume_mm3().unwrap(), 26. / 3.);
        let topology = common.topology_snapshot(owner, 1).unwrap();
        let intents: Vec<_> = topology
            .elements
            .iter()
            .map(|e| GeometryIntent::capture(&topology, e.key).unwrap())
            .collect();
        let scale_feature = EntityId::new();
        let scale = |factor| {
            common
                .affine_transformed_named(
                    AffineTransform {
                        matrix: [[factor, 0., 0.], [0., 3., 0.], [0., 0., 1.]],
                        translation_mm: [5., 2., -3.],
                    },
                    scale_feature,
                    Default::default(),
                )
                .unwrap()
        };
        for factor in [2., 1.5] {
            let scaled = scale(factor);
            near(scaled.volume_mm3().unwrap(), 6. * factor);
            let scaled_topology = scaled.topology_snapshot(owner, 2).unwrap();
            for intent in &intents {
                assert!(matches!(
                    resolve(intent, &scaled_topology, 2),
                    Resolution::Resolved { .. }
                ));
            }
            roundtrip(&scaled);
        }
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
        for output in [&common, &all, &again, &cut, &join, &tool_cut] {
            roundtrip(output);
        }
        #[cfg(feature = "occt")]
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("stock.step");
            stock.write_step(&path).unwrap();
            let native = Solid::read_step(&path, Default::default()).unwrap();
            let path = dir.path().join("tool.step");
            first.write_step(&path).unwrap();
            let native_tool = Solid::read_step(&path, Default::default()).unwrap();
            let reference = native
                .boolean(
                    &native_tool,
                    BooleanOperation::Intersect,
                    Default::default(),
                )
                .unwrap()
                .unwrap();
            compare(&reference, &common);
        }
    }
}

#[test]
fn common_material_distinguishes_empty_unsupported_cancellation_and_piece_limits() {
    let frame = PlaneFrame::default();
    let stock = tetra(frame);
    let tool = box_at(20., 1., frame);
    let before = stock.volume_mm3().unwrap();
    assert!(apply(&[&stock, &tool], BooleanOperation::Intersect).is_none());
    let error = Solid::boolean_planar_certified(
        &[&stock, &tool],
        BooleanOperation::Intersect,
        EntityId::new(),
        Default::default(),
        Some(&|| true),
    )
    .unwrap()
    .err()
    .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Cancelled);
    let sphere = Solid::rust_sphere_named(
        1.,
        PlaneFrame::default(),
        EntityId::new(),
        Default::default(),
    )
    .unwrap();
    assert!(
        Solid::boolean_planar_certified(
            &[&stock, &sphere],
            BooleanOperation::Intersect,
            EntityId::new(),
            Default::default(),
            None
        )
        .is_none()
    );
    let inputs = vec![&stock; 130];
    let error = Solid::boolean_planar_certified(
        &inputs,
        BooleanOperation::Intersect,
        EntityId::new(),
        Default::default(),
        None,
    )
    .unwrap()
    .err()
    .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Limit);
    near(stock.volume_mm3().unwrap(), before);
}

#[test]
fn concave_raw_and_labeled_loft_operands_keep_exposed_faces_not_partition_walls() {
    use multicad_core::references::{Origin, TopologyKind};
    let points = [
        [0., 0.],
        [6., 0.],
        [6., 5.],
        [4., 5.],
        [4., 2.],
        [2., 2.],
        [2., 5.],
        [0., 5.],
    ];
    let profile = |z| CurvedProfile {
        frame: PlaneFrame {
            origin: [0., 0., z],
            ..Default::default()
        },
        outer: points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    let owner = EntityId::new();
    let origins: Vec<_> = (0..8)
        .map(|edge| Origin {
            source: owner,
            role: format!("profile.edge.{edge}"),
        })
        .collect();
    let raw = Solid::rust_extrude_linear_named(
        &profile(0.),
        [1.5, -0.75, 4.],
        0.,
        owner,
        &origins,
        Default::default(),
    )
    .unwrap();
    let loft = Solid::rust_loft_ruled_polygons_named(
        &[profile(0.), profile(4.)],
        EntityId::new(),
        &[
            origins.clone(),
            origins
                .into_iter()
                .map(|mut origin| {
                    origin.role = format!("end.{}", origin.role);
                    origin
                })
                .collect(),
        ],
        Default::default(),
    )
    .unwrap();
    let tool = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [1, 1, 1].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [-0.2, 0.5, 0.5],
            u: [0.8, 0.6, 0.],
            v: [-0.6, 0.8, 0.],
        },
        Default::default(),
    )
    .unwrap();
    for source in [&raw, &loft] {
        for pose in [
            RigidTransform::default(),
            RigidTransform {
                rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
                translation_mm: [11., -7., 23.],
            },
        ] {
            let stock = source.transformed(pose).unwrap();
            let tool = tool.transformed(pose).unwrap();
            let source_topology = stock.topology_snapshot(owner, 0).unwrap();
            let tool_topology = tool.topology_snapshot(owner, 0).unwrap();
            let expected = stock
                .rust_planar_intersection_metrics(&tool, Default::default())
                .unwrap()
                .unwrap();
            let common = stock
                .boolean(&tool, BooleanOperation::Intersect, Default::default())
                .unwrap()
                .unwrap();
            near(common.volume_mm3().unwrap(), expected.0);
            let topology = common.topology_snapshot(owner, 1).unwrap();
            for face in topology
                .elements
                .iter()
                .filter(|face| face.key.kind == TopologyKind::Face)
            {
                assert!(
                    source_topology
                        .elements
                        .iter()
                        .chain(&tool_topology.elements)
                        .filter(|element| element.key.kind == TopologyKind::Face)
                        .any(|source| source.origins.is_subset(&face.origins)),
                    "lost authoritative exposed ancestry: {face:?}"
                );
                assert!(
                    face.origins
                        .iter()
                        .all(|origin| !origin.role.starts_with("planar-boolean.face."))
                );
            }
            roundtrip(&common);
            #[cfg(feature = "occt")]
            {
                let dir = tempfile::tempdir().unwrap();
                let a = dir.path().join("a.step");
                let b = dir.path().join("b.step");
                stock.write_step(&a).unwrap();
                tool.write_step(&b).unwrap();
                let reference = Solid::read_step(&a, Default::default())
                    .unwrap()
                    .boolean(
                        &Solid::read_step(&b, Default::default()).unwrap(),
                        BooleanOperation::Intersect,
                        Default::default(),
                    )
                    .unwrap()
                    .unwrap();
                compare(&reference, &common);
            }
        }
    }
}
