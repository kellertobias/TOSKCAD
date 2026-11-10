use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, MeshOptions,
    PlaneFrame, RigidTransform, Solid,
    rust_kernel::{
        Circle, Rational, cylinder::AxialCylinder, display::DisplayMesh, manufacturing::certify,
    },
};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn options(chord: f64) -> MeshOptions {
    MeshOptions {
        linear_deflection_mm: chord,
        angular_deflection_rad: 0.9,
        maximum_triangles: 100_000,
    }
}

fn mesh_proof(
    receipt: &multicad_kernel::ManufacturingMesh,
) -> multicad_kernel::rust_kernel::manufacturing::MeshProof {
    let mesh = &receipt.mesh;
    let proof = certify(&DisplayMesh {
        vertices: mesh.vertices.clone(),
        indices: mesh.indices.clone(),
        face_indices: mesh.face_indices.clone(),
    })
    .unwrap();
    assert_eq!(receipt.edge_pairs, proof.edge_pairs);
    assert_eq!(receipt.vertex_links, proof.vertex_links);
    assert_eq!(receipt.maximum_displacement_mm, 0.);
    proof
}

fn stock(side: i64, origin: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [n(side); 3],
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}

#[test]
fn rust_cylinder_manufacturing_respects_chord_quality_after_rigid_placement() {
    let radius = 5.;
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([n(0); 2], n(5)).unwrap(),
        n(0),
        n(3),
    )
    .unwrap();
    let source =
        Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), GeometryPolicy::default())
            .unwrap();
    let coarse = source
        .manufacturing_mesh_with_receipt(options(0.1))
        .unwrap();
    let fine = source
        .manufacturing_mesh_with_receipt(options(0.01))
        .unwrap();
    assert!(fine.mesh.indices.len() > coarse.mesh.indices.len());
    let coarse_volume = mesh_proof(&coarse).signed_volume_mm3;
    let fine_volume = mesh_proof(&fine).signed_volume_mm3;
    assert!(coarse_volume < fine_volume && fine_volume < source.volume_mm3().unwrap());
    let vertices = fine.mesh.vertices.as_chunks::<3>().0;
    let mut rim_edges = 0;
    for (&face, triangle) in fine
        .mesh
        .face_indices
        .iter()
        .zip(fine.mesh.indices.as_chunks::<3>().0)
    {
        if face != 2 {
            continue;
        }
        let [a, b, c] = triangle.map(|index| vertices[index as usize]);
        for (a, b) in [(a, b), (b, c), (c, a)] {
            if a[2] == b[2] {
                rim_edges += 1;
                let midpoint_radius = ((a[0] + b[0]) / 2.).hypot((a[1] + b[1]) / 2.);
                assert!(radius - midpoint_radius <= 0.01 + 1e-12);
            }
        }
    }
    assert!(rim_edges > 20);
    let tilted = source
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 0.8, -0.6], [0., 0.6, 0.8]],
            translation_mm: [100., -25., 42.],
        })
        .unwrap();
    let moved = tilted
        .manufacturing_mesh_with_receipt(options(0.01))
        .unwrap();
    assert_eq!(moved.mesh.indices, fine.mesh.indices);
    assert!((mesh_proof(&moved).signed_volume_mm3 - fine_volume).abs() < 1e-9);
    assert_eq!(
        source
            .manufacturing_mesh(MeshOptions {
                maximum_triangles: 3,
                ..options(0.01)
            })
            .unwrap_err()
            .kind,
        GeometryErrorKind::Limit
    );
}

#[test]
fn rust_orthogonal_cavity_keeps_shared_indices_and_inward_inner_walls() {
    let source = stock(6, [0.; 3]);
    let cavity = stock(2, [2.; 3]);
    let solid = source
        .boolean(
            &cavity,
            BooleanOperation::Subtract,
            GeometryPolicy::default(),
        )
        .unwrap()
        .unwrap();
    let receipt = solid
        .manufacturing_mesh_with_receipt(options(0.03))
        .unwrap();
    assert!(receipt.mesh.vertices.len() < solid.mesh(options(0.03)).unwrap().vertices.len());
    assert_eq!(mesh_proof(&receipt).signed_volume_mm3, 208.);
    assert_eq!(
        receipt
            .mesh
            .face_indices
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        12
    );
}

