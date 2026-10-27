use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::*;

fn policy() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn line(a: [f64; 2], b: [f64; 2]) -> CurvedEdge2d {
    CurvedEdge2d::Line { a, b }
}
fn arch() -> BSpline2d {
    BSpline2d {
        degree: 2,
        poles: vec![[0., 0.], [5., 10.], [10., 0.]],
        knots: vec![0., 1.],
        multiplicities: vec![3, 3],
        weights: None,
    }
}
fn arch_profile() -> CurvedProfile {
    CurvedProfile {
        frame: Default::default(),
        outer: vec![CurvedEdge2d::BSpline(arch()), line([10., 0.], [0., 0.])],
        holes: vec![],
    }
}
fn quarter() -> BSpline2d {
    BSpline2d {
        degree: 2,
        poles: vec![[10., 0.], [10., 10.], [0., 10.]],
        knots: vec![0., 1.],
        multiplicities: vec![3, 3],
        weights: Some(vec![1., std::f64::consts::FRAC_1_SQRT_2, 1.]),
    }
}
fn reverse(s: &mut BSpline2d) {
    let sum = s.knots[0] + s.knots[s.knots.len() - 1];
    s.poles.reverse();
    s.knots.reverse();
    s.knots.iter_mut().for_each(|v| *v = sum - *v);
    s.multiplicities.reverse();
    if let Some(w) = &mut s.weights {
        w.reverse();
    }
}
fn reverse_profile(p: &mut CurvedProfile) {
    for wire in std::iter::once(&mut p.outer).chain(&mut p.holes) {
        wire.reverse();
        for edge in wire {
            match edge {
                CurvedEdge2d::Line { a, b } => std::mem::swap(a, b),
                CurvedEdge2d::BSpline(s) => reverse(s),
                CurvedEdge2d::Arc {
                    a, b, clockwise, ..
                } => {
                    std::mem::swap(a, b);
                    *clockwise = !*clockwise;
                }
                CurvedEdge2d::Circle { .. } => {}
            }
        }
    }
}
fn rectangle(a: [f64; 2], b: [f64; 2]) -> Vec<CurvedEdge2d> {
    vec![
        line(a, [b[0], a[1]]),
        line([b[0], a[1]], b),
        line(b, [a[0], b[1]]),
        line([a[0], b[1]], a),
    ]
}

#[test]
fn exact_rational_and_nonrational_basis_evaluation_and_domains() {
    let a = arch();
    let sample = a.evaluate(0.5, policy()).unwrap();
    assert_eq!(sample.point, [5., 5.]);
    assert_eq!(sample.derivative, [10., 0.]);
    assert_eq!(a.evaluate(0., policy()).unwrap().point, [0., 0.]);
    assert_eq!(a.evaluate(1., policy()).unwrap().point, [10., 0.]);
    let s = quarter().evaluate(0.5, policy()).unwrap();
    for v in s.point {
        near(v, 10. * std::f64::consts::FRAC_1_SQRT_2);
    }
    for u in [-1e-9, 1.00001, f64::NAN, f64::INFINITY] {
        assert!(a.evaluate(u, policy()).is_err());
    }
    let multi = BSpline2d {
        degree: 2,
        poles: vec![[0., 0.], [3., 6.], [6., 6.], [9., 0.]],
        knots: vec![0., 0.5, 1.],
        multiplicities: vec![3, 1, 3],
        weights: None,
    };
    let middle = multi.evaluate(0.5, policy()).unwrap();
    assert_eq!(middle.point, [4.5, 6.]);
}

#[test]
fn invalid_and_unbounded_bases_fail_before_native_geometry() {
    let mut invalid = vec![];
    for degree in [0, 26] {
        let mut s = arch();
        s.degree = degree;
        invalid.push(s);
    }
    for weights in [
        vec![],
        vec![1., -1., 1.],
        vec![1., f64::NAN, 1.],
        vec![1e-9, 1e9, 1.],
    ] {
        let mut s = arch();
        s.weights = Some(weights);
        invalid.push(s);
    }
    for knots in [vec![], vec![0., 0.], vec![1., 0.], vec![0., f64::INFINITY]] {
        let mut s = arch();
        s.knots = knots;
        invalid.push(s);
    }
    for multiplicities in [vec![2, 3], vec![3, 4], vec![3], vec![u32::MAX, u32::MAX]] {
        let mut s = arch();
        s.multiplicities = multiplicities;
        invalid.push(s);
    }
    for poles in [
        vec![],
        vec![[0., 0.]; 3],
        vec![[0., 0.]; 257],
        vec![[0., 0.], [5., f64::NAN], [10., 0.]],
        vec![[0., 0.], [5., 1e8], [10., 0.]],
    ] {
        let mut s = arch();
        s.poles = poles;
        invalid.push(s);
    }
    for s in invalid {
        assert!(s.validate(policy()).is_err(), "accepted {s:?}");
        assert!(s.evaluate(0.5, policy()).is_err());
    }
}

