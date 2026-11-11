use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{rust_kernel::Rational, *};

fn box_at(owner: EntityId, origin: [f64; 3], sizes: [i64; 3]) -> Solid {
    Solid::rust_box(
        owner.uuid_bits(),
        sizes.map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn cut(stock: &Solid, tool: EntityId, feature: EntityId, x: f64, width: i64) -> Solid {
    stock
        .rust_subtract_planar_tools(
            &[&box_at(tool, [x, 0., 5.], [width, 20, 5])],
            feature,
            GeometryPolicy::default(),
        )
        .unwrap()
        .unwrap()
}
fn top_intent(solid: &Solid, owner: EntityId, x: f64) -> GeometryIntent {
    let face = solid
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| (f.center_mm[0] - x).abs() < 1e-8 && (f.center_mm[2] - 10.).abs() < 1e-8)
        .unwrap();
    let snapshot = solid.topology_snapshot(owner, 1).unwrap();
    let key = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == face.index)
        .unwrap()
        .key;
    GeometryIntent::capture(&snapshot, key).unwrap()
}
fn resolved_center(intent: &GeometryIntent, solid: &Solid, owner: EntityId) -> Option<[f64; 3]> {
    let after = solid.topology_snapshot(owner, 2).unwrap();
    match resolve(intent, &after, 2) {
        Resolution::Resolved { key, .. } => Some(
            solid
                .faces()
                .unwrap()
                .into_iter()
                .find(|f| f.index == key.ordinal)
                .unwrap()
                .center_mm,
        ),
        Resolution::Missing | Resolution::Ambiguous { .. } => None,
        other => panic!("unexpected resolution {other:?}"),
    }
}
#[test]
fn deleted_split_patch_never_rebinds_to_opposite_finite_face() {
    let (owner, tool, feature) = (EntityId::new(), EntityId::new(), EntityId::new());
    let stock = box_at(owner, [0.; 3], [30, 20, 10]);
    let initial = cut(&stock, tool, feature, 10., 10);
    let left = top_intent(&initial, owner, 5.);
    let right = top_intent(&initial, owner, 25.);
    let snapshot = initial.topology_snapshot(owner, 1).unwrap();
    for face in initial
        .faces()
        .unwrap()
        .into_iter()
        .filter(|f| (f.center_mm[2] - 10.).abs() < 1e-7)
    {
        let wires = initial
            .planar_face_contours_with_sources(
                face.index,
                PlaneFrame {
                    origin: [0., 0., 10.],
                    ..Default::default()
                },
            )
            .unwrap();
        for key in wires.edge_keys.into_iter().flatten() {
            GeometryIntent::capture(&snapshot, key).unwrap();
        }
    }
    assert!(
        left.origins
            .iter()
            .any(|o| o.role.starts_with("derived-fragment.v1."))
    );
    assert!(
        right
            .origins
            .iter()
            .any(|o| o.role.starts_with("derived-fragment.v1."))
    );
    assert!(
        left.origins
            .iter()
            .all(|o| !o.role.starts_with("planar-boolean.face."))
    );
    for (x, kept, gone, center) in [(0., &right, &left, 25.), (10., &left, &right, 5.)] {
        let changed = cut(&stock, tool, feature, x, 20);
        assert_eq!(
            resolved_center(kept, &changed, owner),
            Some([center, 10., 10.])
        );
        assert_eq!(
            resolved_center(gone, &changed, owner),
            None,
            "deleted patch rebound to another finite region"
        );
    }
}

#[test]
fn scaling_a_split_boolean_preserves_fragment_discrimination_after_source_edits() {
    let (owner, tool, feature, scale) = (
        EntityId::new(),
        EntityId::new(),
        EntityId::new(),
        EntityId::new(),
    );
    let stock = box_at(owner, [0.; 3], [30, 20, 10]);
    let apply = |body: &Solid| {
        body.affine_transformed_named(
            AffineTransform {
                matrix: [[1.2, 0., 0.], [0., 1.3, 0.], [0., 0., 1.4]],
                translation_mm: [0.; 3],
            },
            scale,
            GeometryPolicy::default(),
        )
        .unwrap()
    };
    let initial = apply(&cut(&stock, tool, feature, 10., 10));
    let before = initial.topology_snapshot(owner, 1).unwrap();
    let intent = |x: f64| {
        let f = initial
            .faces()
            .unwrap()
            .into_iter()
            .find(|f| (f.center_mm[0] - x).abs() < 1e-8 && (f.center_mm[2] - 14.).abs() < 1e-8)
            .unwrap();
        let e = before
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == f.index)
            .unwrap();
        GeometryIntent::capture(&before, e.key).unwrap()
    };
    let (left, right) = (intent(6.), intent(30.));
    assert!(
        left.origins
            .iter()
            .any(|o| o.role.starts_with("derived-fragment.v1."))
    );
    assert!(
        right
            .origins
            .iter()
            .any(|o| o.role.starts_with("derived-fragment.v1."))
    );
    for (x, kept, gone, center) in [(0., &right, &left, 30.), (10., &left, &right, 6.)] {
        let changed = apply(&cut(&stock, tool, feature, x, 20));
        let resolved = resolved_center(kept, &changed, owner).unwrap();
        for (a, b) in resolved.into_iter().zip([center, 13., 14.]) {
            assert!((a - b).abs() < 1e-8);
        }
        assert_eq!(resolved_center(gone, &changed, owner), None);
    }
}

