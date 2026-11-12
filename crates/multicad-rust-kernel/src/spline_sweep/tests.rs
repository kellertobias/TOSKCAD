use super::*;
use crate::{
    manufacturing::certify,
    revolution::dot,
    spur_gear, step,
    step_read::{ImportProfile, ReadError, read_import_compound, read_import_solid},
};

const TOLERANCE: f64 = 1e-7;
const OPTIONS: Tessellation = Tessellation {
    deflection: 0.003,
    angle: 0.15,
    triangle_limit: 50_000,
};

fn close(value: f64, expected: f64, tolerance: f64) {
    assert!(
        (value - expected).abs() <= tolerance,
        "{value} != {expected} within {tolerance}"
    );
}

fn line(a: Point, b: Point) -> Edge {
    Edge::Analytic(normal_sweep::Edge::Line { a, b })
}

fn parabola() -> Cubic {
    Cubic::new(
        vec![[4., 2.], [8. / 3., 3.], [4. / 3., 3.], [0., 2.]],
        vec![0., 1.],
        false,
    )
    .unwrap()
}

fn profile() -> Vec<Edge> {
    vec![
        line([0., 0.], [4., 0.]),
        line([4., 0.], [4., 2.]),
        Edge::Cubic(parabola()),
        line([0., 2.], [0., 0.]),
    ]
}

#[test]
fn non_gear_sweep_metrics_retain_cubic_area_centroid_and_support() {
    let sweep = Sweep::from_verified(profile(), None, 3., TOLERANCE).unwrap();
    assert_eq!(sweep.outline(), profile());
    assert_eq!(sweep.hole(), None);
    close(sweep.area_mm2(), 10., 1e-12);
    close(sweep.volume_mm3(), 30., 1e-12);
    let [x, y, z] = sweep.centroid_mm();
    close(x, 2., 1e-12);
    close(y, 1.26, 1e-12);
    close(z, 1.5, 1e-12);
    assert_eq!(sweep.bounds(), [[0., 0., 0.], [4., 2.75, 3.]]);
    assert_eq!(sweep.support([0.; 3]), 0.);
    assert!(sweep.support([f64::INFINITY, 0., 0.]).is_nan());

    let exact_length = (5. + 16. / 3. * 0.75_f64.asinh()) / 2.;
    let [lo, hi] = parabola().length_bounds_mm(1e-5).unwrap();
    assert!(lo <= exact_length && exact_length <= hi && hi - lo <= 1e-5);
    let [lo, hi] = sweep.surface_area_bounds_mm2(1e-4).unwrap();
    let exact_area = 20. + 3. * (8. + exact_length);
    assert!(lo <= exact_area && exact_area <= hi && hi - lo <= 1e-4);

    let mesh = sweep.mesh(OPTIONS).unwrap();
    let proof = certify(&mesh).unwrap();
    close(proof.signed_volume_mm3, 30., 0.05);
    assert_eq!(mesh.face_indices.iter().max(), Some(&5));
    for face in 0..=5 {
        assert!(mesh.face_indices.contains(&face));
    }

    let cubic = Edge::Cubic(parabola());
    let samples = cubic.sample(OPTIONS).unwrap();
    assert_eq!(samples.first(), Some(&cubic.point(0.)));
    assert_eq!(samples.last(), Some(&cubic.point(1.)));
    assert!(samples.len() > 10);
    assert_eq!(profile()[0].sample(OPTIONS).unwrap(), [[0., 0.], [4., 0.]]);
}

#[test]
fn source_knots_and_poles_survive_directed_evaluation() {
    let poles = vec![[0., 0.], [1., 1.], [2., 2.], [4., 2.], [5., 1.], [6., 0.]];
    let knots = vec![-1., 1., 4.];
    let forward = Cubic::new(poles.clone(), knots.clone(), false).unwrap();
    let backward = Cubic::new(poles.clone(), knots.clone(), true).unwrap();
    assert_eq!(forward.poles(), poles);
    assert_eq!(forward.knots(), knots);
    assert!(!forward.reversed());
    assert!(backward.reversed());
    let spans: Vec<_> = forward.bezier_spans().collect();
    assert_eq!(spans[0][3], [2.8, 2.]);
    assert_eq!(spans[1][0], spans[0][3]);
    assert_eq!(forward.point(0.4), spans[1][0]);
    for i in 0..=32 {
        let t = f64::from(i) / 32.;
        assert_eq!(forward.point(t), backward.point(1. - t));
    }
    assert_eq!(backward.bezier_spans().last().unwrap()[3], [0., 0.]);
    assert!(forward.support([0., 1.]) >= 2.);
}