#[test]
fn exact_intersections_report_parameters_and_overlap_without_polylines() {
    let curve = CurvedEdge2d::BSpline(arch());
    let hits = curve
        .intersections(&line([0., 2.5], [10., 2.5]), policy())
        .unwrap();
    assert_eq!(hits.points.len(), 2);
    assert_eq!(hits.overlap_segments, 0);
    for hit in hits.points {
        near(hit.point[1], 2.5);
        let evaluated = curve.evaluate(hit.first_parameter, policy()).unwrap();
        near(evaluated.point[0], hit.point[0]);
        near(evaluated.point[1], hit.point[1]);
    }
    assert!(
        curve
            .self_intersections(policy())
            .unwrap()
            .points
            .is_empty()
    );
    let overlap = curve.intersections(&curve, policy()).unwrap();
    assert!(overlap.overlap_segments > 0);
    let quarter = CurvedEdge2d::BSpline(quarter());
    let hits = quarter
        .intersections(&line([0., 0.], [10., 10.]), policy())
        .unwrap();
    assert_eq!(hits.points.len(), 1);
    near(
        hits.points[0].point[0],
        10. * std::f64::consts::FRAC_1_SQRT_2,
    );
}

#[test]
fn self_crossing_spline_is_reported_and_cannot_bound_a_face() {
    let spline = BSpline2d {
        degree: 3,
        poles: vec![[0., 0.], [20., 20.], [-10., 20.], [10., 0.]],
        knots: vec![0., 1.],
        multiplicities: vec![4, 4],
        weights: None,
    };
    let edge = CurvedEdge2d::BSpline(spline);
    let hits = edge.self_intersections(policy()).unwrap();
    assert_eq!(hits.points.len(), 1);
    near(hits.points[0].point[0], 5.);
    near(hits.points[0].point[1], 6.);
    let p = CurvedProfile {
        frame: Default::default(),
        outer: vec![edge, line([10., 0.], [0., 0.])],
        holes: vec![],
    };
    assert!(p.area_mm2(policy()).is_err());
    assert!(Solid::extrude_curved_profile(&p, 2., policy()).is_err());
    // A genuinely closed clamped curve may meet itself only at its endpoints.
    let closed = CurvedProfile {
        frame: Default::default(),
        outer: vec![CurvedEdge2d::BSpline(BSpline2d {
            degree: 3,
            poles: vec![[0., 0.], [10., 0.], [0., 10.], [0., 0.]],
            knots: vec![0., 1.],
            multiplicities: vec![4, 4],
            weights: None,
        })],
        holes: vec![],
    };
    assert!(closed.area_mm2(policy()).unwrap() > 0.);
    assert!(
        Solid::extrude_curved_profile(&closed, 2., policy())
            .unwrap()
            .volume_mm3()
            .unwrap()
            > 0.
    );
}

