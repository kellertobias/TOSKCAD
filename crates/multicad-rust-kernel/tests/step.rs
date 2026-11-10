use multicad_rust_kernel::{
    Circle, Rational,
    circular_pockets::CircularPocket,
    cylinder::AxialCylinder,
    orthogonal::{Box3, Operation, boolean},
    orthogonal_boundary,
    polygon::PolygonPrism,
    prism::Prism,
    step::{
        StepError, write_axial_cylinder, write_box, write_orthogonal, write_placed_cylinder,
        write_pocketed_polygon_prism, write_polygon_prism, write_rectangular_prism,
    },
    step_read::{ImportProfile, ReadError, read_analytic_solid, read_import_solid},
};
use std::collections::BTreeMap;

const ORIGIN: [f64; 3] = [0.; 3];
const X: [f64; 3] = [1., 0., 0.];
const Y: [f64; 3] = [0., 1., 0.];
const Z: [f64; 3] = [0., 0., 1.];

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn ring(points: &[[i64; 2]]) -> Vec<[Rational; 2]> {
    points.iter().map(|point| point.map(n)).collect()
}

fn entities(step: &str) -> impl Iterator<Item = (&str, &str)> {
    step.lines()
        .filter_map(|line| line.strip_prefix('#'))
        .map(|line| line.strip_suffix(';').unwrap().split_once('=').unwrap())
}

fn count(step: &str, kind: &str) -> usize {
    entities(step)
        .filter(|(_, body)| body.split_once('(').unwrap().0 == kind)
        .count()
}

fn closed_edges(step: &str, edge_count: usize) {
    let mut uses: BTreeMap<&str, [usize; 2]> = entities(step)
        .filter(|(_, body)| body.starts_with("EDGE_CURVE("))
        .map(|(id, _)| (id, [0, 0]))
        .collect();
    assert_eq!(uses.len(), edge_count);
    for (_, body) in entities(step) {
        if let Some(oriented) = body.strip_prefix("ORIENTED_EDGE('',*,*,#") {
            let (edge, sense) = oriented.split_once(',').unwrap();
            let orientation = match sense {
                ".F.)" => 0,
                ".T.)" => 1,
                _ => panic!("Invalid edge orientation {sense}"),
            };
            uses.get_mut(edge).unwrap()[orientation] += 1;
        }
    }
    assert!(uses.values().all(|&counts| counts == [1, 1]));
}

fn cartesian_points(step: &str) -> impl Iterator<Item = [f64; 3]> {
    entities(step).filter_map(|(_, body)| {
        let coordinates = body
            .strip_prefix("CARTESIAN_POINT('',(")?
            .strip_suffix("))")
            .unwrap();
        let coords: Vec<_> = coordinates
            .split(',')
            .map(|coordinate| coordinate.parse::<f64>().unwrap())
            .collect();
        Some(coords.try_into().unwrap())
    })
}