#[test]
fn uniquely_remaining_trimmed_patch_does_not_rebind_when_replaced_by_opposite_patch() {
    let (owner, tool, feature) = (EntityId::new(), EntityId::new(), EntityId::new());
    let stock = box_at(owner, [0.; 3], [30, 20, 10]);
    let right_only = cut(&stock, tool, feature, 0., 20);
    let right = top_intent(&right_only, owner, 25.);
    let left_only = cut(&stock, tool, feature, 10., 20);
    assert_eq!(
        resolved_center(&right, &left_only, owner),
        None,
        "a unique trimmed face is not its opposite replacement"
    );
}

#[test]
fn derived_partition_references_captured_after_scale_survive_factor_edits_and_rigid_pose() {
    let (owner, tool, feature, scale) = (
        EntityId::new(),
        EntityId::new(),
        EntityId::new(),
        EntityId::new(),
    );
    let points = [
        [0., 0.],
        [30., 0.],
        [30., 20.],
        [18., 20.],
        [18., 15.],
        [12., 15.],
        [12., 20.],
        [0., 20.],
    ];
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    let origins: Vec<_> = (0..8)
        .map(|i| Origin {
            source: owner,
            role: format!("source.edge.{i}"),
        })
        .collect();
    let stock = Solid::rust_extrude_linear_named(
        &profile,
        [0., 0., 10.],
        0.,
        owner,
        &origins,
        GeometryPolicy::default(),
    )
    .unwrap();
    let tool = box_at(tool, [0.; 3], [10, 60, 5])
        .transformed(RigidTransform {
            rotation: [[0.8, -0.6, 0.], [0.6, 0.8, 0.], [0., 0., 1.]],
            translation_mm: [25., -20., 5.],
        })
        .unwrap();
    let pocket = stock
        .rust_subtract_planar_tools(&[&tool], feature, GeometryPolicy::default())
        .unwrap()
        .unwrap();
    let scaled = |factor: f64| {
        pocket
            .affine_transformed_named(
                AffineTransform {
                    matrix: [[factor, 0., 0.], [0., 1.3, 0.], [0., 0., 1.4]],
                    translation_mm: [0.; 3],
                },
                scale,
                GeometryPolicy::default(),
            )
            .unwrap()
    };
    let initial = scaled(1.2);
    let before = initial.topology_snapshot(owner, 1).unwrap();
    assert!(
        before
            .elements
            .iter()
            .flat_map(|e| &e.origins)
            .any(|o| o.role.starts_with("derived-fragment.v1."))
    );
    let changed = scaled(2.);
    let posed = changed
        .transformed(RigidTransform {
            rotation: [[0.8, -0.6, 0.], [0.6, 0.8, 0.], [0., 0., 1.]],
            translation_mm: [4., 8., 12.],
        })
        .unwrap();
    for solid in [&changed, &posed] {
        let after = solid.topology_snapshot(owner, 2).unwrap();
        for e in &before.elements {
            let intent = GeometryIntent::capture(&before, e.key).unwrap();
            let Resolution::Resolved { key, .. } = resolve(&intent, &after, 2) else {
                panic!("lost post-Scale identity {intent:?}")
            };
            assert_eq!(key, e.key);
            let current = after.elements.iter().find(|v| v.key == key).unwrap();
            assert_eq!(current.origins, e.origins);
            assert_eq!(current.boundary, e.boundary);
        }
    }
}

#[test]
fn split_merge_and_source_dimension_edits_do_not_guess_a_fragment() {
    let (owner, tool, feature) = (EntityId::new(), EntityId::new(), EntityId::new());
    let stock = box_at(owner, [0.; 3], [30, 20, 10]);
    let before = stock.topology_snapshot(owner, 1).unwrap();
    let cap = before
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == "cap.end"))
        .unwrap();
    let source_cap = GeometryIntent::capture(&before, cap.key).unwrap();
    assert!(source_cap.boundary.is_none());
    let split = cut(&stock, tool, feature, 10., 10);
    assert!(matches!(
        resolve(&source_cap, &split.topology_snapshot(owner, 2).unwrap(), 2),
        Resolution::Ambiguous { .. }
    ));
    let left = top_intent(&split, owner, 5.);
    let right = top_intent(&split, owner, 25.);
    let wider = cut(
        &box_at(owner, [0.; 3], [40, 20, 10]),
        tool,
        feature,
        10.,
        10,
    );
    assert_eq!(resolved_center(&left, &wider, owner), Some([5., 10., 10.]));
    assert_eq!(
        resolved_center(&right, &wider, owner),
        Some([30., 10., 10.])
    );
    // A merge discards the old separator; fragment references require repair.
    let merged = cut(&stock, tool, feature, 30., 10);
    assert_eq!(resolved_center(&left, &merged, owner), None);
    assert_eq!(resolved_center(&right, &merged, owner), None);
    // An ordinary unique surviving surface is not locked to its old neighbors.
    let bottom = split
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| f.center_mm[2].abs() < 1e-8)
        .unwrap();
    let snapshot = split.topology_snapshot(owner, 1).unwrap();
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == bottom.index)
        .unwrap();
    let bottom = GeometryIntent::capture(&snapshot, element.key).unwrap();
    assert!(bottom.boundary.is_none());
    assert_eq!(
        resolved_center(&bottom, &wider, owner),
        Some([20., 10., 0.])
    );
    assert!((split.volume_mm3().unwrap() - 5000.).abs() < 1e-7);
    split
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("derived.step");
    split.write_step(&path).unwrap();
    let read = Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    assert!((read.volume_mm3().unwrap() - 5000.).abs() < 1e-7);
    if OCCT_LINKED {
        assert!(
            (Solid::read_step(&path, GeometryPolicy::default())
                .unwrap()
                .volume_mm3()
                .unwrap()
                - 5000.)
                .abs()
                < 1e-6
        );
    }
}