#[test]
fn rust_batched_cuts_match_native_material_and_keep_each_tool_ancestry() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let rust_box = |id: EntityId, size: [i64; 3], origin| {
        Solid::rust_box(
            id.uuid_bits(),
            size.map(n),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let native_box = |size: [i64; 3], translation_mm| {
        Solid::box_solid(size.map(|axis| axis as f64), policy)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm,
                ..Default::default()
            })
            .unwrap()
    };
    let source = rust_box(owner, [10; 3], [0.; 3]);
    let source_topology = source.topology_snapshot(owner, 0).unwrap();
    let source_top = source_topology
        .elements
        .iter()
        .find(|face| {
            matches!(face.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal() == [0., 0., 1.])
        })
        .unwrap();
    let source_intent = GeometryIntent::capture(&source_topology, source_top.key).unwrap();
    let inputs = [
        ([3, 3, 12], [1., 1., -1.]),
        ([3, 3, 12], [3., 2., -1.]),
        ([2, 3, 4], [7., 6., 8.]),
    ];
    let ids = inputs.map(|_| EntityId::new());
    let mut cutters =
        std::array::from_fn::<_, 3, _>(|i| rust_box(ids[i], inputs[i].0, inputs[i].1));
    let result = source
        .rust_subtract_orthogonal_tools(&cutters, policy)
        .unwrap()
        .unwrap();
    let native =
        inputs
            .into_iter()
            .fold(native_box([10; 3], [0.; 3]), |current, (size, origin)| {
                current
                    .boolean(
                        &native_box(size, origin),
                        BooleanOperation::Subtract,
                        policy,
                    )
                    .unwrap()
                    .unwrap()
            });
    assert_eq!(result.solid_count().unwrap(), 1);
    assert_eq!(native.solid_count().unwrap(), 1);
    assert_eq!(result.volume_mm3().unwrap(), 828.);
    assert!((result.volume_mm3().unwrap() - native.volume_mm3().unwrap()).abs() < 1e-8);
    assert_eq!(result.faces().unwrap().len(), native.faces().unwrap().len());
    assert_eq!(
        mesh_proof(
            &result
                .manufacturing_mesh_with_receipt(options(0.03))
                .unwrap()
        )
        .signed_volume_mm3,
        828.
    );

    let topology = result.topology_snapshot(owner, 1).unwrap();
    assert!(matches!(
        resolve(&source_intent, &topology, 1),
        Resolution::Resolved { .. }
    ));
    for cutter in ids {
        assert!(topology.elements.iter().any(|face| {
            face.key.kind == TopologyKind::Face
                && face.origins.iter().any(|origin| origin.source == cutter)
                && face
                    .frames
                    .iter()
                    .any(|anchor| anchor.origin.source == cutter)
        }));
    }
    let pocket_floor = topology
        .elements
        .iter()
        .find(|face| {
            matches!(face.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.origin_mm[2] == 8. && frame.normal() == [0., 0., 1.])
        })
        .unwrap();
    assert!(
        pocket_floor.frames.iter().any(|anchor| {
            anchor.origin.source == ids[2] && anchor.frame.normal() == [0., 0., 1.]
        })
    );
    cutters.reverse();
    let reordered = source
        .rust_subtract_orthogonal_tools(&cutters, policy)
        .unwrap()
        .unwrap();
    assert_eq!(reordered.topology_snapshot(owner, 1).unwrap(), topology);
}

