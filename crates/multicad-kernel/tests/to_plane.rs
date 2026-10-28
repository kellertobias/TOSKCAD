use multicad_core::{
    document::EntityId,
    references::{Attachment, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::*;
use std::f64::consts::PI;
fn p() -> GeometryPolicy {
    Default::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn line(a: [f64; 2], b: [f64; 2]) -> CurvedEdge2d {
    CurvedEdge2d::Line { a, b }
}
fn rect(a: [f64; 2], b: [f64; 2]) -> Vec<CurvedEdge2d> {
    vec![
        line(a, [b[0], a[1]]),
        line([b[0], a[1]], b),
        line(b, [a[0], b[1]]),
        line([a[0], b[1]], a),
    ]
}
fn profile() -> CurvedProfile {
    CurvedProfile {
        frame: Default::default(),
        outer: rect([0., 0.], [10., 6.]),
        holes: vec![],
    }
}
/// z = height + slope*x, including genuine orthonormal in-plane axes.
fn target(height: f64, slope: f64) -> PlaneFrame {
    let l = 1_f64.hypot(slope);
    PlaneFrame {
        origin: [0., 0., height],
        u: [1. / l, 0., slope / l],
        v: [0., 1., 0.],
    }
}
fn origins(profile: &CurvedProfile) -> Vec<Origin> {
    profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
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
fn check_caps_and_frames(solid: &Solid, id: EntityId, target: PlaneFrame) -> usize {
    let snap = solid.topology_snapshot(id, 0).unwrap();
    let end = origin(id, "cap.end");
    let start = origin(id, "cap.start");
    let cap = snap
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&end))
        .expect("genuine named end cap");
    assert_eq!(cap.frames[0].frame.origin_mm, target.origin);
    assert_eq!(cap.frames[0].frame.u, target.u);
    assert_eq!(cap.frames[0].frame.v, target.v);
    assert!(
        snap.elements
            .iter()
            .any(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&start))
    );
    let vertices = solid.display_vertices().unwrap();
    let mut count = 0;
    for e in snap
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Vertex && e.origins.contains(&end))
    {
        count += 1;
        let p = vertices
            .iter()
            .find(|v| v.index == e.key.ordinal)
            .unwrap()
            .position_mm;
        near(
            dot(
                std::array::from_fn(|i| p[i] - target.origin[i]),
                target.normal(),
            ),
            0.,
        );
    }
    assert!(count > 0);
    let faces = solid.faces().unwrap();
    for e in snap
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        assert!(!e.origins.is_empty());
        for anchored in &e.frames {
            anchored.frame.validate().unwrap();
            let (point, n) = faces
                .iter()
                .find(|f| f.index == e.key.ordinal)
                .unwrap()
                .plane
                .unwrap();
            near(
                dot(
                    std::array::from_fn(|i| anchored.frame.origin_mm[i] - point[i]),
                    n,
                ),
                0.,
            );
            near(dot(anchored.frame.u, n), 0.);
            near(dot(anchored.frame.v, n), 0.);
        }
    }
    count
}
#[test]
fn sloping_target_varies_every_end_vertex_instead_of_using_center_distance() {
    let profile = profile();
    let termination = target(10., 0.2);
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0., 0., 1.],
        termination,
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 660.);
    let bounds = solid.bounds().unwrap();
    near(bounds.min[2], 0.);
    near(bounds.max[2], 12.);
    assert_eq!(check_caps_and_frames(&solid, id, termination), 4);
}
#[test]
fn oblique_sweep_to_tilted_plane_has_exact_volume_and_bounds() {
    let profile = profile();
    let termination = target(10., 0.2);
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0.6, 0., 0.8],
        termination,
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 60. * 0.8 * 11. / 0.68);
    let bounds = solid.bounds().unwrap();
    near(bounds.min[0], 0.);
    near(bounds.max[0], 10. + 0.6 * 12. / 0.68);
    near(bounds.max[2], 0.8 * 12. / 0.68);
    check_caps_and_frames(&solid, id, termination);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tilted-plane-oblique.step");
    solid.write_step(&path).unwrap();
    near(
        Solid::read_step(&path, p()).unwrap().volume_mm3().unwrap(),
        solid.volume_mm3().unwrap(),
    );
}
#[test]
fn circular_and_spline_sections_with_holes_clip_true_curves_and_preserve_ancestry() {
    let circle = CurvedProfile {
        frame: Default::default(),
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: 3.,
        }],
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: 1.,
        }]],
    };
    let id = EntityId::new();
    let t = target(10., 0.3);
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &circle,
        [0., 0., 1.],
        t,
        id,
        &origins(&circle),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 80. * PI);
    check_caps_and_frames(&solid, id, t);
    near(solid.bounds().unwrap().max[2], 10.9);
    let profile = CurvedProfile {
        frame: Default::default(),
        outer: vec![
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![[0., 0.], [5., 10.], [10., 0.]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            }),
            line([10., 0.], [0., 0.]),
        ],
        holes: vec![rect([4., 1.], [6., 2.])],
    };
    let id = EntityId::new();
    let curves = origins(&profile);
    let t = target(10., 0.2);
    let solid =
        Solid::extrude_curved_profile_to_plane_named(&profile, [0., 0., 1.], t, id, &curves, p())
            .unwrap();
    near(solid.volume_mm3().unwrap(), (100. / 3. - 2.) * 11.);
    check_caps_and_frames(&solid, id, t);
    let snap = solid.topology_snapshot(id, 0).unwrap();
    assert!(
        snap.elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&curves[0]))
            .all(|e| e.frames.is_empty())
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spline-plane-hole.step");
    solid.write_step(&path).unwrap();
    near(
        Solid::read_step(&path, p()).unwrap().volume_mm3().unwrap(),
        solid.volume_mm3().unwrap(),
    );
}
#[test]
fn full_profile_support_rejects_crossing_despite_reachable_center_or_frame_origin() {
    let mut profile = profile();
    let original = profile.clone();
    for t in [
        target(-1., 0.4),
        target(0., 0.4),
        target(-10., 0.),
        target(1e-7, 0.),
    ] {
        assert!(Solid::extrude_curved_profile_to_plane(&profile, [0., 0., 1.], t, p()).is_err());
        assert_eq!(profile, original);
    }
    // All source points are below the target even though the sketch frame origin
    // lies above it. Half-space selection must use its semantic retained side.
    profile.outer = rect([10., 0.], [20., 6.]);
    let t = target(-1., 0.2);
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0., 0., 1.],
        t,
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 120.);
    check_caps_and_frames(&solid, id, t);
}
#[test]
fn rotated_frames_reverse_direction_and_offset_sources_keep_real_cap_support() {
    let mut profile = profile();
    profile.frame = PlaneFrame {
        origin: [20., 30., 40.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let mut t = target(10., 0.2);
    let rotate = |p: [f64; 3]| [p[2], p[0], p[1]];
    t.origin = std::array::from_fn(|i| profile.frame.origin[i] + rotate(t.origin)[i]);
    t.u = rotate(t.u);
    t.v = rotate(t.v);
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0.8, 0.6, 0.],
        t,
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 60. * 0.8 * 11. / 0.68);
    check_caps_and_frames(&solid, id, t);
    let mut profile = profile.clone();
    profile.frame = Default::default();
    let t = target(-10., 0.2);
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0., 0., -1.],
        t,
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 540.);
    check_caps_and_frames(&solid, id, t);
}
#[test]
fn named_cap_reference_follows_plane_edits_without_replacing_authored_curve_origins() {
    let profile = profile();
    let id = EntityId::new();
    let curves = origins(&profile);
    let first = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0., 0., 1.],
        target(10., 0.2),
        id,
        &curves,
        p(),
    )
    .unwrap()
    .topology_snapshot(id, 0)
    .unwrap();
    let cap = first
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin(id, "cap.end")))
        .unwrap();
    let mut intent = GeometryIntent::capture(&first, cap.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin(id, "cap.end"),
        offset_mm: [2., 3., 0.],
        angle_rad: 0.,
    });
    let t = target(20., -0.3);
    let next =
        Solid::extrude_curved_profile_to_plane_named(&profile, [0.6, 0., 0.8], t, id, &curves, p())
            .unwrap()
            .topology_snapshot(id, 1)
            .unwrap();
    let Resolution::Resolved {
        frame: Some(frame), ..
    } = resolve(&intent, &next, 1)
    else {
        panic!("cap reference unresolved")
    };
    for (i, p) in t.point([2., 3.]).into_iter().enumerate() {
        near(frame.origin_mm[i], p);
    }
}
#[test]
fn invalid_parallel_nonunit_nonfinite_and_out_of_policy_inputs_leave_geometry_untouched() {
    let profile = profile();
    let original = profile.clone();
    let valid =
        Solid::extrude_curved_profile_to_plane(&profile, [0., 0., 1.], target(10., 0.2), p())
            .unwrap();
    let volume = valid.volume_mm3().unwrap();
    for d in [
        [0.; 3],
        [0., 0., 2.],
        [1., 0., 0.],
        [f64::NAN, 0., 1.],
        [1., 0., f64::INFINITY],
    ] {
        assert!(
            Solid::extrude_curved_profile_to_plane(&profile, d, target(10., 0.2), p()).is_err()
        );
    }
    let parallel = PlaneFrame {
        origin: [5., 0., 0.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    assert!(Solid::extrude_curved_profile_to_plane(&profile, [0., 0., 1.], parallel, p()).is_err());
    let mut bad = target(10., 0.2);
    bad.u = [2., 0., 0.];
    assert!(Solid::extrude_curved_profile_to_plane(&profile, [0., 0., 1.], bad, p()).is_err());
    assert!(
        Solid::extrude_curved_profile_to_plane(&profile, [0., 0., 1.], target(1e8, 0.), p())
            .is_err()
    );
    let mut curves = origins(&profile);
    curves[1] = curves[0].clone();
    assert!(
        Solid::extrude_curved_profile_to_plane_named(
            &profile,
            [0., 0., 1.],
            target(10., 0.2),
            EntityId::new(),
            &curves,
            p()
        )
        .is_err()
    );
    assert_eq!(profile, original);
    near(valid.volume_mm3().unwrap(), volume);
}
#[test]
fn oblique_circular_and_arc_sections_use_exact_support_not_only_curve_vertices() {
    let circle = CurvedProfile {
        frame: Default::default(),
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: 3.,
        }],
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: 1.,
        }]],
    };
    let id = EntityId::new();
    let t = target(10., 0.3);
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &circle,
        [0.6, 0., 0.8],
        t,
        id,
        &origins(&circle),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 8. * PI * 0.8 * 10. / 0.62);
    check_caps_and_frames(&solid, id, t);
    assert!(
        Solid::extrude_curved_profile_to_plane(&circle, [0., 0., 1.], target(0.5, 0.3), p())
            .is_err()
    );
    let arc = CurvedProfile {
        frame: Default::default(),
        outer: vec![
            CurvedEdge2d::Arc {
                center: [0., 0.],
                a: [0., -3.],
                b: [0., 3.],
                clockwise: false,
            },
            line([0., 3.], [0., -3.]),
        ],
        holes: vec![],
    };
    let id = EntityId::new();
    let t = target(10., 0.2);
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &arc,
        [0., 0., 1.],
        t,
        id,
        &origins(&arc),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 45. * PI + 3.6);
    check_caps_and_frames(&solid, id, t);
}
#[test]
fn native_spline_support_uses_actual_curve_extrema_and_bounds_oversized_work() {
    // The control pole at y=10 lies above the termination plane; the actual
    // quadratic curve reaches y=5 and remains strictly below it everywhere.
    let profile = CurvedProfile {
        frame: Default::default(),
        outer: vec![
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![[0., 0.], [5., 10.], [10., 0.]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            }),
            line([10., 0.], [0., 0.]),
        ],
        holes: vec![],
    };
    let l = 1_f64.hypot(0.2);
    let t = PlaneFrame {
        origin: [0., 0., 1.5],
        u: [1., 0., 0.],
        v: [0., 1. / l, -0.2 / l],
    };
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0., 0., 1.],
        t,
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 110. / 3.);
    check_caps_and_frames(&solid, id, t);
    let near_parallel = PlaneFrame {
        origin: [20., 0., 0.],
        u: [1e-9, 0., -1.],
        v: [0., 1., 0.],
    };
    let result = Solid::extrude_curved_profile_to_plane(&profile, [0., 0., 1.], near_parallel, p());
    assert!(result.err().unwrap().message.contains("length limit"));
}