#[test]
fn blind_pockets_reimport_with_exact_rims_on_both_sides_of_a_placed_panel() {
    let through = Circle::new([n(8), n(5)], n(1)).unwrap();
    let prism = PolygonPrism::with_circular_holes(
        vec![ring(&[[0, 0], [12, 0], [12, 10], [0, 10]])],
        vec![through],
        n(6),
    )
    .unwrap();
    let pockets = [
        CircularPocket {
            center_mm: [3., 5.],
            radius_mm: 2.,
            depth_mm: 1.,
            end: false,
        },
        CircularPocket {
            center_mm: [3., 5.],
            radius_mm: 1.5,
            depth_mm: 2.,
            end: true,
        },
    ];
    let (s, c) = 0.731_f64.sin_cos();
    let u = [c, s, 0.];
    let v = [0., 0., 1.];
    let origin = [-810., 10., 55.];
    for reverse in [false, true] {
        let step = write_pocketed_polygon_prism(&prism, &pockets, origin, u, v, reverse).unwrap();
        assert_eq!(count(&step, "CYLINDRICAL_SURFACE"), 3);
        assert_eq!(count(&step, "CIRCLE"), 6);
        assert_eq!(count(&step, "PLANE"), 8);
        closed_edges(&step, 21);
        let verified = read_analytic_solid(&step, 1e-7).unwrap();
        assert_eq!(verified.face_count, 11);
        assert_eq!(verified.circles.len(), 6);
        assert!((verified.volume_mm3 - (720. - 14.5 * std::f64::consts::PI)).abs() < 1e-9);
        let (readback, imported) = read_import_solid(&step, 1e-7).unwrap();
        assert_eq!(verified, readback);
        let ImportProfile::PocketedPolygon {
            rings,
            circular_holes,
            pockets,
        } = imported.profile
        else {
            panic!("blind pockets were not imported");
        };
        assert!((imported.height_mm - 6.).abs() < 1e-10);
        assert_eq!(rings.len(), 1);
        assert_eq!(circular_holes.len(), 1);
        assert_eq!(pockets.len(), 2);
        assert_ne!(pockets[0].end, pockets[1].end);
        for (radius, depth) in [(2., 1.), (1.5, 2.)] {
            let pocket = pockets
                .iter()
                .find(|p| (p.radius_mm - radius).abs() < 1e-10)
                .unwrap();
            assert!((pocket.depth_mm - depth).abs() < 1e-10);
        }
    }
    let invalid = [CircularPocket {
        depth_mm: 6.,
        ..pockets[0]
    }];
    assert_eq!(
        write_pocketed_polygon_prism(&prism, &invalid, origin, u, v, false),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn blind_pocket_import_rejects_opposite_floors_with_intersecting_volumes() {
    let prism = PolygonPrism::with_circular_holes(
        vec![ring(&[[0, 0], [12, 0], [12, 10], [0, 10]])],
        vec![Circle::new([n(8), n(5)], n(1)).unwrap()],
        n(6),
    )
    .unwrap();
    let pocket = CircularPocket {
        center_mm: [3., 5.],
        radius_mm: 2.,
        depth_mm: 1.,
        end: false,
    };
    let step = write_pocketed_polygon_prism(
        &prism,
        &[
            pocket,
            CircularPocket {
                radius_mm: 1.,
                depth_mm: 2.,
                end: true,
                ..pocket
            },
        ],
        ORIGIN,
        X,
        Y,
        false,
    )
    .unwrap();
    let intersecting = step
        .lines()
        .map(|line| {
            if line.contains("=CARTESIAN_POINT(") {
                line.replace(",1.0000000000000000E0));", ",5.0000000000000000E0));")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(step.trim(), intersecting);
    assert!(read_analytic_solid(&intersecting, 1e-7).is_ok());
    assert_eq!(
        read_import_solid(&intersecting, 1e-7),
        Err(ReadError::Unsupported)
    );
}

#[test]
fn planar_pocket_import_retains_orthogonal_recognition() {
    let body = boolean(
        &[Box3 {
            min: ORIGIN,
            max: [12., 10., 6.],
        }],
        &[Box3 {
            min: [2., 2., 4.],
            max: [8., 6., 7.],
        }],
        Operation::Subtract,
        0.,
    )
    .unwrap()
    .unwrap();
    let boundary = orthogonal_boundary::build(body.quads()).unwrap();
    let step = write_orthogonal(&boundary, |point| point).unwrap();
    let (readback, solid) = read_import_solid(&step, 1e-7).unwrap();
    assert_eq!(readback.volume_mm3, 672.);
    assert!(matches!(solid.profile, ImportProfile::Orthogonal { .. }));
}

#[test]
fn polygon_step_refuses_a_far_cap_whose_edges_collapse_after_placement() {
    let prism = PolygonPrism::new(
        vec![ring(&[[0, 0], [1, 0], [1, 1], [0, 1]])],
        n(100_000_000_000_000_000),
    )
    .unwrap();
    let a = (0.5_f64).sqrt();
    let b = (1. / 6_f64).sqrt();
    assert_eq!(
        write_polygon_prism(&prism, ORIGIN, [a, -a, 0.], [b, b, -2. * b], false),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn box_step_is_deterministic_and_uses_a_shared_manifold_shell() {
    let size = [n(12), n(8), Rational::new(10, 3).unwrap()];
    let first = write_box(size, ORIGIN, X, Y).unwrap();
    let second = write_box(size, ORIGIN, X, Y).unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("ISO-10303-21;\nHEADER;\n"));
    assert!(first.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    assert!(first.contains("AUTOMOTIVE_DESIGN"));
    assert!(first.contains("SI_UNIT(.MILLI.,.METRE.)"));
    assert_eq!(count(&first, "MANIFOLD_SOLID_BREP"), 1);
    assert_eq!(count(&first, "ADVANCED_FACE"), 6);
    assert_eq!(count(&first, "FACE_BOUND"), 0);
    assert_eq!(count(&first, "PLANE"), 6);
    assert_eq!(count(&first, "LINE"), 12);
    assert!(cartesian_points(&first).any(|point| point == [12., 8., 10. / 3.]));
    closed_edges(&first, 12);
}

#[test]
fn rectangular_prism_keeps_circular_cuts_and_rejects_a_rounded_exterior() {
    let stock = Prism::rectangular(1, [n(10), n(8), n(3)]).unwrap();
    let bored = stock
        .subtract_cylinder(2, Circle::new([n(4), n(5)], n(2)).unwrap(), n(-1), n(4))
        .unwrap();
    let encoded = write_rectangular_prism(&bored, ORIGIN, X, Y).unwrap();
    assert_eq!(count(&encoded, "CYLINDRICAL_SURFACE"), 1);
    assert_eq!(count(&encoded, "CIRCLE"), 2);
    assert_eq!(count(&encoded, "ADVANCED_FACE"), 7);
    closed_edges(&encoded, 15);
    let rounded = Prism::extrude(1, n(10), n(4), n(3)).unwrap();
    assert_eq!(
        write_rectangular_prism(&rounded, ORIGIN, X, Y),
        Err(StepError::Unsupported)
    );
}

#[test]
fn placed_signed_concave_prism_retains_its_separate_cap_hole_loops() {
    let prism = PolygonPrism::new(
        vec![
            ring(&[[0, 0], [8, 0], [8, 3], [4, 3], [4, 7], [0, 7]]),
            ring(&[[1, 1], [1, 2], [2, 2], [2, 1]]),
        ],
        n(3),
    )
    .unwrap();
    // XZ profile normal is negative Y. A reverse sweep goes toward positive Y.
    let origin = [10., 20., 30.];
    let step = write_polygon_prism(&prism, origin, X, Z, true).unwrap();
    assert_eq!(count(&step, "ADVANCED_FACE"), 12);
    assert_eq!(count(&step, "FACE_OUTER_BOUND"), 12);
    assert_eq!(count(&step, "FACE_BOUND"), 2);
    assert_eq!(count(&step, "EDGE_LOOP"), 14);
    assert_eq!(count(&step, "VERTEX_POINT"), 20);
    assert_eq!(count(&step, "CLOSED_SHELL"), 1);
    closed_edges(&step, 30);
    assert!(cartesian_points(&step).any(|point| point == [14., 23., 37.]));
    assert!(cartesian_points(&step).any(|point| point == [12., 23., 32.]));
}

#[test]
fn mixed_polygon_prism_exports_shared_round_holes_in_both_sweep_directions() {
    let prism = PolygonPrism::with_circular_holes(
        vec![
            ring(&[[0, 0], [12, 0], [12, 8], [0, 8]]),
            ring(&[[1, 1], [1, 2], [2, 2], [2, 1]]),
        ],
        vec![
            Circle::new([n(4), n(3)], Rational::new(5, 4).unwrap()).unwrap(),
            Circle::new([n(8), n(3)], n(2)).unwrap(),
        ],
        Rational::new(10, 3).unwrap(),
    )
    .unwrap();
    let origin = [10., 20., 30.];
    for reverse in [false, true] {
        let step = write_polygon_prism(&prism, origin, X, Z, reverse).unwrap();
        assert_eq!(count(&step, "ADVANCED_FACE"), 12);
        assert_eq!(count(&step, "FACE_BOUND"), 6);
        assert_eq!(count(&step, "CIRCLE"), 4);
        assert_eq!(count(&step, "CYLINDRICAL_SURFACE"), 2);
        assert_eq!(
            entities(&step)
                .filter(|(_, body)| body.starts_with("ADVANCED_FACE(") && body.ends_with(".F.)"))
                .count(),
            2
        );
        let end = 20. + if reverse { 10. / 3. } else { -10. / 3. };
        assert!(cartesian_points(&step).any(|point| point == [15.25, end, 33.]));
        closed_edges(&step, 30);
    }
    assert_eq!(
        write_polygon_prism(&prism, [1e100, 0., 0.], X, Z, false),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn axial_cylinder_exports_two_fractional_rims_and_one_periodic_wall() {
    let cylinder = AxialCylinder::new(
        1,
        Circle::new([n(4), n(6)], Rational::new(5, 4).unwrap()).unwrap(),
        n(-3),
        Rational::new(10, 3).unwrap(),
    )
    .unwrap();
    let step = write_axial_cylinder(&cylinder, Y).unwrap();
    assert_eq!(step, write_axial_cylinder(&cylinder, Y).unwrap());
    assert_eq!(count(&step, "ADVANCED_FACE"), 3);
    assert_eq!(count(&step, "PLANE"), 2);
    assert_eq!(count(&step, "FACE_BOUND"), 0);
    assert_eq!(count(&step, "CIRCLE"), 2);
    assert_eq!(count(&step, "CYLINDRICAL_SURFACE"), 1);
    assert_eq!(count(&step, "LINE"), 1);
    assert_eq!(
        entities(&step)
            .filter(|(_, body)| body.starts_with("CIRCLE("))
            .filter(|(_, body)| body.ends_with(",1.2500000000000000E0)"))
            .count(),
        2
    );
    assert!(cartesian_points(&step).any(|point| point == [4., 7.25, -3.]));
    assert!(cartesian_points(&step).any(|point| point == [4., 7.25, 1. / 3.]));
    closed_edges(&step, 3);
    assert_eq!(
        write_axial_cylinder(&cylinder, Z),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_axial_cylinder(&cylinder, [1., 0., 1e-8]),
        Err(StepError::InvalidPlacement)
    );
}

#[test]
fn placed_cylinder_step_retains_tilted_rims_and_exact_projected_bounds() {
    let cylinder =
        AxialCylinder::new(1, Circle::new([n(4), n(6)], n(2)).unwrap(), n(-3), n(5)).unwrap();
    let encoded = write_placed_cylinder(&cylinder, [10., 20., 30.], X, [0., 0.8, 0.6]).unwrap();
    let read = read_analytic_solid(&encoded, 1e-8).unwrap();
    assert_eq!(read.face_count, 3);
    assert_eq!(read.circles.len(), 2);
    assert!((read.volume_mm3 - 20. * std::f64::consts::PI).abs() < 1e-8);
    for (actual, expected) in read
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([8., 15.4, 28.8, 12., 21.6, 35.2])
    {
        assert!((actual - expected).abs() < 1e-8);
    }
    for center in [[10., 20., 30.], [10., 17., 34.]] {
        assert!(read.circles.iter().any(|rim| {
            let alignment = rim.normal[1] * -0.6 + rim.normal[2] * 0.8;
            rim.center_mm == center && (alignment.abs() - 1.).abs() < 1e-8 && rim.radius_mm == 2.
        }));
    }
}

#[test]
fn step_rejects_invalid_or_degenerate_placements() {
    let center = Circle::new([n(100_000_000_000_000_000), n(0)], n(1)).unwrap();
    let collapsed_rim = AxialCylinder::new(1, center, n(0), n(2)).unwrap();
    assert_eq!(
        write_axial_cylinder(&collapsed_rim, X),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        write_box([n(4), n(4), n(4)], ORIGIN, X, X),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_box([n(4), n(4), n(4)], [f64::NAN, 0., 0.], X, Y),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_box([n(4), n(4), n(4)], [1e100; 3], X, Y),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        write_box([n(4), n(0), n(4)], ORIGIN, X, Y),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn invalid_placed_circular_sweeps_refuse_nonpositive_or_overlapping_dimensions() {
    use multicad_rust_kernel::step::{StepError, write_placed_circular_sweep};
    for (outer, inner, height) in [
        (4., Some(4.), 3.),
        (4., Some(5.), 3.),
        (4., Some(f64::NAN), 3.),
        (0., None, 3.),
        (4., None, -1.),
    ] {
        assert_eq!(
            write_placed_circular_sweep(outer, inner, height, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
            Err(StepError::InvalidGeometry),
        );
    }
}

#[test]
fn directed_draft_step_shares_planar_edges_and_retains_the_hole() {
    use multicad_rust_kernel::{
        linear_sweep::Sweep,
        step::write_linear_sweep,
        step_read::{ImportProfile, read_import_solid},
    };

    let sweep = Sweep::new(
        vec![
            vec![[0., 0.], [10., 0.], [10., 6.], [0., 6.]],
            vec![[3., 2.], [3., 4.], [5., 4.], [5., 2.]],
        ],
        vec![
            vec![[1., 2.], [13., 2.], [13., 10.], [1., 10.]],
            vec![[5.5, 5.5], [5.5, 6.5], [6.5, 6.5], [6.5, 5.5]],
        ],
        5.,
    )
    .unwrap();
    let placement = [10., -7., 4.];
    let encoded = write_linear_sweep(&sweep, placement, Y, Z).unwrap();
    assert_eq!(
        encoded,
        write_linear_sweep(&sweep, placement, Y, Z).unwrap()
    );
    closed_edges(&encoded, 24);
    assert_eq!(count(&encoded, "ADVANCED_FACE"), 10);
    assert_eq!(count(&encoded, "PLANE"), 10);
    assert_eq!(count(&encoded, "FACE_BOUND"), 2);

    let read = read_analytic_solid(&encoded, 1e-8).unwrap();
    assert_eq!(read.solid_count, 1);
    assert_eq!(read.face_count, 10);
    assert_eq!(read.bounds_mm, [[10., -7., 4.], [15., 6., 14.]]);
    assert_eq!(read.edges_mm.len(), 24);
    assert!((read.volume_mm3 - 375.).abs() < 1e-8);

    let rotation = 0.43_f64;
    let tilted_u = [rotation.cos(), rotation.sin(), 0.];
    for (u, v) in [(Y, Z), (tilted_u, Z)] {
        let encoded = write_linear_sweep(&sweep, placement, u, v).unwrap();
        let (expected, solid) = read_import_solid(&encoded, 1e-8).unwrap();
        let ImportProfile::LinearSweep { base, top } = solid.profile else {
            panic!("draft must retain paired sections");
        };
        assert_eq!(base.len(), 2);
        assert_eq!(top.len(), 2);
        let rebuilt = Sweep::new(base, top, solid.height_mm).unwrap();
        assert!((rebuilt.volume_mm3() - 375.).abs() < 1e-8);
        let encoded = write_linear_sweep(&rebuilt, solid.origin_mm, solid.u, solid.v).unwrap();
        let read = read_analytic_solid(&encoded, 1e-8).unwrap();
        assert_eq!(read.face_count, expected.face_count);
        assert_eq!(read.edges_mm.len(), expected.edges_mm.len());
        for (actual, expected) in read
            .bounds_mm
            .into_iter()
            .flatten()
            .zip(expected.bounds_mm.into_iter().flatten())
        {
            assert!((actual - expected).abs() < 1e-8);
        }
    }
    assert_eq!(
        write_linear_sweep(&sweep, ORIGIN, X, X),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_linear_sweep(&sweep, [1e100; 3], X, Y),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn oblique_plane_trim_step_retains_both_sloped_caps_and_the_through_hole() {
    use multicad_rust_kernel::{
        step::write_trimmed_sweep,
        step_read::{ImportProfile, read_import_solid},
        trimmed_sweep::TrimmedSweep,
    };

    let sweep = TrimmedSweep::new(
        vec![
            vec![[0., 0.], [10., 0.], [10., 6.], [0., 6.]],
            vec![[3., 2.], [3., 4.], [5., 4.], [5., 2.]],
        ],
        [0.6, 0., 0.8],
        [[-2., 0.05, 0.1], [5., 0.25, -0.1]],
        1e-5,
    )
    .unwrap();
    let placement = [10., -7., 4.];
    let encoded = write_trimmed_sweep(&sweep, placement, Y, Z).unwrap();
    assert_eq!(
        encoded,
        write_trimmed_sweep(&sweep, placement, Y, Z).unwrap()
    );
    closed_edges(&encoded, 24);
    assert_eq!(count(&encoded, "ADVANCED_FACE"), 10);
    assert_eq!(count(&encoded, "PLANE"), 10);
    assert_eq!(count(&encoded, "FACE_BOUND"), 2);

    let read = read_analytic_solid(&encoded, 1e-8).unwrap();
    assert_eq!(read.solid_count, 1);
    assert_eq!(read.face_count, 10);
    assert_eq!(read.edges_mm.len(), 24);
    for (actual, expected) in read
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([8.4, -8.2, 4., 16., 7.5, 10.])
    {
        assert!((actual - expected).abs() < 1e-8);
    }
    assert!((read.volume_mm3 - 332.16).abs() < 1e-8);
    // Pick a generator from the hole, which proves the cap trims are
    // placed independently rather than merely recovering the same volume.
    let rim = [[8.68, -4.99, 6.], [14.44, -0.67, 6.]];
    assert!(read.edges_mm.iter().any(|edge| {
        [*edge, [edge[1], edge[0]]].iter().any(|edge| {
            edge.iter()
                .flatten()
                .zip(rim.iter().flatten())
                .all(|(a, b)| (a - b).abs() < 1e-8)
        })
    }));
    let (_, imported) = read_import_solid(&encoded, 1e-8).unwrap();
    let ImportProfile::TrimmedSweep {
        rings,
        direction,
        travels,
    } = imported.profile
    else {
        panic!("the cap slope must survive import");
    };
    assert_eq!(rings.len(), 2);
    assert_eq!(travels[0], [0.; 3]);
    let rebuilt = TrimmedSweep::new(rings, direction, travels, 1e-8).unwrap();
    assert!((rebuilt.volume_mm3() - 332.16).abs() < 1e-8);
    let encoded =
        write_trimmed_sweep(&rebuilt, imported.origin_mm, imported.u, imported.v).unwrap();
    let roundtrip = read_analytic_solid(&encoded, 1e-8).unwrap();
    assert_eq!(roundtrip.face_count, read.face_count);
    assert!(read.edges_mm.iter().all(|old| {
        roundtrip.edges_mm.iter().any(|new| {
            [*new, [new[1], new[0]]].iter().any(|new| {
                old.iter()
                    .flatten()
                    .zip(new.iter().flatten())
                    .all(|(a, b)| (a - b).abs() < 1e-8)
            })
        })
    }));
    assert_eq!(
        write_trimmed_sweep(&sweep, ORIGIN, X, X),
        Err(StepError::InvalidPlacement)
    );
}

#[test]
fn oblique_trim_can_terminate_at_a_vertical_plane() {
    use multicad_rust_kernel::{step::write_trimmed_sweep, trimmed_sweep::TrimmedSweep};

    let sweep = TrimmedSweep::new(
        vec![vec![[0., 0.], [10., 0.], [10., 6.], [0., 6.]]],
        [0.6, 0., 0.8],
        [[0.; 3], [20., -1. / 0.6, 0.]],
        1e-5,
    )
    .unwrap();
    let encoded = write_trimmed_sweep(&sweep, ORIGIN, X, Y).unwrap();
    let read = read_analytic_solid(&encoded, 1e-8).unwrap();
    closed_edges(&encoded, 12);
    assert_eq!(read.face_count, 6);
    assert_eq!(read.bounds_mm, [[0.; 3], [12., 6., 16.]]);
    assert!((read.volume_mm3 - 560.).abs() < 1e-8);
}