#[test]
fn true_spline_face_holes_winding_classification_and_extrusion() {
    let mut p = arch_profile();
    p.holes.push(rectangle([4., 1.], [6., 2.]));
    near(p.area_mm2(policy()).unwrap(), 100. / 3. - 2.);
    assert_eq!(
        p.classify([5., 3.], policy()).unwrap(),
        CurvePointLocation::Inside
    );
    assert_eq!(
        p.classify([5., 1.5], policy()).unwrap(),
        CurvePointLocation::Outside
    );
    assert_eq!(
        p.classify([5., 5.], policy()).unwrap(),
        CurvePointLocation::Boundary
    );
    assert_eq!(
        p.classify([20., 0.], policy()).unwrap(),
        CurvePointLocation::Outside
    );
    let source = p.clone();
    let a = Solid::extrude_curved_profile(&p, 3., policy()).unwrap();
    near(a.volume_mm3().unwrap(), 94.);
    assert_eq!(p, source);
    reverse_profile(&mut p);
    near(p.area_mm2(policy()).unwrap(), 100. / 3. - 2.);
    p.frame = PlaneFrame {
        origin: [20., 30., 40.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let b = Solid::extrude_curved_profile(&p, -3., policy()).unwrap();
    near(b.volume_mm3().unwrap(), 94.);
    let bounds = b.bounds().unwrap();
    near(bounds.min[0], 17.);
    near(bounds.max[0], 20.);
}

#[test]
fn rational_solid_step_roundtrip_keeps_exact_volume_and_named_faces() {
    std::thread::spawn(|| {
        let feature = EntityId::new();
        let curve = EntityId::new();
        let p = CurvedProfile {
            frame: Default::default(),
            outer: vec![
                CurvedEdge2d::BSpline(quarter()),
                line([0., 10.], [0., 0.]),
                line([0., 0.], [10., 0.]),
            ],
            holes: vec![],
        };
        let origins: Vec<_> = (0..3)
            .map(|i| Origin {
                source: curve,
                role: format!("curve-{i}"),
            })
            .collect();
        near(p.area_mm2(policy()).unwrap(), 25. * std::f64::consts::PI);
        let solid =
            Solid::extrude_curved_profile_named(&p, 4., feature, &origins, policy()).unwrap();
        near(solid.volume_mm3().unwrap(), 100. * std::f64::consts::PI);
        let snapshot = solid.topology_snapshot(feature, 0).unwrap();
        let faces: Vec<_> = snapshot
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .collect();
        assert_eq!(faces.len(), 5);
        let curved = faces
            .iter()
            .find(|e| e.origins.contains(&origins[0]))
            .unwrap();
        assert!(
            curved.frames.is_empty(),
            "curved side must not fabricate a planar attachment frame"
        );
        for origin in &origins {
            assert_eq!(
                faces.iter().filter(|e| e.origins.contains(origin)).count(),
                1
            );
        }
        assert_eq!(
            faces
                .iter()
                .filter(|e| e.origins.iter().any(|o| o.role.starts_with("cap.")))
                .count(),
            2
        );
        let mut rewound = p.clone();
        reverse_profile(&mut rewound);
        let mut reordered = origins.clone();
        reordered.reverse();
        let rebuilt =
            Solid::extrude_curved_profile_named(&rewound, 7., feature, &reordered, policy())
                .unwrap()
                .topology_snapshot(feature, 1)
                .unwrap();
        for face in faces {
            let intent = GeometryIntent::capture(&snapshot, face.key).unwrap();
            assert!(matches!(
                resolve(&intent, &rebuilt, 1),
                Resolution::Resolved { .. }
            ));
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rational.step");
        solid.write_step(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("RATIONAL_B_SPLINE_CURVE"));
        let reopened = Solid::read_step(&path, policy()).unwrap();
        near(reopened.volume_mm3().unwrap(), 100. * std::f64::consts::PI);
        let mut duplicate = origins.clone();
        duplicate[1] = duplicate[0].clone();
        assert!(
            Solid::extrude_curved_profile_named(&p, 4., feature, &duplicate, policy()).is_err()
        );
    })
    .join()
    .unwrap();
}

#[test]
fn invalid_boundaries_do_not_heal_crossings_touching_holes_or_open_wires() {
    let mut cases = vec![];
    let mut open = arch_profile();
    open.outer[1] = line([10., 0.], [0.01, 0.]);
    cases.push(open);
    let mut outside = arch_profile();
    outside.holes.push(rectangle([20., 1.], [21., 2.]));
    cases.push(outside);
    let mut touching = arch_profile();
    touching.holes.push(rectangle([4., 0.], [6., 2.]));
    cases.push(touching);
    let mut nested = arch_profile();
    nested.holes.push(rectangle([3., 1.], [7., 3.]));
    nested.holes.push(rectangle([4., 1.5], [6., 2.]));
    cases.push(nested);
    let mut crossing = arch_profile();
    crossing.outer = vec![
        line([0., 0.], [10., 10.]),
        line([10., 10.], [0., 10.]),
        line([0., 10.], [10., 0.]),
        line([10., 0.], [0., 0.]),
    ];
    cases.push(crossing);
    for p in cases {
        assert!(p.area_mm2(policy()).is_err(), "accepted {p:?}");
        assert!(Solid::extrude_curved_profile(&p, 3., policy()).is_err());
    }
    let mut budget = arch_profile();
    budget.outer = vec![CurvedEdge2d::BSpline(arch()); 65];
    assert_eq!(
        budget.area_mm2(policy()).unwrap_err().kind,
        GeometryErrorKind::Limit
    );
}

#[test]
fn mixed_true_arcs_circles_and_spline_profiles_have_exact_material_and_origins() {
    let circle = CurvedEdge2d::Circle {
        center: [5., 2.],
        radius_mm: 0.5,
    };
    let mut p = arch_profile();
    p.holes.push(vec![circle.clone()]);
    let expected = 100. / 3. - std::f64::consts::PI * 0.25;
    near(p.area_mm2(policy()).unwrap(), expected);
    near(
        Solid::extrude_curved_profile(&p, 4., policy())
            .unwrap()
            .volume_mm3()
            .unwrap(),
        expected * 4.,
    );
    let arc = CurvedEdge2d::Arc {
        center: [0., 0.],
        a: [10., 0.],
        b: [0., 10.],
        clockwise: false,
    };
    let domain = arc.domain(policy()).unwrap();
    near(domain[1], std::f64::consts::FRAC_PI_2);
    let mid = arc.evaluate(domain[1] * 0.5, policy()).unwrap();
    near(mid.point[0], 10. * std::f64::consts::FRAC_1_SQRT_2);
    near(mid.point[1], mid.point[0]);
    // A linear B-spline is still an exact authored spline, not a display segment.
    let closure = CurvedEdge2d::BSpline(BSpline2d {
        degree: 1,
        poles: vec![[0., 10.], [0., 0.]],
        knots: vec![0., 1.],
        multiplicities: vec![2, 2],
        weights: None,
    });
    let mut mixed = CurvedProfile {
        frame: Default::default(),
        outer: vec![arc.clone(), closure, line([0., 0.], [10., 0.])],
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [3., 3.],
            radius_mm: 1.,
        }]],
    };
    let feature = EntityId::new();
    let origins: Vec<_> = (0..4)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect();
    let solid =
        Solid::extrude_curved_profile_named(&mixed, 3., feature, &origins, policy()).unwrap();
    near(solid.volume_mm3().unwrap(), 72. * std::f64::consts::PI);
    let before = solid.topology_snapshot(feature, 0).unwrap();
    reverse_profile(&mut mixed);
    let mut reordered = origins.clone();
    reordered[..3].reverse();
    let after = Solid::extrude_curved_profile_named(&mixed, 5., feature, &reordered, policy())
        .unwrap()
        .topology_snapshot(feature, 1)
        .unwrap();
    for face in before
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        let intent = GeometryIntent::capture(&before, face.key).unwrap();
        assert!(matches!(
            resolve(&intent, &after, 1),
            Resolution::Resolved { .. }
        ));
    }
    let clockwise = CurvedEdge2d::Arc {
        center: [0., 0.],
        a: [0., 10.],
        b: [10., 0.],
        clockwise: true,
    };
    near(
        clockwise
            .evaluate(clockwise.domain(policy()).unwrap()[1], policy())
            .unwrap()
            .point[0],
        10.,
    );
    let hit = arc
        .intersections(
            &CurvedEdge2d::BSpline(BSpline2d {
                degree: 1,
                poles: vec![[0., 0.], [10., 10.]],
                knots: vec![0., 1.],
                multiplicities: vec![2, 2],
                weights: None,
            }),
            policy(),
        )
        .unwrap();
    assert_eq!(hit.points.len(), 1);
    near(
        hit.points[0].point[0],
        10. * std::f64::consts::FRAC_1_SQRT_2,
    );
    let invalid = CurvedEdge2d::Arc {
        center: [0., 0.],
        a: [1., 0.],
        b: [0., 2.],
        clockwise: false,
    };
    assert!(invalid.evaluate(0., policy()).is_err());
    let invalid = CurvedEdge2d::Circle {
        center: [0., 0.],
        radius_mm: -1.,
    };
    assert!(invalid.evaluate(0., policy()).is_err());
}
