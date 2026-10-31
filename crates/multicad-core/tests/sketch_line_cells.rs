use multicad_core::{
    document::EntityId,
    sketch_regions::{Edge, ExactCurve, Extraction, PointLocation, extract_line_cells},
};

fn line(a: [f64; 2], b: [f64; 2]) -> Edge {
    Edge {
        authored: EntityId::new(),
        reversed: false,
        fragment: None,
        curve: ExactCurve::Line { a, b },
    }
}
fn square() -> Vec<Edge> {
    vec![
        line([0., 0.], [20., 0.]),
        line([20., 0.], [20., 20.]),
        line([20., 20.], [0., 20.]),
        line([0., 20.], [0., 0.]),
    ]
}
fn cells(edges: &[Edge]) -> Extraction {
    extract_line_cells(edges, 1e-7, None).unwrap()
}
fn areas(result: &Extraction, expected: &[f64]) {
    let mut actual: Vec<_> = result.regions.iter().map(|r| r.area_mm2).collect();
    actual.sort_by(f64::total_cmp);
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 1e-7, "{a} != {b}");
    }
}

#[test]
fn diagonal_yields_two_exact_triangles_sharing_the_authored_stroke() {
    let mut edges = square();
    let diagonal = line([0., 0.], [20., 20.]);
    edges.push(diagonal.clone());
    let before = edges.clone();
    let result = cells(&edges);
    areas(&result, &[200., 200.]);
    assert_eq!(before, edges);
    for region in &result.regions {
        assert_eq!(region.outer.edges.len(), 3);
        assert!(region.outer.intent.curves.contains(&diagonal.authored));
    }
    assert_ne!(result.regions[0].intent(), result.regions[1].intent());
    for p in [[15., 5.], [5., 15.]] {
        assert_eq!(
            result
                .regions
                .iter()
                .filter(|r| r.contains(p).unwrap())
                .count(),
            1
        );
    }
    assert!(
        result
            .regions
            .iter()
            .all(|r| r.classify([10., 10.]).unwrap() == PointLocation::Boundary)
    );
}

#[test]
fn edge_endings_and_interior_crossings_partition_without_authored_trimming() {
    let mut edges = square();
    edges.push(line([-5., 10.], [25., 10.]));
    areas(&cells(&edges), &[200., 200.]);
    edges.push(line([10., -5.], [10., 25.]));
    let result = cells(&edges);
    areas(&result, &[100., 100., 100., 100.]);
    for region in &result.regions {
        assert!(result.resolve(&region.intent()).is_some());
    }
}

#[test]
fn dangling_strokes_and_bridges_do_not_pollute_closed_wires() {
    let mut edges = square();
    edges.push(line([0., 0.], [10., 7.]));
    edges.push(line([20., 10.], [25., 10.]));
    areas(&cells(&edges), &[400.]);
    assert_eq!(cells(&edges).regions[0].outer.edges.len(), 5);
    let mut edges = square();
    let mut inner = square();
    for edge in &mut inner {
        let ExactCurve::Line { a, b } = &mut edge.curve else {
            unreachable!()
        };
        for p in [a, b] {
            *p = p.map(|v| 5. + v * 0.5);
        }
    }
    edges.extend(inner);
    edges.push(line([0., 10.], [5., 10.]));
    let result = cells(&edges);
    areas(&result, &[100., 300.]);
    assert_eq!(
        result.regions.iter().filter(|r| r.holes.len() == 1).count(),
        1
    );
}

#[test]
fn identities_survive_direction_order_and_unambiguous_dimensions() {
    let mut edges = square();
    edges.push(line([0., 0.], [20., 20.]));
    let before = cells(&edges);
    edges.reverse();
    for edge in &mut edges {
        let ExactCurve::Line { a, b } = &mut edge.curve else {
            unreachable!()
        };
        std::mem::swap(a, b);
        for p in [a, b] {
            p[0] *= 2.;
        }
    }
    let after = cells(&edges);
    areas(&after, &[400., 400.]);
    for region in &before.regions {
        assert!(after.resolve(&region.intent()).is_some());
    }
    edges.pop();
    let removed = cells(&edges);
    assert!(
        before
            .regions
            .iter()
            .any(|r| removed.resolve(&r.intent()).is_none())
    );
}

#[test]
fn duplicate_intents_are_ambiguous_and_never_resolve_to_the_first_match() {
    let mut result = cells(&square());
    let region = result.regions[0].clone();
    result.regions.push(region.clone());
    assert!(result.resolve(&region.intent()).is_none());
}

#[test]
fn invalid_overlaps_budget_and_cancellation_fail_without_partial_cells() {
    let mut edges = square();
    edges.push(line([5., 0.], [15., 0.]));
    assert!(extract_line_cells(&edges, 1e-7, None).is_err());
    assert!(extract_line_cells(&square(), 1e-7, Some(&|| true)).is_err());
    assert!(extract_line_cells(&square(), f64::NAN, None).is_err());
    assert!(extract_line_cells(&vec![line([0., 0.], [1., 1.]); 513], 1e-7, None).is_err());
    assert!(cells(&[line([0., 0.], [10., 10.])]).regions.is_empty());
}
