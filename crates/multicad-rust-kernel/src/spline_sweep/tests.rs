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
