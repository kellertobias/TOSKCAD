use multicad_core::{
    document::EntityId,
    references::{Attachment, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::*;
fn policy() -> GeometryPolicy {
    Default::default()
}
fn profile() -> CurvedProfile {
    CurvedProfile {
        frame: Default::default(),
        outer: vec![
            CurvedEdge2d::Line {
                a: [0., 0.],
                b: [10., 0.],
            },
            CurvedEdge2d::Line {
                a: [10., 0.],
                b: [10., 6.],
            },
            CurvedEdge2d::Line {
                a: [10., 6.],
                b: [0., 6.],
            },
            CurvedEdge2d::Line {
                a: [0., 6.],
                b: [0., 0.],
            },
        ],
        holes: vec![],
    }
}
fn curves() -> Vec<Origin> {
    (0..4)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect()
}
fn origin(id: EntityId, role: &str) -> Origin {
    Origin {
        source: id,
        role: role.into(),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn prism(feature: EntityId, curves: &[Origin], height: f64) -> Solid {
    Solid::extrude_curved_profile_vector_named(
        &profile(),
        [0., 0., height],
        feature,
        curves,
        policy(),
    )
    .unwrap()
}
fn target(height: f64, slope: f64) -> PlaneFrame {
    let l = 1_f64.hypot(slope);
    PlaneFrame {
        origin: [0., 0., height],
        u: [1. / l, 0., slope / l],
        v: [0., 1., 0.],
    }
}
fn cap_frame(solid: &Solid, id: EntityId, role: &str) -> multicad_core::references::ReferenceFrame {
    let snapshot = solid.topology_snapshot(id, 0).unwrap();
    let origin = origin(id, role);
    let caps: Vec<_> = snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
        .collect();
    assert_eq!(caps.len(), 1, "ambiguous logical cap {role}");
    let mut intent = GeometryIntent::capture(&snapshot, caps[0].key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let Resolution::Resolved {
        frame: Some(frame), ..
    } = resolve(&intent, &snapshot, 0)
    else {
        panic!("cap frame unresolved")
    };
    frame
}
#[test]
fn role_swap_is_simultaneous_and_leaves_native_geometry_and_foreign_provenance_unchanged() {
    let id = EntityId::new();
    let foreign = EntityId::new();
    let mut curves = curves();
    curves[0] = origin(foreign, "cap.start");
    let solid = prism(id, &curves, -5.);
    let before = solid.topology_snapshot(id, 0).unwrap();
    let volume = solid.volume_mm3().unwrap();
    let bounds = solid.bounds().unwrap();
    let solid = solid
        .with_feature_cap_roles(id, "cap.end", "cap.start")
        .unwrap();
    near(solid.volume_mm3().unwrap(), volume);
    assert_eq!(solid.bounds().unwrap(), bounds);
    near(cap_frame(&solid, id, "cap.start").origin_mm[2], -5.);
    near(cap_frame(&solid, id, "cap.end").origin_mm[2], 0.);
    let after = solid.topology_snapshot(id, 0).unwrap();
    assert_eq!(
        before
            .elements
            .iter()
            .filter(|e| e.origins.contains(&curves[0]))
            .count(),
        after
            .elements
            .iter()
            .filter(|e| e.origins.contains(&curves[0]))
            .count()
    );
    let vertices = solid.display_vertices().unwrap();
    for e in after.elements.iter().filter(|e| {
        e.key.kind == TopologyKind::Vertex && e.origins.contains(&origin(id, "cap.start"))
    }) {
        near(
            vertices
                .iter()
                .find(|v| v.index == e.key.ordinal)
                .unwrap()
                .position_mm[2],
            -5.,
        );
        assert!(e.boundary.contains(&origin(id, "cap.start")));
    }
}
#[test]
fn two_sided_vector_union_has_only_genuine_far_start_and_end_caps() {
    let id = EntityId::new();
    let curves = curves();
    let forward = prism(id, &curves, 10.);
    let backward = prism(id, &curves, -5.)
        .with_feature_cap_roles(id, "cap.end", "cap.start")
        .unwrap();
    let result = forward
        .boolean(&backward, BooleanOperation::Union, policy())
        .unwrap()
        .unwrap();
    near(result.volume_mm3().unwrap(), 900.);
    assert_eq!(result.solid_count().unwrap(), 1);
    near(cap_frame(&result, id, "cap.start").origin_mm[2], -5.);
    near(cap_frame(&result, id, "cap.end").origin_mm[2], 10.);
}
#[test]
fn independent_sloping_terminations_keep_distinct_far_cap_origins_after_exact_union() {
    let id = EntityId::new();
    let curves = curves();
    let forward = Solid::extrude_curved_profile_to_plane_named(
        &profile(),
        [0., 0., 1.],
        target(10., 0.2),
        id,
        &curves,
        policy(),
    )
    .unwrap();
    let backward = Solid::extrude_curved_profile_to_plane_named(
        &profile(),
        [0., 0., -1.],
        target(-5., -0.1),
        id,
        &curves,
        policy(),
    )
    .unwrap()
    .with_feature_cap_roles(id, "cap.end", "cap.start")
    .unwrap();
    let result = forward
        .boolean(&backward, BooleanOperation::Union, policy())
        .unwrap()
        .unwrap();
    near(result.volume_mm3().unwrap(), 990.);
    assert_eq!(
        cap_frame(&result, id, "cap.start").origin_mm,
        target(-5., -0.1).origin
    );
    assert_eq!(
        cap_frame(&result, id, "cap.end").origin_mm,
        target(10., 0.2).origin
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two-independent-planes.step");
    result.write_step(&path).unwrap();
    near(
        Solid::read_step(&path, policy())
            .unwrap()
            .volume_mm3()
            .unwrap(),
        990.,
    );
}
#[test]
fn invalid_colliding_or_unowned_roles_cannot_relabel_arbitrary_geometry() {
    let id = EntityId::new();
    let curves = curves();
    for (start, end) in [
        ("", "cap.end"),
        (" ", "end"),
        ("same", "same"),
        ("cap.start", &"x".repeat(257)),
    ] {
        assert!(
            prism(id, &curves, 5.)
                .with_feature_cap_roles(id, start, end)
                .is_err()
        );
    }
    assert!(
        prism(id, &curves, 5.)
            .with_feature_cap_roles(EntityId::new(), "start", "end")
            .is_err()
    );
    let collision = format!("side/{}/{}", curves[0].source, curves[0].role);
    assert!(
        prism(id, &curves, 5.)
            .with_feature_cap_roles(id, &collision, "end")
            .is_err()
    );
    let mut ambiguous = curves.clone();
    ambiguous[0] = origin(id, "cap.start");
    assert!(
        prism(id, &ambiguous, 5.)
            .with_feature_cap_roles(id, "end", "start")
            .is_err()
    );
    let custom = prism(id, &curves, 5.)
        .with_feature_cap_roles(id, "far.a", "far.b")
        .unwrap();
    near(cap_frame(&custom, id, "far.a").origin_mm[2], 0.);
    near(cap_frame(&custom, id, "far.b").origin_mm[2], 5.);
}