fn gear() -> spur_gear::Profile {
    let angle = 20_f64.to_radians();
    let teeth = 20;
    let pitch = f64::from(teeth);
    spur_gear::Profile::new(
        spur_gear::Spec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * angle.cos(),
            tip_radius_mm: pitch + 2.,
            root_radius_mm: pitch - 2.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: std::f64::consts::PI / (2. * pitch) + angle.tan()
                - angle
                - 0.05 / (2. * pitch),
            face_width_mm: 8.,
            bore_radius_mm: 2.,
        },
        1e-5,
        1e7,
    )
    .unwrap()
}

#[test]
fn oblique_step_rebuilds_source_poles_and_bore_without_a_gear_recipe() {
    let profile = gear();
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let (origin, u, v, n) = ([11., -7., 23.], [s, 0., s], [0., 1., 0.], [-s, 0., s]);
    let bytes = step::write_spur_gear(&profile, origin, u, v, TOLERANCE).unwrap();
    let (readback, decoded) = read_import_solid(&bytes, TOLERANCE).unwrap();
    let ImportProfile::SplineSweep { sweep } = decoded.profile else {
        panic!("expected source spline sweep");
    };
    assert_eq!(
        readback,
        crate::step_read::read_analytic_solid(&bytes, TOLERANCE).unwrap()
    );
    assert_eq!(decoded.origin_mm, origin);
    assert_eq!(decoded.u, u);
    assert_eq!(decoded.v, v);
    assert_eq!(sweep.outline().len(), profile.outline().len());
    assert_eq!(
        sweep.hole(),
        Some(normal_sweep::Edge::Circle {
            center: [0.; 2],
            radius_mm: 2.,
            clockwise: true,
        }),
    );
    close(sweep.volume_mm3(), readback.volume_mm3, 1e-9);
    close(sweep.volume_mm3(), profile.volume_mm3(), 1e-4);
    let [lo, hi] = sweep.surface_area_bounds_mm2(0.005).unwrap();
    assert!(hi - lo <= 0.005);
    assert!((lo..=hi).contains(&profile.surface_area_mm2()));
    let expected_bounds = readback.bounds_mm;
    for i in 0..3 {
        let direction = [u[i], v[i], n[i]];
        close(
            origin[i] + sweep.support(direction),
            expected_bounds[1][i],
            1e-7,
        );
        close(
            origin[i] - sweep.support(direction.map(|v| -v)),
            expected_bounds[0][i],
            1e-7,
        );
    }

    let mut cubics = 0;
    for edge in sweep.outline() {
        if let Edge::Cubic(cubic) = edge {
            cubics += 1;
            assert!(readback.splines.iter().any(|source| {
                source.knots() == cubic.knots()
                    && source.poles_mm().len() == cubic.poles().len()
                    && source
                        .poles_mm()
                        .iter()
                        .zip(cubic.poles())
                        .all(|(&world, &local)| {
                            let relative = std::array::from_fn::<_, 3, _>(|i| world[i] - origin[i]);
                            let project = |axis: [f64; 3]| {
                                axis.into_iter()
                                    .zip(relative)
                                    .map(|(a, b)| a * b)
                                    .sum::<f64>()
                            };
                            length(sub([project(u), project(v)], local)) <= 1e-10
                                && project(n).abs() <= 1e-10
                        })
            }));
        } else {
            let points = edge.sample(OPTIONS).unwrap();
            assert_eq!(points.first(), Some(&edge.point(0.)));
            assert_eq!(points.last(), Some(&edge.point(1.)));
        }
    }
    assert_eq!(cubics, 40);
    let proof = certify(&sweep.mesh(OPTIONS).unwrap()).unwrap();
    close(proof.signed_volume_mm3, readback.volume_mm3, 1.);
    assert!(dot([u[0], u[1]], [v[0], v[1]]).abs() < 1e-14);
    assert_eq!(read_import_solid("", TOLERANCE), Err(ReadError::Invalid));
}

