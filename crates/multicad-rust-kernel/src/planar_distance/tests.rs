use super::*;

const TOL: f64 = 1e-7;

fn vertex(p: Point) -> Shape {
    Shape {
        points: vec![p],
        ..Shape::default()
    }
}

fn edge(s: Segment) -> Shape {
    Shape {
        points: s.to_vec(),
        edges: vec![s],
        ..Shape::default()
    }
}

fn square([x0, y0, x1, y1]: [f64; 4], z: f64) -> Vec<Segment> {
    let corners = [[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]];
    corners
        .into_iter()
        .zip(corners.into_iter().cycle().skip(1))
        .map(|(a, b)| [a, b])
        .collect()
}

fn polygon(edges: Vec<Segment>) -> Shape {
    Shape {
        points: edges.iter().flatten().copied().collect(),
        edges: edges.clone(),
        faces: vec![Face {
            point: edges[0][0],
            normal: [0., 0., 1.],
            boundary: edges,
        }],
        solid: false,
    }
}

fn box_shape(min: Point, max: Point) -> Shape {
    let mut shape = Shape {
        solid: true,
        ..Shape::default()
    };
    for axis in 0..3 {
        for depth in [min[axis], max[axis]] {
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            let corner = |a, b| {
                let mut point = [0.; 3];
                point[axis] = depth;
                point[u] = a;
                point[v] = b;
                point
            };
            let [a, b, c, d] = [
                corner(min[u], min[v]),
                corner(max[u], min[v]),
                corner(max[u], max[v]),
                corner(min[u], max[v]),
            ];
            let boundary = vec![[a, b], [b, c], [c, d], [d, a]];
            let mut normal = [0.; 3];
            normal[axis] = if depth == min[axis] { -1. } else { 1. };
            shape.points.extend([a, b, c, d]);
            shape.edges.extend(&boundary);
            shape.faces.push(Face {
                point: a,
                normal,
                boundary,
            });
        }
    }
    shape
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-9, "{actual} != {expected}");
}

fn measure_one(a: Shape, b: Shape) -> Measurement {
    let result = minimum(&[a], &[b], TOL).unwrap();
    for &[a, b] in &result.witnesses {
        near(length(sub(a, b)), result.distance);
    }
    result
}

#[test]
fn finite_points_and_segments_use_endpoints_and_skew_interior_witnesses() {
    let a = [0., 0., 0.];
    let b = [3., 4., 0.];
    let points = measure_one(vertex(a), vertex(b));
    assert_eq!(points.witnesses, [[a, b]]);
    near(points.distance, 5.);

    let p = [1., 4., 1.];
    let s = [[3., 2., 1.], [7., 2., 1.]];
    let one = measure_one(vertex(p), edge(s));
    let reversed = measure_one(edge(s), vertex(p));
    assert_eq!(one.witnesses, [[p, s[0]]]);
    assert_eq!(reversed.witnesses, [[s[0], p]]);
    near(one.distance, 8_f64.sqrt());

    let first = [[-3., 0., 0.], [3., 0., 0.]];
    let second = [[0., -2., 2.], [0., 2., 2.]];
    let interior = measure_one(edge(first), edge(second));
    assert_eq!(interior.witnesses, [[[0., 0., 0.], [0., 0., 2.]]]);
    near(interior.distance, 2.);
}

#[test]
fn trimmed_faces_keep_holes_and_crossing_edges_get_a_coincident_witness() {
    let mut outline = square([0., 0., 10., 10.], 0.);
    outline.extend(square([2., 2., 8., 8.], 0.));
    let p = [5., 5., 1.];
    let near_hole = measure_one(vertex(p), polygon(outline));
    near(near_hole.distance, 10_f64.sqrt());
    assert!(
        near_hole
            .witnesses
            .iter()
            .all(|[a, b]| *a == p && b[2] == 0.)
    );

    let crossing = measure_one(
        edge([[1., 1., -2.], [1., 1., 3.]]),
        polygon(square([0., 0., 10., 10.], 0.)),
    );
    assert_eq!(crossing.witnesses.len(), 1);
    assert_eq!(crossing.witnesses[0][0], crossing.witnesses[0][1]);
    near(crossing.witnesses[0][0][2], 0.);

    let normal = measure_one(
        polygon(square([0., 0., 10., 10.], 0.)),
        polygon(square([3., 4., 7., 8.], 5.)),
    );
    near(normal.distance, 5.);
    assert!(normal.witnesses.iter().all(|[a, b]| a[..2] == b[..2]));
}

#[test]
fn solid_clearance_distinguishes_cavity_touching_and_contained_geometry() {
    let large = || box_shape([0.; 3], [10.; 3]);
    let inner = box_shape([3.; 3], [5.; 3]);
    let overlap = measure_one(large(), inner);
    assert_eq!(overlap.distance, 0.);
    assert!(overlap.inner_solution);
    assert!(overlap.witnesses.iter().all(|[a, b]| a == b));
    let touching = measure_one(large(), box_shape([10., 0., 0.], [13., 2., 2.]));
    assert_eq!(touching.distance, 0.);
    assert!(!touching.inner_solution);

    let mut shell = large();
    let void = box_shape([2.; 3], [8.; 3]);
    shell.points.extend(void.points);
    shell.edges.extend(void.edges);
    shell.faces.extend(void.faces);
    let clearance = measure_one(shell, box_shape([3.; 3], [5.; 3]));
    near(clearance.distance, 1.);
    assert!(!clearance.inner_solution);
}

