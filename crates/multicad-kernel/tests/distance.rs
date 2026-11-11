use multicad_kernel::*;
fn p() -> GeometryPolicy {
    Default::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn moved(s: &Solid, offset: [f64; 3]) -> Solid {
    s.transformed(RigidTransform {
        translation_mm: offset,
        ..Default::default()
    })
    .unwrap()
}
fn face(s: &Solid, n: [f64; 3]) -> u32 {
    s.faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane
                .is_some_and(|(_, normal)| normal.iter().zip(n).all(|(a, b)| (a - b).abs() < 1e-9))
        })
        .unwrap()
        .index
}
#[test]
fn finite_selected_faces_do_not_become_infinite_supporting_planes() {
    let a = Solid::box_solid([10., 10., 10.], p()).unwrap();
    let b = moved(&a, [30., 0., 0.]);
    let topa = face(&a, [0., 0., 1.]);
    let topb = face(&b, [0., 0., 1.]);
    let result = a
        .minimum_distance(BRepSelection::Face(topa), &b, BRepSelection::Face(topb))
        .unwrap();
    near(result.distance_mm, 20.);
    assert!(result.witnesses_complete);
    assert!(result.total_solutions.is_some_and(|total| total > 0));
    for w in result.witnesses {
        near(w.b_mm[0] - w.a_mm[0], 20.);
        near(w.b_mm[2], 10.);
        near(w.a_mm[2], 10.);
    }
}
#[test]
fn same_part_two_distinct_faces_and_points_are_measured_independently() {
    let a = Solid::box_solid([10., 8., 3.], p()).unwrap();
    let bottom = face(&a, [0., 0., -1.]);
    let top = face(&a, [0., 0., 1.]);
    let result = a
        .minimum_distance(BRepSelection::Face(bottom), &a, BRepSelection::Face(top))
        .unwrap();
    near(result.distance_mm, 3.);
    for w in result.witnesses {
        near(w.b_mm[2] - w.a_mm[2], 3.);
    }
    let vertices = a.display_vertices().unwrap();
    let va = &vertices[0];
    let vb = vertices
        .iter()
        .max_by(|x, y| {
            let dist = |v: &DisplayVertex| {
                v.position_mm
                    .iter()
                    .zip(va.position_mm)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
            };
            dist(x).total_cmp(&dist(y))
        })
        .unwrap();
    let result = a
        .minimum_distance(
            BRepSelection::Vertex(va.index),
            &a,
            BRepSelection::Vertex(vb.index),
        )
        .unwrap();
    near(result.distance_mm, 173_f64.sqrt());
}
#[test]
fn whole_shape_clearance_uses_current_transformed_geometry_including_contact_and_containment() {
    let a = Solid::box_solid([10., 10., 10.], p()).unwrap();
    let b = moved(&a, [30., 0., 0.]);
    near(
        a.minimum_distance(BRepSelection::Whole, &b, BRepSelection::Whole)
            .unwrap()
            .distance_mm,
        20.,
    );
    let touching = moved(&a, [10., 0., 0.]);
    near(
        a.minimum_distance(BRepSelection::Whole, &touching, BRepSelection::Whole)
            .unwrap()
            .distance_mm,
        0.,
    );
    let inner = moved(&Solid::box_solid([2., 2., 2.], p()).unwrap(), [2., 2., 2.]);
    let result = a
        .minimum_distance(BRepSelection::Whole, &inner, BRepSelection::Whole)
        .unwrap();
    near(result.distance_mm, 0.);
    assert!(result.inner_solution);
}
#[test]
fn curved_surface_distance_is_exact_and_out_of_range_selectors_fail_without_mutation() {
    let cylinder = Solid::cylinder(PlaneFrame::default(), 5., 10., p()).unwrap();
    let b = moved(&cylinder, [14., 0., 0.]);
    near(
        cylinder
            .minimum_distance(BRepSelection::Whole, &b, BRepSelection::Whole)
            .unwrap()
            .distance_mm,
        4.,
    );
    assert!(
        cylinder
            .minimum_distance(BRepSelection::Face(u32::MAX), &b, BRepSelection::Whole)
            .is_err()
    );
    near(cylinder.volume_mm3().unwrap(), std::f64::consts::PI * 250.);
}
#[test]
fn finite_edge_endpoints_limit_distance_instead_of_infinite_lines() {
    use multicad_core::document::EntityId;
    use multicad_core::references::{AnalyticGeometry, TopologyKind};
    let a = Solid::box_solid([10., 8., 3.], p()).unwrap();
    let edge=a.topology_snapshot(EntityId::new(),0).unwrap().elements.into_iter().find(|e|e.key.kind==TopologyKind::Edge&&matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm})if a_mm[1].abs()<1e-9&&b_mm[1].abs()<1e-9&&a_mm[2].abs()<1e-9&&b_mm[2].abs()<1e-9)).unwrap().key.ordinal;
    let b = moved(&a, [20., 0., 4.]);
    let result = a
        .minimum_distance(BRepSelection::Edge(edge), &b, BRepSelection::Edge(edge))
        .unwrap();
    near(result.distance_mm, 116_f64.sqrt());
    for w in result.witnesses {
        near(w.a_mm[0], 10.);
        near(w.b_mm[0], 20.);
    }
}
