use multicad_core::document::EntityId;
use multicad_kernel::{
    BooleanOperation, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform, Solid,
};
use std::collections::BTreeMap;
fn closed(mesh: &multicad_kernel::Mesh) {
    let mut edges = BTreeMap::new();
    for t in mesh.indices.as_chunks::<3>().0 {
        assert_ne!(t[0], t[1]);
        assert_ne!(t[1], t[2]);
        assert_ne!(t[2], t[0]);
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            let entry = edges.entry((a.min(b), a.max(b))).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += if a < b { 1 } else { -1 };
        }
    }
    assert!(edges.values().all(|v| *v == (2, 0)));
}
#[test]
fn manufacturing_shared_native_edges_close_fused_blind_holes_without_mutating_operands() {
    let policy = GeometryPolicy::default();
    let plate = Solid::box_solid([43., 86., 4.], policy).unwrap();
    let plug = Solid::box_solid([39.7, 82.4, 16.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [3.3, 1.8, 4.],
            ..Default::default()
        })
        .unwrap();
    let mut solid = plate
        .boolean(&plug, BooleanOperation::Union, policy)
        .unwrap()
        .unwrap();
    for y in [0.8, 76.2] {
        let tool = Solid::cylinder(
            PlaneFrame {
                origin: [23., y, 12.],
                u: [0., 0., 1.],
                v: [1., 0., 0.],
            },
            2.,
            9.,
            policy,
        )
        .unwrap();
        solid = solid
            .boolean(&tool, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap();
    }
    let solid = solid
        .transformed(RigidTransform {
            rotation: [[0.36, -0.8, 0.48], [0.48, 0.6, 0.64], [-0.8, 0., 0.6]],
            translation_mm: [19., -23., 41.],
        })
        .unwrap();
    let options = MeshOptions {
        linear_deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
        maximum_triangles: 100_000,
    };
    let owner = EntityId::new();
    let source = solid.topology_snapshot(owner, 4).unwrap();
    let bounds = solid.bounds().unwrap();
    let volume = solid.volume_mm3().unwrap();
    let raw = solid.mesh(options).unwrap();
    let receipt = solid.manufacturing_mesh_with_receipt(options).unwrap();
    closed(&receipt.mesh);
    assert_eq!(raw.indices.len(), receipt.mesh.indices.len());
    assert_eq!(raw.face_indices, receipt.mesh.face_indices);
    assert!(receipt.edge_pairs > 0 && receipt.vertex_links > 0);
    assert_eq!(receipt.removed_native_pole_triangles, 0);
    assert!(receipt.maximum_displacement_mm <= policy.linear_tolerance_mm);
    eprintln!(
        "manufacturing seam maximum_displacement_mm={:.17} edge_pairs={} vertex_links={}",
        receipt.maximum_displacement_mm, receipt.edge_pairs, receipt.vertex_links
    );
    assert_eq!(source, solid.topology_snapshot(owner, 4).unwrap());
    assert_eq!(bounds, solid.bounds().unwrap());
    assert_eq!(volume, solid.volume_mm3().unwrap());
    assert_eq!(raw, solid.mesh(options).unwrap());
}
#[test]
fn manufacturing_periodic_and_compound_topology_preserve_distinct_native_shells() {
    let p = GeometryPolicy::default();
    let options = MeshOptions {
        linear_deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
        maximum_triangles: 100_000,
    };
    let cylinder = Solid::cylinder(PlaneFrame::default(), 3., 5., p).unwrap();
    closed(&cylinder.manufacturing_mesh(options).unwrap());
    let other = cylinder
        .transformed(RigidTransform {
            translation_mm: [10., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let pair = cylinder
        .boolean(&other, BooleanOperation::Union, p)
        .unwrap()
        .unwrap();
    assert_eq!(pair.solid_count().unwrap(), 2);
    closed(&pair.manufacturing_mesh(options).unwrap());
    let mut bad = options;
    bad.maximum_triangles = 0;
    assert!(pair.manufacturing_mesh(bad).is_err());
}
#[test]
fn manufacturing_sphere_poles_and_torus_double_periodic_seams_are_closed() {
    use multicad_core::references::Origin;
    use multicad_kernel::{CurvedEdge2d, CurvedProfile, RevolutionAxis};
    let options = MeshOptions {
        linear_deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
        maximum_triangles: 100_000,
    };
    for torus in [false, true] {
        let id = EntityId::new();
        let outer = if torus {
            // Use the supported complete-circle source from revolve.rs rather
            // than two tangent arcs, which the source-wire guard refuses.
            vec![CurvedEdge2d::Circle {
                center: [3., 0.],
                radius_mm: 1.,
            }]
        } else {
            vec![
                CurvedEdge2d::Arc {
                    center: [0., 0.],
                    a: [0., -3.],
                    b: [0., 3.],
                    clockwise: false,
                },
                CurvedEdge2d::Line {
                    a: [0., 3.],
                    b: [0., -3.],
                },
            ]
        };
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 0., 1.],
            },
            outer,
            holes: vec![],
        };
        let origins = [
            Origin {
                source: id,
                role: "profile/first".into(),
            },
            Origin {
                source: id,
                role: "profile/second".into(),
            },
        ];
        let shape = Solid::revolve_curved_profile_named(
            &profile,
            RevolutionAxis {
                origin_mm: [0.; 3],
                direction: [0., 0., 1.],
            },
            std::f64::consts::TAU,
            id,
            &origins[..profile.outer.len()],
            Default::default(),
        )
        .unwrap();
        let before = shape.topology_snapshot(id, 0).unwrap();
        let receipt = shape.manufacturing_mesh_with_receipt(options).unwrap();
        closed(&receipt.mesh);
        let raw = shape.mesh(options).unwrap();
        assert_eq!(
            raw.indices.len() / 3,
            receipt.mesh.indices.len() / 3
                + receipt.removed_native_pole_triangles as usize
                + receipt.removed_exact_zero_triangles as usize
        );
        if torus {
            assert_eq!(receipt.removed_native_pole_triangles, 0);
        } else {
            assert!(receipt.removed_native_pole_triangles > 0);
        }
        eprintln!(
            "manufacturing torus={torus} removed_native_pole_triangles={}",
            receipt.removed_native_pole_triangles
        );
        assert_eq!(before, shape.topology_snapshot(id, 0).unwrap());
        assert!(receipt.maximum_displacement_mm <= 1e-7);
    }
}
#[test]
fn native_buttress_internal_constraints_are_not_cross_face_mesh_seams() {
    use multicad_core::{
        commands::Session,
        document::Document,
        model::{self, ButtressLoadFlankSide},
    };
    use multicad_kernel::{HelicalThread, RootRoundedHelicalThread};
    let session = Session::new(Document::new("Internal buttress seam").unwrap()).unwrap();
    for hand in [model::ThreadHand::Right, model::ThreadHand::Left] {
        let mut parameters = model::buttress_thread_presets()
            .remove(0)
            .parameters(ButtressLoadFlankSide::PositiveAxisSide);
        parameters.hand = hand;
        parameters.length = model::length(13.48);
        parameters.radial_allowance = model::length(0.005);
        let r = parameters
            .resolve(session.parameters(), model::ThreadMode::NewExternal)
            .unwrap();
        let profile = r.profile;
        let id = EntityId::new();
        let shape = Solid::root_rounded_helical_thread_named(
            RootRoundedHelicalThread {
                profile: HelicalThread {
                    major_radius_mm: r.major_radius_mm,
                    pitch_mm: r.pitch_mm,
                    length_mm: r.length_mm,
                    depth_mm: profile.depth_mm,
                    crest_width_mm: profile.crest_width_mm,
                    lower_flank_deg: profile.lower_flank_deg,
                    upper_flank_deg: profile.upper_flank_deg,
                    radial_taper: 0.,
                    starts: 1,
                    left_hand: hand == model::ThreadHand::Left,
                },
                root_radius_mm: r
                    .buttress
                    .as_ref()
                    .unwrap()
                    .root_circle
                    .as_ref()
                    .unwrap()
                    .radius_mm,
            },
            id,
            Default::default(),
        )
        .unwrap()
        .transformed(Default::default())
        .unwrap();
        let source_brep = shape.to_brep().unwrap();
        let before = shape.topology_snapshot(id, 3).unwrap();
        let options = MeshOptions {
            linear_deflection_mm: 0.003,
            angular_deflection_rad: 5_f64.to_radians(),
            maximum_triangles: 2_000_000,
        };
        let raw = shape.mesh(options).unwrap();
        let receipt = shape.manufacturing_mesh_with_receipt(options).unwrap();
        closed(&receipt.mesh);
        assert!(
            receipt.internal_constraint_edges > 0 || receipt.removed_meshing_internal_edges > 0,
            "fixture must exercise native INTERNAL constraints"
        );
        assert!(receipt.maximum_displacement_mm <= 1e-7);
        if receipt.remeshed_without_internal_constraints {
            assert!(receipt.removed_meshing_internal_edges > 0);
            assert!(receipt.removed_meshing_constraint_wire_containers > 0);
        } else {
            assert_eq!(
                raw.indices.len() / 3,
                receipt.mesh.indices.len() / 3
                    + receipt.removed_native_pole_triangles as usize
                    + receipt.removed_exact_zero_triangles as usize
            );
        }
        assert_eq!(source_brep, shape.to_brep().unwrap());
        assert_eq!(before, shape.topology_snapshot(id, 3).unwrap());
        assert_eq!(raw, shape.mesh(options).unwrap());
        eprintln!(
            "buttress manufacturing internal_constraints={} exact_duplicates={} removed_exact_zero={} removed_pole={} maximum_displacement_mm={:.17}",
            receipt.internal_constraint_edges,
            receipt.exact_vertex_pairs,
            receipt.removed_exact_zero_triangles,
            receipt.removed_native_pole_triangles,
            receipt.maximum_displacement_mm
        );
    }
}