#[test]
fn member_clearance_ignores_remote_components_and_reports_witness_truncation() {
    let bodies = [box_shape([0.; 3], [1.; 3]), box_shape([20.; 3], [21.; 3])];
    let r = minimum(&bodies, &[vertex([2., 0.5, 0.5])], TOL).unwrap();
    assert_eq!(r.witnesses, [[[1., 0.5, 0.5], [2., 0.5, 0.5]]]);

    let points: Vec<_> = (0..70).map(|i| [f64::from(i), 0., 0.]).collect();
    let a = Shape {
        points: points.clone(),
        ..Shape::default()
    };
    let b = Shape {
        points: points.into_iter().map(|[x, y, z]| [x, y + 2., z]).collect(),
        ..Shape::default()
    };
    let r = measure_one(a, b);
    assert_eq!(r.witnesses.len(), 64);
    assert_eq!(r.total_solutions, 70);
}

#[test]
fn grazing_rays_are_retried_and_ambiguous_geometry_is_unresolved() {
    let point = [0.; 3];
    let mut shape = box_shape([-1.; 3], [1., 0.371, 0.127]);
    let parallel = [0.371, -1., 0.];
    shape.faces.insert(
        0,
        Face {
            point: [0., 3., 0.],
            normal: parallel.map(|v| v / length(parallel)),
            boundary: vec![],
        },
    );
    assert!(matches!(
        classify(point, &shape, TOL, &mut Work(1000)),
        Ok(Location::Inside)
    ));
    let mut ambiguous = Shape::default();
    for p in [
        [1., 0.371, 0.127],
        [0.229, 1., -0.513],
        [-0.631, 0.217, 1.],
        [1., -0.719, 0.421],
        [-0.319, 1., 0.815],
        [0.867, 0.649, 1.],
    ] {
        let boundary = square([p[0], p[1], p[0] + 0.01, p[1] + 0.01], p[2]);
        ambiguous.faces.push(Face {
            point: p,
            normal: [0., 0., 1.],
            boundary,
        });
    }
    assert!(matches!(
        classify(point, &ambiguous, TOL, &mut Work(1000)),
        Err(DistanceError::Unresolved)
    ));
}

#[test]
fn representative_limit_is_enforced_and_rounding_never_hides_the_shortest_pair() {
    let mut closest = Closest {
        tolerance: TOL,
        distance: f64::INFINITY,
        witnesses: BTreeMap::new(),
        inner: false,
    };
    closest.add([0.; 3], [0., 2., 0.]).unwrap();
    closest.add([0.; 3], [0., 2. - 1e-9, 0.]).unwrap();
    assert_eq!(closest.witnesses.len(), 1);
    let shortest = closest.witnesses.values().next().unwrap();
    assert_eq!(shortest.0, 2. - 1e-9);
    assert_eq!(shortest.1[1], [0., 2. - 1e-9, 0.]);

    for x in 1..MAX_SOLUTIONS {
        let x = x as f64;
        closest.add([x, 0., 0.], [x, 2., 0.]).unwrap();
    }
    let x = MAX_SOLUTIONS as f64;
    assert_eq!(
        closest.add([x, 0., 0.], [x, 2., 0.]),
        Err(DistanceError::Limit)
    );
}

#[test]
fn invalid_and_over_budget_inputs_do_not_produce_measurements() {
    let point = vertex([1.; 3]);
    assert_eq!(
        minimum(&[], std::slice::from_ref(&point), TOL),
        Err(DistanceError::Invalid)
    );
    assert_eq!(
        minimum(&[vertex([f64::NAN; 3])], std::slice::from_ref(&point), TOL),
        Err(DistanceError::Invalid)
    );
    assert_eq!(
        minimum(&[edge([[1.; 3]; 2])], std::slice::from_ref(&point), TOL),
        Err(DistanceError::Invalid)
    );
    assert_eq!(
        minimum(
            &[Shape {
                solid: true,
                ..vertex([0.; 3])
            }],
            std::slice::from_ref(&point),
            TOL
        ),
        Err(DistanceError::Invalid),
    );
    let mut invalid = polygon(square([0., 0., 1., 1.], 0.));
    invalid.faces[0].normal = [0.; 3];
    assert_eq!(
        minimum(&[invalid], std::slice::from_ref(&point), TOL),
        Err(DistanceError::Invalid)
    );
    assert_eq!(
        measure(
            std::slice::from_ref(&point),
            std::slice::from_ref(&point),
            TOL,
            1
        ),
        Err(DistanceError::Limit)
    );
}