#[test]
fn rust_step_rebuilds_placed_orthogonal_stock_with_blind_and_through_cuts() {
    let policy = GeometryPolicy::default();
    let stock = stock(10, [0.; 3]);
    let box_at = |size: [i64; 3], origin| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(n),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let milled = stock
        .rust_subtract_orthogonal_tools(
            &[
                box_at([2, 2, 12], [1., 1., -1.]),
                box_at([3, 2, 5], [6., 6., 7.]),
            ],
            policy,
        )
        .unwrap()
        .unwrap();
    let [a, b] = [0.37_f64, -0.61_f64];
    let (sa, ca) = a.sin_cos();
    let (sb, cb) = b.sin_cos();
    let placement = RigidTransform {
        rotation: [
            [ca * cb, -sa, ca * sb],
            [sa * cb, ca, sa * sb],
            [-sb, 0., cb],
        ],
        translation_mm: [14_000., -9_700., 600.],
    };
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("machined.step");
    let owner = EntityId::new();
    let near = |a: f64, b: f64| assert!((a - b).abs() <= 1e-6, "{a} != {b}");
    for (index, shape) in [
        milled.share().unwrap(),
        milled.transformed(placement).unwrap(),
    ]
    .into_iter()
    .enumerate()
    {
        shape.write_step(&path).unwrap();
        let native = Solid::read_step(&path, policy).unwrap();
        near(native.volume_mm3().unwrap(), 942.);
        let text = std::fs::read_to_string(&path).unwrap();
        let rust = Solid::import_rust_step(&text, owner, policy).unwrap();
        assert!(rust.uses_rust_geometry());
        let bounds = shape.bounds().unwrap();
        let rebuilt = rust.bounds().unwrap();
        for (actual, expected) in rebuilt
            .min
            .into_iter()
            .chain(rebuilt.max)
            .zip(bounds.min.into_iter().chain(bounds.max))
        {
            near(actual, expected);
        }
        let receipt = rust.manufacturing_mesh_with_receipt(options(0.03)).unwrap();
        near(mesh_proof(&receipt).signed_volume_mm3, 942.);
        let topology = rust.topology_snapshot(owner, 0).unwrap();
        let top = topology
            .elements
            .iter()
            .find(|e| {
                e.key.kind == TopologyKind::Face
                    && e.frames.first().is_some_and(|anchor| {
                        let frame = anchor.frame;
                        rust.planar_face_contours(
                            e.key.ordinal,
                            PlaneFrame {
                                origin: frame.origin_mm,
                                u: frame.u,
                                v: frame.v,
                            },
                        )
                        .is_ok_and(|c| c.loops.len() == 3)
                    })
            })
            .expect("top face retains both pocket and through-hole contours");
        let intent = GeometryIntent::capture(&topology, top.key).unwrap();
        let reopened = Solid::import_rust_step(&text, owner, policy).unwrap();
        assert!(
            matches!(resolve(&intent, &reopened.topology_snapshot(owner, 1).unwrap(), 1),
            Resolution::Resolved { key, .. } if key == top.key)
        );
        if index == 0 {
            let edited = reopened
                .rust_subtract_orthogonal_tools(&[box_at([2, 2, 12], [5., 1., -1.])], policy)
                .unwrap()
                .unwrap();
            near(edited.volume_mm3().unwrap(), 902.);
            assert!(matches!(
                resolve(&intent, &edited.topology_snapshot(owner, 2).unwrap(), 2),
                Resolution::Resolved { .. }
            ));
        }
        let original = shape.faces().unwrap();
        for face in rust.faces().unwrap() {
            assert!(original.iter().any(|source| {
                (source.area_mm2 - face.area_mm2).abs() < 1e-6
                    && source
                        .center_mm
                        .into_iter()
                        .zip(face.center_mm)
                        .all(|(a, b)| (a - b).abs() < 1e-6)
            }));
        }
    }
}

