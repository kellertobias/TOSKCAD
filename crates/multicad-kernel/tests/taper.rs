use multicad_core::{
    document::EntityId,
    references::{Origin, TopologyKind},
};
use multicad_kernel::*;
use std::f64::consts::{FRAC_PI_2, PI};
fn p() -> GeometryPolicy {
    Default::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
fn line(a: [f64; 2], b: [f64; 2]) -> CurvedEdge2d {
    CurvedEdge2d::Line { a, b }
}
fn rectangle() -> CurvedProfile {
    CurvedProfile {
        frame: Default::default(),
        outer: vec![
            line([0., 0.], [10., 0.]),
            line([10., 0.], [10., 6.]),
            line([10., 6.], [0., 6.]),
            line([0., 6.], [0., 0.]),
        ],
        holes: vec![],
    }
}
fn circle(radius: f64) -> CurvedProfile {
    CurvedProfile {
        frame: Default::default(),
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: radius,
        }],
        holes: vec![],
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
fn named(profile: &CurvedProfile, d: [f64; 3], delta: f64) -> (EntityId, Solid) {
    let id = EntityId::new();
    let length = d.into_iter().map(|x| x * x).sum::<f64>().sqrt();
    let s = Solid::extrude_curved_profile_vector_tapered_named(
        profile,
        d,
        (delta / length).atan(),
        id,
        &origins(profile),
        p(),
    )
    .unwrap();
    (id, s)
}
fn check_frames(solid: &Solid, id: EntityId) {
    let faces = solid.faces().unwrap();
    let snapshot = solid.topology_snapshot(id, 0).unwrap();
    for e in snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        assert!(!e.origins.is_empty());
        for f in &e.frames {
            f.frame.validate().unwrap();
            let (plane, n) = faces
                .iter()
                .find(|f| f.index == e.key.ordinal)
                .unwrap()
                .plane
                .unwrap();
            let dot =
                |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
            near(
                dot(std::array::from_fn(|i| f.frame.origin_mm[i] - plane[i]), n),
                0.,
            );
            near(dot(f.frame.u, n), 0.);
            near(dot(f.frame.v, n), 0.);
        }
    }
    assert_eq!(solid.solid_count().unwrap(), 1);
}
#[test]
fn rectangle_uses_equal_edge_offsets_instead_of_uniform_scale() {
    let profile = rectangle();
    let before = profile.clone();
    for delta in [1., -1.] {
        let (id, solid) = named(&profile, [0., 0., 10.], delta);
        near(
            solid.volume_mm3().unwrap(),
            10. * (60. + 16. * delta + 4. * delta * delta / 3.),
        );
        check_frames(&solid, id);
        let snapshot = solid.topology_snapshot(id, 0).unwrap();
        let cap = Origin {
            source: id,
            role: "cap.end".into(),
        };
        let vertices = solid.display_vertices().unwrap();
        let mut xs = vec![];
        let mut ys = vec![];
        for e in snapshot
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Vertex && e.origins.contains(&cap))
        {
            let v = vertices.iter().find(|v| v.index == e.key.ordinal).unwrap();
            xs.push(v.position_mm[0]);
            ys.push(v.position_mm[1]);
            near(v.position_mm[2], 10.);
        }
        near(xs.iter().copied().fold(f64::INFINITY, f64::min), -delta);
        near(
            xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            10. + delta,
        );
        near(ys.iter().copied().fold(f64::INFINITY, f64::min), -delta);
        near(
            ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            6. + delta,
        );
        assert_eq!(profile, before);
    }
}
#[test]
fn exact_circular_frusta_support_widening_narrowing_oblique_and_reverse_sweeps() {
    let profile = circle(3.);
    for (d, delta) in [
        ([0., 0., 10.], 1.),
        ([0., 0., 10.], -2.),
        ([0., 0., -10.], 1.),
        ([6., 0., 8.], 1.),
    ] {
        let (id, solid) = named(&profile, d, delta);
        let r = 3. + delta;
        let normalheight = d[2].abs();
        near(
            solid.volume_mm3().unwrap(),
            PI * normalheight * (9. + 3. * r + r * r) / 3.,
        );
        check_frames(&solid, id);
        let snapshot = solid.topology_snapshot(id, 0).unwrap();
        assert!(
            snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face
                    && e.origins.iter().any(|o| o.role.starts_with("side/")))
                .all(|e| e.frames.is_empty())
        );
        if d == [6., 0., 8.] {
            let bounds = solid.bounds().unwrap();
            near(bounds.min[0], -3.);
            near(bounds.max[0], 10.);
            near(bounds.min[1], -4.);
            near(bounds.max[1], 4.);
        }
    }
}
#[test]
fn material_expansion_shrinks_holes_with_exact_volume_and_step_roundtrip() {
    let mut profile = circle(5.);
    profile.holes = vec![vec![CurvedEdge2d::Circle {
        center: [0., 0.],
        radius_mm: 2.,
    }]];
    let (id, solid) = named(&profile, [6., 0., 8.], 1.);
    near(solid.volume_mm3().unwrap(), 224. * PI);
    check_frames(&solid, id);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("exact-oblique-taper-holes.step");
    solid.write_step(&path).unwrap();
    near(
        Solid::read_step(&path, p()).unwrap().volume_mm3().unwrap(),
        224. * PI,
    );
    let mut profile = rectangle();
    profile.holes = vec![vec![CurvedEdge2d::Circle {
        center: [5., 3.],
        radius_mm: 2.,
    }]];
    let (id, solid) = named(&profile, [0., 0., 10.], 0.5);
    near(
        solid.volume_mm3().unwrap(),
        10. * (60. + 8. + 1. / 3.) - 10. * PI * (4. + 3. + 2.25) / 3.,
    );
    check_frames(&solid, id);
}
#[test]
fn polygon_holes_and_rotated_frames_preserve_edge_identity_and_offset_direction() {
    let mut profile = rectangle();
    profile.holes = vec![vec![
        line([3., 2.], [7., 2.]),
        line([7., 2.], [7., 4.]),
        line([7., 4.], [3., 4.]),
        line([3., 4.], [3., 2.]),
    ]];
    profile.frame = PlaneFrame {
        origin: [20., 30., 40.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let (id, solid) = named(&profile, [8., 6., 0.], 0.5);
    near(
        solid.volume_mm3().unwrap(),
        8. * ((60. + 8. + 1. / 3.) - (8. - 3. + 1. / 3.)),
    );
    check_frames(&solid, id);
    let source = circle(3.);
    let id = EntityId::new();
    let curves = origins(&source);
    let a = Solid::extrude_curved_profile_vector_tapered_named(
        &source,
        [0., 0., 10.],
        0.1,
        id,
        &curves,
        p(),
    )
    .unwrap();
    let b = Solid::extrude_curved_profile_vector_tapered_named(
        &source,
        [0., 0., 20.],
        0.1,
        id,
        &curves,
        p(),
    )
    .unwrap();
    for solid in [a, b] {
        let snapshot = solid.topology_snapshot(id, 0).unwrap();
        assert!(
            snapshot
                .elements
                .iter()
                .any(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&curves[0]))
        );
    }
}
#[test]
fn collapse_topology_changes_concavity_and_unsupported_offsets_fail_without_source_mutation() {
    let profile = rectangle();
    let before = profile.clone();
    for a in [
        FRAC_PI_2,
        -FRAC_PI_2,
        f64::NAN,
        f64::INFINITY,
        (-0.4_f64).atan(),
        1e-10,
    ] {
        assert!(
            Solid::extrude_curved_profile_vector_tapered(&profile, [0., 0., 10.], a, p()).is_err()
        );
    }
    assert!(
        Solid::extrude_curved_profile_vector_tapered(&profile, [10., 0., 0.], 0.1, p()).is_err()
    );
    let mut holes = rectangle();
    holes.holes = vec![vec![CurvedEdge2d::Circle {
        center: [5., 3.],
        radius_mm: 1.,
    }]];
    assert!(Solid::extrude_curved_profile_vector_tapered(&holes, [0., 0., 10.], 0.1, p()).is_err());
    assert!(
        Solid::extrude_curved_profile_vector_tapered(&holes, [0., 0., 10.], -0.2, p()).is_err()
    );
    let mut concave = rectangle();
    concave.outer = vec![
        line([0., 0.], [4., 0.]),
        line([4., 0.], [4., 4.]),
        line([4., 4.], [2., 2.]),
        line([2., 2.], [0., 4.]),
        line([0., 4.], [0., 0.]),
    ];
    assert!(
        Solid::extrude_curved_profile_vector_tapered(&concave, [0., 0., 10.], 0.1, p())
            .err()
            .unwrap()
            .message
            .contains("convex")
    );
    let spline = CurvedProfile {
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
    assert!(
        Solid::extrude_curved_profile_vector_tapered(&spline, [0., 0., 10.], 0.1, p())
            .err()
            .unwrap()
            .message
            .contains("unsupported")
    );
    let ordinary = Solid::extrude_curved_profile_vector(&spline, [1., 2., 10.], p()).unwrap();
    let zero =
        Solid::extrude_curved_profile_vector_tapered(&spline, [1., 2., 10.], 0., p()).unwrap();
    near(ordinary.volume_mm3().unwrap(), zero.volume_mm3().unwrap());
    assert_eq!(profile, before);
}
#[test]
fn strict_convex_triangle_offsets_are_winding_independent_and_closed() {
    let profile = CurvedProfile {
        frame: Default::default(),
        outer: vec![
            line([0., 0.], [8., 0.]),
            line([8., 0.], [0., 6.]),
            line([0., 6.], [0., 0.]),
        ],
        holes: vec![],
    };
    let (mut id, mut solid) = named(&profile, [0., 0., 10.], 0.5);
    near(solid.volume_mm3().unwrap(), 305.);
    check_frames(&solid, id);
    let mut reversed = profile.clone();
    reversed.outer = profile
        .outer
        .iter()
        .rev()
        .map(|e| match e {
            CurvedEdge2d::Line { a, b } => line(*b, *a),
            _ => unreachable!(),
        })
        .collect();
    (id, solid) = named(&reversed, [0., 0., 10.], 0.5);
    near(solid.volume_mm3().unwrap(), 305.);
    check_frames(&solid, id);
    let bounds = solid.bounds().unwrap();
    near(bounds.min[0], -0.5);
    near(bounds.min[1], -0.5);
    near(bounds.max[0], 9.5);
    near(bounds.max[1], 7.);
}
#[test]
fn two_tapered_halves_start_at_original_section_and_keep_only_genuine_far_caps() {
    let profile = circle(3.);
    let id = EntityId::new();
    let curves = origins(&profile);
    let alpha = 0.1_f64.atan();
    let forward = Solid::extrude_curved_profile_vector_tapered_named(
        &profile,
        [0., 0., 10.],
        alpha,
        id,
        &curves,
        p(),
    )
    .unwrap();
    let backward = Solid::extrude_curved_profile_vector_tapered_named(
        &profile,
        [0., 0., -5.],
        alpha,
        id,
        &curves,
        p(),
    )
    .unwrap()
    .with_feature_cap_roles(id, "cap.end", "cap.start")
    .unwrap();
    let result = forward
        .boolean(&backward, BooleanOperation::Union, p())
        .unwrap()
        .unwrap();
    near(result.volume_mm3().unwrap(), 176.25 * PI);
    check_frames(&result, id);
    let snapshot = result.topology_snapshot(id, 0).unwrap();
    for (role, z) in [("cap.start", -5.), ("cap.end", 10.)] {
        let caps: Vec<_> = snapshot
            .elements
            .iter()
            .filter(|e| {
                e.key.kind == TopologyKind::Face
                    && e.origins.contains(&Origin {
                        source: id,
                        role: role.into(),
                    })
            })
            .collect();
        assert_eq!(caps.len(), 1);
        near(caps[0].frames[0].frame.origin_mm[2], z);
    }
}
#[test]
fn unsupported_arc_offsets_collinearity_and_bad_names_are_explicit() {
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
    assert!(
        Solid::extrude_curved_profile_vector_tapered(&arc, [0., 0., 10.], 0.1, p())
            .err()
            .unwrap()
            .message
            .contains("unsupported")
    );
    assert!(Solid::extrude_curved_profile_vector_tapered(&arc, [0., 0., 10.], 0., p()).is_ok());
    let mut collinear = rectangle();
    collinear.outer = vec![
        line([0., 0.], [5., 0.]),
        line([5., 0.], [10., 0.]),
        line([10., 0.], [10., 6.]),
        line([10., 6.], [0., 6.]),
        line([0., 6.], [0., 0.]),
    ];
    assert!(
        Solid::extrude_curved_profile_vector_tapered(&collinear, [0., 0., 10.], 0.1, p())
            .err()
            .unwrap()
            .message
            .contains("collinear")
    );
    let mut gap = rectangle();
    let CurvedEdge2d::Line { b, .. } = &mut gap.outer[0] else {
        unreachable!()
    };
    b[0] += 1e-8;
    assert!(
        Solid::extrude_curved_profile_vector_tapered(&gap, [0., 0., 10.], 0.1, p())
            .err()
            .unwrap()
            .message
            .contains("shared endpoints")
    );
    let profile = rectangle();
    let mut curves = origins(&profile);
    curves[1] = curves[0].clone();
    assert!(
        Solid::extrude_curved_profile_vector_tapered_named(
            &profile,
            [0., 0., 10.],
            0.1,
            EntityId::new(),
            &curves,
            p()
        )
        .is_err()
    );
}
