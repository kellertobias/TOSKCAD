use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::*;
fn p() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-7, "{a} != {b}");
}
fn xy(z: f64) -> PlaneFrame {
    PlaneFrame {
        origin: [0., 0., z],
        ..Default::default()
    }
}
fn vertical_x(x: f64) -> PlaneFrame {
    PlaneFrame {
        origin: [x, 0., 0.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    }
}
fn vertical_y(y: f64) -> PlaneFrame {
    PlaneFrame {
        origin: [0., y, 0.],
        u: [1., 0., 0.],
        v: [0., 0., -1.],
    }
}
fn query(
    s: &Solid,
    selection: BRepSelection,
    frame: PlaneFrame,
    method: SketchReferenceMethod,
) -> SketchReferenceCatalog {
    s.sketch_reference_geometry(selection, frame, method, p())
        .unwrap()
}
fn edge_matching(
    s: &Solid,
    frame: PlaneFrame,
    predicate: impl Fn(&SketchReferenceGeometry) -> bool,
) -> u32 {
    let snapshot = s.topology_snapshot(EntityId::new(), 0).unwrap();
    snapshot
        .elements
        .into_iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .find_map(|e| {
            s.sketch_reference_geometry(
                BRepSelection::Edge(e.key.ordinal),
                frame,
                SketchReferenceMethod::Coplanar,
                p(),
            )
            .ok()
            .and_then(|q| {
                q.results
                    .into_iter()
                    .any(|r| predicate(&r.geometry))
                    .then_some(e.key.ordinal)
            })
        })
        .expect("native edge with requested exact geometry")
}
fn points(q: &SketchReferenceCatalog) -> Vec<[f64; 2]> {
    q.results
        .iter()
        .map(|r| match r.geometry {
            SketchReferenceGeometry::Point(p) => p,
            ref other => panic!("expected point, got {other:?}"),
        })
        .collect()
}
fn arch(weights: Option<Vec<f64>>) -> BSpline2d {
    BSpline2d {
        degree: 2,
        poles: vec![[0., 0.], [5., 10.], [10., 0.]],
        knots: vec![0., 1.],
        multiplicities: vec![3, 3],
        weights,
    }
}
fn spline_solid(spline: BSpline2d) -> Solid {
    Solid::extrude_curved_profile(
        &CurvedProfile {
            frame: Default::default(),
            outer: vec![
                CurvedEdge2d::BSpline(spline),
                CurvedEdge2d::Line {
                    a: [10., 0.],
                    b: [0., 0.],
                },
            ],
            holes: vec![],
        },
        3.,
        p(),
    )
    .unwrap()
}
fn spline_edge(s: &Solid) -> u32 {
    edge_matching(s, xy(0.), |g| {
        matches!(g, SketchReferenceGeometry::Curve(CurvedEdge2d::BSpline(_)))
    })
}
#[test]
fn finite_line_plane_contacts_reject_infinite_support_and_preserve_native_identity() {
    let s = Solid::box_solid([10., 20., 30.], p()).unwrap();
    let snapshot = s.topology_snapshot(EntityId::new(), 7).unwrap();
    let index = snapshot
        .elements
        .iter()
        .find_map(|e| match e.geometry {
            Some(AnalyticGeometry::Line { a_mm: a, b_mm: b })
                if a[0].abs() < 1e-8
                    && b[0].abs() < 1e-8
                    && a[1].abs() < 1e-8
                    && b[1].abs() < 1e-8 =>
            {
                Some(e.key.ordinal)
            }
            _ => None,
        })
        .unwrap();
    let before = s.volume_mm3().unwrap();
    let hit = query(
        &s,
        BRepSelection::Edge(index),
        xy(15.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(points(&hit), vec![[0., 0.]]);
    assert!(!hit.coincident);
    assert_eq!(hit.results[0].support.len(), 1);
    assert_eq!(hit.results[0].support[0].ordinal, index);
    assert!(hit.results[0].source_parameter.is_some());
    assert!(hit.results[0].crossing.is_some());
    assert!(!hit.results[0].tangent);
    assert!(
        query(
            &s,
            BRepSelection::Edge(index),
            xy(31.),
            SketchReferenceMethod::Intersect
        )
        .results
        .is_empty()
    );
    let overlap = query(
        &s,
        BRepSelection::Edge(index),
        vertical_x(0.),
        SketchReferenceMethod::Intersect,
    );
    assert!(overlap.coincident && overlap.results.is_empty());
    let endpoint = query(
        &s,
        BRepSelection::Edge(index),
        xy(0.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(endpoint.results.len(), 1);
    near(before, s.volume_mm3().unwrap());
    assert_eq!(snapshot, s.topology_snapshot(snapshot.owner, 7).unwrap());
}
#[test]
fn finite_circle_and_arc_report_two_tangent_empty_and_trimmed_endpoint_contacts() {
    let s = Solid::cylinder(Default::default(), 5., 10., p()).unwrap();
    let circle = edge_matching(&s, xy(0.), |g| {
        matches!(
            g,
            SketchReferenceGeometry::Curve(CurvedEdge2d::Circle { .. })
        )
    });
    let hit = query(
        &s,
        BRepSelection::Edge(circle),
        vertical_x(0.),
        SketchReferenceMethod::Intersect,
    );
    let mut coordinates = points(&hit);
    coordinates.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(coordinates.len(), 2);
    near(coordinates[0][0], -5.);
    near(coordinates[1][0], 5.);
    assert!(
        hit.results
            .iter()
            .all(|r| r.source_parameter.is_some() && r.crossing.is_some())
    );
    let tangent = query(
        &s,
        BRepSelection::Edge(circle),
        vertical_x(5.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(tangent.results.len(), 1);
    assert!(tangent.results[0].tangent);
    assert_eq!(tangent.results[0].crossing, None);
    assert!(
        query(
            &s,
            BRepSelection::Edge(circle),
            vertical_x(6.),
            SketchReferenceMethod::Intersect
        )
        .results
        .is_empty()
    );
    let sector = Solid::extrude_curved_profile(
        &CurvedProfile {
            frame: Default::default(),
            outer: vec![
                CurvedEdge2d::Arc {
                    center: [0., 0.],
                    a: [5., 0.],
                    b: [0., 5.],
                    clockwise: false,
                },
                CurvedEdge2d::Line {
                    a: [0., 5.],
                    b: [0., 0.],
                },
                CurvedEdge2d::Line {
                    a: [0., 0.],
                    b: [5., 0.],
                },
            ],
            holes: vec![],
        },
        2.,
        p(),
    )
    .unwrap();
    let arc = edge_matching(&sector, xy(0.), |g| {
        matches!(g, SketchReferenceGeometry::Curve(CurvedEdge2d::Arc { .. }))
    });
    assert!(
        query(
            &sector,
            BRepSelection::Edge(arc),
            vertical_y(-1.),
            SketchReferenceMethod::Intersect
        )
        .results
        .is_empty()
    );
    let end = query(
        &sector,
        BRepSelection::Edge(arc),
        vertical_y(0.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(end.results.len(), 1);
    near(points(&end)[0][0], 5.);
}
#[test]
fn rational_and_nonrational_spline_contacts_are_native_parameterized_and_tangent() {
    for weights in [None, Some(vec![1., 0.7, 1.])] {
        let authored = arch(weights);
        let s = spline_solid(authored.clone());
        let edge = spline_edge(&s);
        let hit = query(
            &s,
            BRepSelection::Edge(edge),
            vertical_y(3.),
            SketchReferenceMethod::Intersect,
        );
        assert_eq!(hit.results.len(), 2);
        assert!(!hit.coincident);
        for r in &hit.results {
            let u = r.source_parameter.unwrap();
            let actual = authored.evaluate(u, p()).unwrap();
            near(actual.point[1], 3.);
            let SketchReferenceGeometry::Point(q) = r.geometry else {
                panic!()
            };
            near(actual.point[0], q[0]);
            near(q[1], 0.);
            assert!(r.crossing.is_some());
        }
        let top = authored.evaluate(0.5, p()).unwrap().point[1];
        let tangent = query(
            &s,
            BRepSelection::Edge(edge),
            vertical_y(top),
            SketchReferenceMethod::Intersect,
        );
        assert_eq!(tangent.results.len(), 1);
        assert!(tangent.results[0].tangent);
        assert!(
            query(
                &s,
                BRepSelection::Edge(edge),
                vertical_y(top + 1.),
                SketchReferenceMethod::Intersect
            )
            .results
            .is_empty()
        );
    }
}
#[test]
fn exact_rational_pole_projection_preserves_basis_weights_and_real_tilted_circle_locus() {
    let authored = arch(Some(vec![1., 0.7, 1.]));
    let s = spline_solid(authored.clone());
    let edge = spline_edge(&s);
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let tilted = PlaneFrame {
        origin: [0., 0., 0.],
        u: [1., 0., 0.],
        v: [0., c, c],
    };
    let q = query(
        &s,
        BRepSelection::Edge(edge),
        tilted,
        SketchReferenceMethod::Project,
    );
    let SketchReferenceGeometry::Curve(CurvedEdge2d::BSpline(projected)) = &q.results[0].geometry
    else {
        panic!("expected exact spline")
    };
    assert_eq!(projected.degree, authored.degree);
    assert_eq!(projected.knots, authored.knots);
    assert_eq!(projected.multiplicities, authored.multiplicities);
    assert_eq!(projected.weights, authored.weights);
    for i in 0..=80 {
        let u = i as f64 / 80.;
        let a = authored.evaluate(u, p()).unwrap().point;
        let b = projected.evaluate(u, p()).unwrap().point;
        near(a[0], b[0]);
        near(a[1] * c, b[1]);
    }
    assert!(
        s.sketch_reference_geometry(
            BRepSelection::Edge(edge),
            tilted,
            SketchReferenceMethod::Coplanar,
            p()
        )
        .is_err()
    );
    let cylinder = Solid::cylinder(Default::default(), 5., 3., p()).unwrap();
    let circle = edge_matching(&cylinder, xy(0.), |g| {
        matches!(
            g,
            SketchReferenceGeometry::Curve(CurvedEdge2d::Circle { .. })
        )
    });
    let q = query(
        &cylinder,
        BRepSelection::Edge(circle),
        tilted,
        SketchReferenceMethod::Project,
    );
    let SketchReferenceGeometry::Curve(CurvedEdge2d::BSpline(ellipse)) = &q.results[0].geometry
    else {
        panic!("tilted circle must become rational spline")
    };
    assert!(ellipse.weights.is_some());
    let domain = [ellipse.knots[0], *ellipse.knots.last().unwrap()];
    for i in 0..=100 {
        let u = domain[0] + (domain[1] - domain[0]) * i as f64 / 100.;
        let at = ellipse.evaluate(u, p()).unwrap().point;
        near(at[0] * at[0] + 2. * at[1] * at[1], 25.);
    }
}
#[test]
fn finite_face_sections_respect_holes_curved_surfaces_rotated_planes_and_empty_results() {
    let panel = Solid::box_solid([20., 20., 10.], p()).unwrap();
    let hole = Solid::cylinder(
        PlaneFrame {
            origin: [10., 10., -1.],
            ..Default::default()
        },
        3.,
        12.,
        p(),
    )
    .unwrap();
    let panel = panel
        .boolean(&hole, BooleanOperation::Subtract, p())
        .unwrap()
        .unwrap();
    let top = panel
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane
                .is_some_and(|(o, n)| (o[2] - 10.).abs() < 1e-7 && n[2].abs() > 0.99)
        })
        .unwrap()
        .index;
    let q = query(
        &panel,
        BRepSelection::Face(top),
        vertical_y(10.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(q.results.len(), 2);
    assert_ne!(
        q.results[0].support, q.results[1].support,
        "Native boundary history distinguishes the hole-split branches"
    );
    for r in &q.results {
        assert!(
            r.support
                .iter()
                .any(|k| k.kind == TopologyKind::Face && k.ordinal == top)
        );
        let SketchReferenceGeometry::Curve(CurvedEdge2d::Line { a, b }) = r.geometry else {
            panic!()
        };
        let low = a[0].min(b[0]);
        let high = a[0].max(b[0]);
        assert!(high <= 7. + 1e-7 || low >= 13. - 1e-7);
    }
    assert!(
        query(
            &panel,
            BRepSelection::Face(top),
            vertical_y(30.),
            SketchReferenceMethod::Intersect
        )
        .results
        .is_empty()
    );
    let same = query(
        &panel,
        BRepSelection::Face(top),
        xy(10.),
        SketchReferenceMethod::Intersect,
    );
    assert!(same.coincident && same.results.is_empty());
    assert_eq!(
        query(
            &panel,
            BRepSelection::Face(top),
            xy(10.),
            SketchReferenceMethod::Coplanar
        )
        .results
        .len(),
        5
    );
    let cylinder = Solid::cylinder(Default::default(), 5., 10., p()).unwrap();
    let q = query(
        &cylinder,
        BRepSelection::Whole,
        xy(5.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(q.results.len(), 1);
    assert!(matches!(
        q.results[0].geometry,
        SketchReferenceGeometry::Curve(CurvedEdge2d::Circle { .. })
    ));
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let tilt = PlaneFrame {
        origin: [0., 0., 5.],
        u: [1., 0., 0.],
        v: [0., c, c],
    };
    let q = query(
        &cylinder,
        BRepSelection::Whole,
        tilt,
        SketchReferenceMethod::Intersect,
    );
    assert!(!q.results.is_empty());
    assert!(q.results.iter().all(|r| !r.support.is_empty()));
    assert!(
        query(
            &cylinder,
            BRepSelection::Whole,
            xy(11.),
            SketchReferenceMethod::Intersect
        )
        .results
        .is_empty()
    );
}
#[test]
fn transformed_brep_is_queried_once_and_invalid_selections_never_mutate_source() {
    let s = Solid::box_solid([10., 20., 30.], p()).unwrap();
    let moved = s
        .transformed(RigidTransform {
            rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
            translation_mm: [100., 200., 300.],
        })
        .unwrap();
    let q = query(
        &moved,
        BRepSelection::Whole,
        PlaneFrame {
            origin: [100., 200., 315.],
            u: [0., 1., 0.],
            v: [-1., 0., 0.],
        },
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(q.results.len(), 4);
    for r in &q.results {
        let SketchReferenceGeometry::Curve(CurvedEdge2d::Line { a, b }) = r.geometry else {
            panic!()
        };
        for p in [a, b] {
            assert!((-1e-7..=10. + 1e-7).contains(&p[0]));
            assert!((-1e-7..=20. + 1e-7).contains(&p[1]));
        }
    }
    for selection in [
        BRepSelection::Edge(u32::MAX),
        BRepSelection::Face(u32::MAX),
        BRepSelection::Vertex(u32::MAX),
    ] {
        assert!(
            s.sketch_reference_geometry(selection, xy(0.), SketchReferenceMethod::Intersect, p())
                .is_err()
        );
    }
    assert!(
        s.sketch_reference_geometry(
            BRepSelection::Whole,
            xy(0.),
            SketchReferenceMethod::Project,
            p()
        )
        .is_err()
    );
    near(s.volume_mm3().unwrap(), 6000.);
}
#[test]
fn independent_native_section_owners_can_query_concurrently() {
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for _ in 0..6 {
                    let s = Solid::cylinder(Default::default(), 5., 10., p()).unwrap();
                    let q = query(
                        &s,
                        BRepSelection::Whole,
                        xy(5.),
                        SketchReferenceMethod::Intersect,
                    );
                    assert_eq!(q.results.len(), 1);
                }
            });
        }
    });
}

#[test]
fn native_trimmed_circular_fragments_keep_finite_source_parameters_across_the_circle_seam() {
    let cylinder = Solid::cylinder(Default::default(), 5., 3., p()).unwrap();
    let clip = Solid::box_solid([5., 12., 5.], p())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [2., -6., -1.],
            ..Default::default()
        })
        .unwrap();
    let clipped = cylinder
        .boolean(&clip, BooleanOperation::Intersect, p())
        .unwrap()
        .unwrap();
    let snapshot = clipped.topology_snapshot(EntityId::new(), 0).unwrap();
    let mut contacts = vec![];
    for key in snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .map(|e| e.key)
    {
        let Ok(projected) = clipped.sketch_reference_geometry(
            BRepSelection::Edge(key.ordinal),
            xy(0.),
            SketchReferenceMethod::Coplanar,
            p(),
        ) else {
            continue;
        };
        if !matches!(
            projected.results[0].geometry,
            SketchReferenceGeometry::Curve(CurvedEdge2d::Arc { .. })
        ) {
            continue;
        }
        let domain = projected.results[0].source_interval.unwrap();
        let hits = query(
            &clipped,
            BRepSelection::Edge(key.ordinal),
            vertical_x(4.),
            SketchReferenceMethod::Intersect,
        );
        for result in &hits.results {
            assert_eq!(result.source_interval, Some(domain));
            let at = result.source_parameter.unwrap();
            assert!(at >= domain[0] && at <= domain[1]);
        }
        contacts.extend(points(&hits));
    }
    contacts.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(contacts.len(), 2);
    near(contacts[0][0], -3.);
    near(contacts[1][0], 3.);
}

#[test]
fn singular_conic_collapsed_and_closed_retraced_spline_projections_fail_but_open_line_locus_survives()
 {
    let cylinder = Solid::cylinder(Default::default(), 5., 3., p()).unwrap();
    let circle = edge_matching(&cylinder, xy(0.), |g| {
        matches!(
            g,
            SketchReferenceGeometry::Curve(CurvedEdge2d::Circle { .. })
        )
    });
    let failure = cylinder
        .sketch_reference_geometry(
            BRepSelection::Edge(circle),
            vertical_x(0.),
            SketchReferenceMethod::Project,
            p(),
        )
        .unwrap_err();
    assert!(failure.message.contains("singular"));
    let curved = spline_solid(arch(None));
    let edge = spline_edge(&curved);
    let failure = curved
        .sketch_reference_geometry(
            BRepSelection::Edge(edge),
            vertical_x(0.),
            SketchReferenceMethod::Project,
            p(),
        )
        .unwrap_err();
    assert!(failure.message.contains("retraces"));
    let profile = CurvedProfile {
        frame: Default::default(),
        outer: vec![
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 2,
                poles: vec![[0., 0.], [5., 0.], [10., 0.]],
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            }),
            CurvedEdge2d::Line {
                a: [10., 0.],
                b: [10., 5.],
            },
            CurvedEdge2d::Line {
                a: [10., 5.],
                b: [0., 5.],
            },
            CurvedEdge2d::Line {
                a: [0., 5.],
                b: [0., 0.],
            },
        ],
        holes: vec![],
    };
    let solid = Solid::extrude_curved_profile(&profile, 3., p()).unwrap();
    let edge = spline_edge(&solid);
    let collapsed = solid
        .sketch_reference_geometry(
            BRepSelection::Edge(edge),
            vertical_x(0.),
            SketchReferenceMethod::Project,
            p(),
        )
        .unwrap_err();
    assert!(collapsed.message.contains("collapses"));
    let supported = query(
        &solid,
        BRepSelection::Edge(edge),
        vertical_y(0.),
        SketchReferenceMethod::Project,
    );
    let SketchReferenceGeometry::Curve(CurvedEdge2d::BSpline(line)) =
        &supported.results[0].geometry
    else {
        panic!()
    };
    assert_eq!(line.poles, vec![[0., 0.], [5., 0.], [10., 0.]]);
    near(solid.volume_mm3().unwrap(), 150.);
}

#[test]
fn finite_cylinder_face_marks_proven_tangent_line_and_distinguishes_two_inside_from_empty_outside()
{
    let cylinder = Solid::cylinder(Default::default(), 5., 10., p()).unwrap();
    let side = cylinder
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| f.plane.is_none())
        .unwrap()
        .index;
    let tangent = query(
        &cylinder,
        BRepSelection::Face(side),
        vertical_x(5.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(tangent.results.len(), 1);
    assert!(tangent.results[0].tangent);
    assert!(matches!(
        tangent.results[0].geometry,
        SketchReferenceGeometry::Curve(CurvedEdge2d::Line { .. })
    ));
    let crossing = query(
        &cylinder,
        BRepSelection::Face(side),
        vertical_x(4.),
        SketchReferenceMethod::Intersect,
    );
    assert_eq!(crossing.results.len(), 2);
    assert!(crossing.results.iter().all(|c| !c.tangent));
    let empty = query(
        &cylinder,
        BRepSelection::Face(side),
        vertical_x(5.001),
        SketchReferenceMethod::Intersect,
    );
    assert!(empty.results.is_empty());
    near(cylinder.volume_mm3().unwrap(), 250. * std::f64::consts::PI);
}