#[test]
fn rust_batched_cuts_keep_empty_and_refused_outcomes_distinct() {
    let policy = GeometryPolicy::default();
    let source = stock(6, [0.; 3]);
    assert_eq!(
        source
            .rust_subtract_orthogonal_tools(&[], policy)
            .unwrap()
            .unwrap()
            .volume_mm3()
            .unwrap(),
        216.
    );
    assert!(
        source
            .rust_subtract_orthogonal_tools(&[stock(8, [-1.; 3])], policy)
            .unwrap()
            .is_none()
    );

    let invalid_tools = [
        (
            Solid::box_solid([2.; 3], policy).unwrap(),
            GeometryErrorKind::Unsupported,
        ),
        (
            stock(2, [0.5 * policy.linear_tolerance_mm, 2., 2.]),
            GeometryErrorKind::InvalidResult,
        ),
    ];
    for (tool, kind) in invalid_tools {
        let result = source.rust_subtract_orthogonal_tools(&[stock(1, [2.; 3]), tool], policy);
        assert!(matches!(result, Err(error) if error.kind == kind));
    }
    let fuzzy = GeometryPolicy {
        boolean_fuzzy_mm: policy.linear_tolerance_mm,
        ..policy
    };
    let result = source.rust_subtract_orthogonal_tools(&[], fuzzy);
    assert!(matches!(result, Err(error) if error.kind == GeometryErrorKind::Unsupported));
    assert_eq!(source.volume_mm3().unwrap(), 216.);
}

#[test]
fn rust_batched_cuts_accept_exact_float_rectangles_without_rational_products() {
    let policy = GeometryPolicy::default();
    let source = stock(8, [0.; 3]);
    let frame = PlaneFrame {
        origin: [3., 2., -1.],
        u: [0., 1., 0.],
        v: [1., 0., 0.],
    };
    let [x0, x1] = [0.123_456_789_012_345_6, 2.987_654_321_098_765];
    let [y0, y1] = [0.567_890_123_456_789, 3.876_543_210_987_654];
    let rectangle = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let prism = |points: [[f64; 2]; 4], displacement, taper| {
        let profile = CurvedProfile {
            frame,
            outer: (0..4)
                .map(|i| CurvedEdge2d::Line {
                    a: points[i],
                    b: points[(i + 1) % 4],
                })
                .collect(),
            holes: vec![],
        };
        let feature = EntityId::new();
        let origins = std::array::from_fn::<_, 4, _>(|i| Origin {
            source: feature,
            role: format!("cutter.{i}"),
        });
        Solid::rust_extrude_linear_named(&profile, displacement, taper, feature, &origins, policy)
            .unwrap()
    };
    let exact = prism(rectangle, [0., 0., 10.], 0.);
    let output = source
        .rust_subtract_orthogonal_tools(&[exact], policy)
        .unwrap()
        .unwrap();
    let expected = 512. - 8. * (x1 - x0) * (y1 - y0);
    assert!((output.volume_mm3().unwrap() - expected).abs() < 1e-10);
    let mesh = output
        .manufacturing_mesh_with_receipt(options(0.03))
        .unwrap();
    assert!((mesh_proof(&mesh).signed_volume_mm3 - expected).abs() < 1e-10);
    assert_eq!(output.faces().unwrap().len(), 10);

    let mut trapezoid = rectangle;
    trapezoid[3][0] += 0.1;
    for tool in [
        prism(trapezoid, [0., 0., 10.], 0.),
        prism(rectangle, [0.1, 0., 10.], 0.),
        prism(rectangle, [0., 0., 10.], 0.1),
    ] {
        assert!(matches!(
            source.rust_subtract_orthogonal_tools(&[tool], policy),
            Err(error) if error.kind == GeometryErrorKind::Unsupported
        ));
    }
}

#[test]
fn rust_manufacturing_keeps_coincident_assembly_member_indices_separate() {
    let source = stock(3, [0.; 3]);
    let compound = Solid::compound_named(
        &[&source, &source],
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let receipt = compound
        .manufacturing_mesh_with_receipt(options(0.03))
        .unwrap();
    assert_eq!(receipt.mesh.vertices.len() / 3, 16);
    assert_eq!(receipt.mesh.face_indices.len(), 24);
    assert_eq!(mesh_proof(&receipt).signed_volume_mm3, 54.);
    assert!(receipt.mesh.indices[..36].iter().all(|&index| index < 8));
    assert!(receipt.mesh.indices[36..].iter().all(|&index| index >= 8));
}