#[test]
fn malformed_source_and_tessellation_fail_with_bounded_work() {
    assert_eq!(Cubic::new(vec![], vec![0.; 129], false), Err(Error::Limit));
    for (poles, knots) in [
        (vec![], vec![0., 1.]),
        (vec![[0.; 2]; 4], vec![0., 1.]),
        (vec![[0.; 2], [1., 1.], [2., 1.], [3., 0.]], vec![1., 0.]),
        (
            vec![[0.; 2], [1., 1.], [2., f64::NAN], [3., 0.]],
            vec![0., 1.],
        ),
    ] {
        assert_eq!(Cubic::new(poles, knots, false), Err(Error::Invalid));
    }
    assert_eq!(parabola().length_bounds_mm(0.), Err(Error::Invalid));
    let cubic = Edge::Cubic(parabola());
    assert_eq!(cubic.length_bounds_mm(f64::NAN), Err(Error::Invalid));
    assert!(cubic.length_bounds_mm(0.01).is_ok());
    assert_eq!(profile()[0].length_bounds_mm(0.01), Ok([4.; 2]));
    assert_eq!(
        Sweep::from_verified(profile(), None, 0., TOLERANCE),
        Err(Error::Invalid)
    );
    let clockwise = vec![
        line([0., 0.], [0., 1.]),
        line([0., 1.], [1., 0.]),
        line([1., 0.], [0., 0.]),
    ];
    assert_eq!(
        Sweep::from_verified(clockwise, None, 3., TOLERANCE),
        Err(Error::Invalid)
    );
    let mut disconnected = profile();
    disconnected[3] = line([0., 2.01], [0., 0.]);
    assert_eq!(
        Sweep::from_verified(disconnected, None, 3., TOLERANCE),
        Err(Error::Invalid)
    );
    let mut roundoff = profile();
    roundoff[3] = line([0., 2. + 5e-11], [0., 0.]);
    let sweep = Sweep::from_verified(roundoff, None, 3., TOLERANCE).unwrap();
    assert_eq!(
        sweep.mesh(Tessellation {
            deflection: 1e-11,
            ..OPTIONS
        }),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(sweep.surface_area_bounds_mm2(0.), Err(Error::Invalid));
    assert_eq!(
        sweep.surface_area_bounds_mm2(f64::from_bits(1)),
        Err(Error::Invalid)
    );
    let round = Edge::Analytic(normal_sweep::Edge::Arc {
        center: [0., 0.],
        a: [1., 0.],
        b: [0., 1.],
        clockwise: false,
    });
    assert_eq!(
        round.sample_with_limit(OPTIONS, 1),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        cubic.sample(Tessellation {
            triangle_limit: 1,
            ..OPTIONS
        }),
        Err(MeshError::BudgetExceeded)
    );

    let plain = Sweep::from_verified(profile(), None, 3., TOLERANCE).unwrap();
    let hole = normal_sweep::Edge::Circle {
        center: [2., 1.],
        radius_mm: 0.2,
        clockwise: true,
    };
    let bored = Sweep::from_verified(profile(), Some(hole), 3., TOLERANCE).unwrap();
    let limit = plain.mesh(OPTIONS).unwrap().face_indices.len();
    assert_eq!(
        bored.mesh(Tessellation {
            triangle_limit: limit,
            ..OPTIONS
        }),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn standard_import_reads_mixed_members_and_refuses_unmodeled_hubs() {
    let gear = gear();
    let mut writer = step::Writer::new();
    let u = [1., 0., 0.];
    let v = [0., 1., 0.];
    writer
        .append_box([crate::Rational::new(2, 1).unwrap(); 3], [0.; 3], u, v)
        .unwrap();
    writer
        .append_spur_gear(&gear, [50., 0., 0.], u, v, TOLERANCE)
        .unwrap();
    let bytes = writer.finish().unwrap();
    let members = read_import_compound(&bytes, TOLERANCE).unwrap();
    assert_eq!(members.len(), 2);
    assert!(matches!(
        members[0].1.profile,
        ImportProfile::Polygon { .. }
    ));
    assert!(matches!(
        members[1].1.profile,
        ImportProfile::SplineSweep { .. }
    ));
    close(members[0].0.volume_mm3, 8., 1e-12);
    close(members[1].0.volume_mm3, gear.volume_mm3(), 1e-4);

    let hub = gear
        .with_hub(
            spur_gear::Hub {
                radius_mm: 10.,
                axial_start_mm: -3.,
                axial_end_mm: 11.,
            },
            1e-5,
            1e7,
        )
        .unwrap();
    let bytes = step::write_spur_gear(&hub, [0.; 3], u, v, TOLERANCE).unwrap();
    assert!(crate::step_read::read_analytic_solid(&bytes, TOLERANCE).is_ok());
    assert_eq!(
        read_import_solid(&bytes, TOLERANCE),
        Err(ReadError::Unsupported)
    );
}

fn authored_profile() -> Vec<Edge> {
    let source = parabola().bezier_spans().next().unwrap();
    let midpoint = |a: Point, b: Point| std::array::from_fn(|i| a[i].midpoint(b[i]));
    let [a, b, c, d] = source;
    let [ab, bc, cd] = [midpoint(a, b), midpoint(b, c), midpoint(c, d)];
    let [x, y] = [midpoint(ab, bc), midpoint(bc, cd)];
    let m = midpoint(x, y);
    vec![
        line([0., 0.], [4., 0.]),
        line([4., 0.], [4., 2.]),
        Edge::Cubic(Cubic::new(vec![a, ab, x, m], vec![0., 0.5], false).unwrap()),
        Edge::Cubic(Cubic::new(vec![m, y, cd, d], vec![0.5, 1.], false).unwrap()),
        line([0., 2.], [0., 0.]),
    ]
}
#[test]
fn authored_section_proves_material_before_mesh_and_real_step_roundtrip() {
    let holes = [
        None,
        Some(normal_sweep::Edge::Circle {
            center: [2., 1.],
            radius_mm: 0.25,
            clockwise: true,
        }),
    ];
    for hole in holes {
        let shape = Sweep::new(authored_profile(), hole, 3., TOLERANCE).unwrap();
        close(
            shape.volume_mm3(),
            30. - hole.map_or(0., |_| 3. * std::f64::consts::PI * 0.25_f64.powi(2)),
            1e-10,
        );
        let mesh = shape.mesh(OPTIONS).unwrap();
        let receipt = certify(&mesh).unwrap();
        close(receipt.signed_volume_mm3, shape.volume_mm3(), 0.02);
        let bytes =
            step::write_spline_sweep(&shape, [0., 0., 0.], [1., 0., 0.], [0., 1., 0.], TOLERANCE)
                .unwrap();
        let (_, decoded) = read_import_solid(&bytes, TOLERANCE).unwrap();
        let ImportProfile::SplineSweep { sweep } = decoded.profile else {
            panic!("lost analytic source");
        };
        close(sweep.volume_mm3(), shape.volume_mm3(), 1e-9);
        assert_eq!(shape.outline(), sweep.outline());
    }
}
#[test]
fn authored_section_refuses_invalid_uncertain_and_cancelled_work_atomically() {
    use std::cell::Cell;
    let source = authored_profile();
    let snapshot = source.clone();
    for tolerance in [0., f64::NAN] {
        assert_eq!(
            Sweep::new(source.clone(), None, 3., tolerance),
            Err(ConstructionError::Invalid)
        );
    }
    assert_eq!(
        Sweep::new(source.clone(), None, -3., TOLERANCE),
        Err(ConstructionError::Invalid)
    );
    assert_eq!(
        Sweep::new(source.clone(), None, 1e9, TOLERANCE),
        Err(ConstructionError::Limit)
    );
    assert_eq!(
        Sweep::new(
            vec![source[0].clone(); crate::polygon::MAX_VERTICES + 1],
            None,
            3.,
            TOLERANCE
        ),
        Err(ConstructionError::Limit)
    );
    assert_eq!(
        Sweep::new(profile(), None, 3., TOLERANCE),
        Err(ConstructionError::Unresolved)
    );
    assert_eq!(
        Sweep::new_with_interrupt(source.clone(), None, 3., TOLERANCE, Some(&|| true)),
        Err(ConstructionError::Cancelled)
    );
    let count = Cell::new(0);
    let interrupt = || {
        count.set(count.get() + 1);
        count.get() > source.len() + 3
    };
    assert_eq!(
        Sweep::new_with_interrupt(source.clone(), None, 3., TOLERANCE, Some(&interrupt)),
        Err(ConstructionError::Cancelled)
    );
    assert_eq!(source, snapshot);
    let early = Cell::new(0);
    let stop_during_span_count = || {
        early.set(early.get() + 1);
        early.get() == 3
    };
    assert_eq!(
        Sweep::new_with_interrupt(
            source.clone(),
            None,
            3.,
            TOLERANCE,
            Some(&stop_during_span_count)
        ),
        Err(ConstructionError::Cancelled)
    );
    let knots: Vec<_> = (0..128).map(f64::from).collect();
    let poles: Vec<_> = (0..256).map(|i| [f64::from(i), 0.]).collect();
    let many_spans = Edge::Cubic(Cubic::new(poles, knots, false).unwrap());
    assert_eq!(
        Sweep::new(vec![many_spans; 9], None, 3., TOLERANCE),
        Err(ConstructionError::Limit)
    );
    assert!(Sweep::new(source, None, 3., TOLERANCE).is_ok());
}

#[test]
fn authored_multiple_counter_loops_keep_material_winding_and_closed_mesh() {
    let square = vec![
        line([0.5, 0.5], [0.5, 1.]),
        line([0.5, 1.], [1., 1.]),
        line([1., 1.], [1., 0.5]),
        line([1., 0.5], [0.5, 0.5]),
    ];
    let circle = vec![Edge::Analytic(normal_sweep::Edge::Circle {
        center: [3., 1.],
        radius_mm: 0.2,
        clockwise: true,
    })];
    let holes = vec![square, circle];
    let sweep = Sweep::new_rings(
        vec![
            line([0., 0.], [4., 0.]),
            line([4., 0.], [4., 3.]),
            line([4., 3.], [0., 3.]),
            line([0., 3.], [0., 0.]),
        ],
        holes.clone(),
        3.,
        TOLERANCE,
    )
    .unwrap();
    let area = 12. - 0.25 - std::f64::consts::PI * 0.04;
    close(sweep.area_mm2(), area, 1e-12);
    close(sweep.volume_mm3(), area * 3., 1e-12);
    assert_eq!(sweep.holes(), holes);
    assert_eq!(sweep.edges().count(), 9);
    let mesh = sweep.mesh(OPTIONS).unwrap();
    let proof = certify(&mesh).unwrap();
    close(proof.signed_volume_mm3, area * 3., 0.05);
    assert_eq!(mesh.face_indices.iter().max(), Some(&10));
}

fn subdivided_profile() -> Vec<Edge> {
    fn split([a, b, c, d]: [Point; 4]) -> [[Point; 4]; 2] {
        let midpoint = |a: Point, b: Point| std::array::from_fn(|i| (a[i] + b[i]) / 2.);
        let [ab, bc, cd] = [midpoint(a, b), midpoint(b, c), midpoint(c, d)];
        let [x, y] = [midpoint(ab, bc), midpoint(bc, cd)];
        let m = midpoint(x, y);
        [[a, ab, x, m], [m, y, cd, d]]
    }
    let poles: [Point; 4] = parabola().poles().try_into().unwrap();
    let mut result = vec![line([0., 0.], [4., 0.]), line([4., 0.], [4., 2.])];
    result.extend(
        split(poles)
            .into_iter()
            .flat_map(split)
            .map(|p| Edge::Cubic(Cubic::new(p.to_vec(), vec![0., 1.], false).unwrap())),
    );
    result.push(line([0., 2.], [0., 0.]));
    result
}
fn counter_square(x: f64, y: f64, size: f64) -> Vec<Edge> {
    let points = [[x, y], [x, y + size], [x + size, y + size], [x + size, y]];
    (0..4)
        .map(|i| line(points[i], points[(i + 1) % 4]))
        .collect()
}

#[test]
fn multi_counter_source_curves_survive_placed_step_reconstruction() {
    let circle = vec![Edge::Analytic(normal_sweep::Edge::Circle {
        center: [3., 1.],
        radius_mm: 0.2,
        clockwise: true,
    })];
    for (holes, area) in [
        (
            vec![counter_square(0.5, 0.5, 0.5), counter_square(2.5, 0.5, 0.5)],
            9.5,
        ),
        (
            vec![counter_square(0.5, 0.5, 0.5), circle],
            9.75 - std::f64::consts::PI * 0.04,
        ),
        (vec![counter_square(0.5, 0.5, 0.5)], 9.75),
    ] {
        let count = holes.len();
        let sweep = Sweep::new_rings(subdivided_profile(), holes, 3., TOLERANCE).unwrap();
        close(sweep.area_mm2(), area, 1e-12);
        for [origin, u, v] in [
            [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
            [[80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
        ] {
            let source = step::write_spline_sweep(&sweep, origin, u, v, TOLERANCE).unwrap();
            let (readback, import) = read_import_solid(&source, TOLERANCE).unwrap();
            close(readback.volume_mm3, area * 3., 1e-8);
            let ImportProfile::SplineSweep { sweep: loaded } = import.profile else {
                panic!()
            };
            assert_eq!(loaded.holes().len(), count);
            close(loaded.area_mm2(), area, 1e-10);
            let again =
                step::write_spline_sweep(&loaded, import.origin_mm, import.u, import.v, TOLERANCE)
                    .unwrap();
            close(
                read_import_solid(&again, TOLERANCE).unwrap().0.volume_mm3,
                area * 3.,
                1e-8,
            );
        }
    }
}

#[test]
fn multiple_counter_proof_rejects_invalid_material_and_is_cooperatively_bounded() {
    let valid = vec![counter_square(0.5, 0.5, 0.5), counter_square(2.5, 0.5, 0.5)];
    for holes in [
        vec![Vec::new()],
        vec![counter_square(8., 8., 0.5)],
        vec![counter_square(0., 0., 10.)],
        vec![
            counter_square(0.5, 0.5, 1.),
            counter_square(0.75, 0.75, 0.25),
        ],
        {
            let mut ring = counter_square(0.5, 0.5, 0.5);
            ring.reverse();
            ring = ring
                .into_iter()
                .map(|edge| match edge {
                    Edge::Analytic(edge) => Edge::Analytic(edge.reversed()),
                    _ => unreachable!(),
                })
                .collect();
            vec![ring]
        },
    ] {
        assert!(matches!(
            Sweep::new_rings(subdivided_profile(), holes, 3., TOLERANCE),
            Err(ConstructionError::Invalid)
        ));
    }
    for holes in [
        vec![counter_square(0.5, 0.5, 0.5); 2],
        vec![counter_square(0.5, 0.5, 0.5), counter_square(1., 0.5, 0.5)],
    ] {
        assert_eq!(
            Sweep::new_rings(subdivided_profile(), holes, 3., TOLERANCE),
            Err(ConstructionError::Unresolved)
        );
    }
    let single =
        Sweep::new_rings(subdivided_profile(), vec![valid[0].clone()], 3., TOLERANCE).unwrap();
    assert_eq!(single.hole(), None);
    assert_eq!(single.holes().len(), 1);
    let count = std::cell::Cell::new(0usize);
    let observe = || {
        count.set(count.get() + 1);
        false
    };
    Sweep::new_rings_with_interrupt(
        subdivided_profile(),
        valid.clone(),
        3.,
        TOLERANCE,
        Some(&observe),
    )
    .unwrap();
    for stop in 0..count.get() {
        let calls = std::cell::Cell::new(0usize);
        let cancel = || {
            let old = calls.get();
            calls.set(old + 1);
            old == stop
        };
        assert_eq!(
            Sweep::new_rings_with_interrupt(
                subdivided_profile(),
                valid.clone(),
                3.,
                TOLERANCE,
                Some(&cancel)
            ),
            Err(ConstructionError::Cancelled)
        );
    }
    Sweep::new_rings(subdivided_profile(), valid, 3., TOLERANCE).unwrap();
    let outer = vec![
        line([0., 0.], [100., 0.]),
        line([100., 0.], [100., 100.]),
        line([100., 100.], [0., 100.]),
        line([0., 100.], [0., 0.]),
    ];
    let many = (0..512)
        .map(|i| {
            vec![Edge::Analytic(normal_sweep::Edge::Circle {
                center: [1. + 2. * (i % 32) as f64, 1. + 2. * (i / 32) as f64],
                radius_mm: 0.1,
                clockwise: true,
            })]
        })
        .collect();
    assert_eq!(
        Sweep::new_rings(outer, many, 3., TOLERANCE),
        Err(ConstructionError::Limit)
    );
}
