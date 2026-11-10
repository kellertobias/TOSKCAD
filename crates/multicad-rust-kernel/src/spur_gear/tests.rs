use super::*;

const MINIMUM: f64 = 1e-5;
const TOLERANCE: f64 = 1e-7;

fn spec(teeth: u32, pressure_deg: f64) -> Spec {
    let pressure = pressure_deg.to_radians();
    let pitch = f64::from(teeth);
    Spec {
        teeth,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * pressure.cos(),
        tip_radius_mm: pitch + 2.,
        root_radius_mm: pitch - 2.5,
        root_fillet_radius_mm: 0.4,
        half_base_angle_rad: PI / (2. * pitch) + pressure.tan() - pressure - 0.05 / (2. * pitch),
        face_width_mm: 8.,
        bore_radius_mm: 2.,
    }
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}

fn point(edge: Edge, t: f64) -> Point {
    match edge {
        Edge::Planar(edge) => edge.point(t),
        Edge::Involute(edge) => edge.point(t),
    }
}

fn norm([x, y]: Point) -> f64 {
    x.hypot(y)
}

fn arc_tangent(edge: Edge, at_start: bool) -> Point {
    let Edge::Planar(normal_sweep::Edge::Arc {
        center,
        a,
        b,
        clockwise,
    }) = edge
    else {
        panic!("Expected circle");
    };
    let [x, y] = sub(if at_start { a } else { b }, center);
    scale([-y, x], if clockwise { -1. } else { 1. } / x.hypot(y))
}

#[test]
fn exact_profiles_cover_both_native_fillet_constructions() {
    for (spec, radial) in [
        (spec(20, 20.), true),
        (spec(64, 20.), false),
        (spec(128, 15.), false),
    ] {
        let profile = Profile::new(spec, MINIMUM, MAX_COORDINATE_MM).unwrap();
        let edges = profile.outline();
        let edges_per_tooth = if radial { 8 } else { 6 };
        assert_eq!(edges.len(), spec.teeth as usize * edges_per_tooth);
        assert_eq!(profile.height_mm(), spec.face_width_mm);
        assert_eq!(
            profile.bore(),
            Some(normal_sweep::Edge::Circle {
                center: [0.; 2],
                radius_mm: spec.bore_radius_mm,
                clockwise: true,
            })
        );
        for (tooth, sector) in edges.chunks_exact(edges_per_tooth).enumerate() {
            let mut roles = vec![
                ToothRole::LeftFillet,
                ToothRole::LeftFlank,
                ToothRole::Tip,
                ToothRole::RightFlank,
                ToothRole::RightFillet,
                ToothRole::Root,
            ];
            if radial {
                roles.insert(1, ToothRole::LeftRadial);
                roles.insert(5, ToothRole::RightRadial);
            }
            assert_eq!(sector.iter().map(|e| e.role).collect::<Vec<_>>(), roles);
            assert!(sector.iter().all(|e| e.tooth as usize == tooth));
            for (i, edge) in sector.iter().enumerate() {
                let next = edges[(tooth * edges_per_tooth + i + 1) % edges.len()];
                close(
                    norm(sub(point(edge.edge, 1.), point(next.edge, 0.))),
                    0.,
                    1e-12,
                );
            }
            let flank_index = if radial { 2 } else { 1 };
            let Edge::Involute(flank) = sector[flank_index].edge else {
                panic!("Expected analytic involute");
            };
            let Edge::Planar(fillet) = sector[0].edge else {
                panic!("Expected circular root fillet");
            };
            close(
                fillet.radius_mm().unwrap(),
                spec.root_fillet_radius_mm,
                1e-12,
            );
            close(norm(fillet.start()), spec.root_radius_mm, 1e-12);
            close(norm(flank.point(1.)), spec.tip_radius_mm, 1e-12);
            let [a, b] = flank.interval();
            let t = (spec.base_radius_mm / spec.pitch_radius_mm).acos().tan();
            let phase = TAU * tooth as f64 / f64::from(spec.teeth);
            let pitch_point = rotate(flank.point((t - a) / (b - a)), -phase);
            close(norm(pitch_point), spec.pitch_radius_mm, 1e-12);
            close(
                pitch_point[1].atan2(pitch_point[0]),
                -spec.half_base_angle_rad + t - t.atan(),
                1e-12,
            );
            close(
                norm(sub(arc_tangent(sector[0].edge, false), flank.tangent(0.))),
                0.,
                1e-12,
            );
            let root = sector[edges_per_tooth - 1].edge;
            close(
                norm(sub(
                    arc_tangent(root, false),
                    arc_tangent(
                        edges[(tooth * edges_per_tooth + edges_per_tooth) % edges.len()].edge,
                        true,
                    ),
                )),
                0.,
                1e-12,
            );
        }

        let mut sampled_area = Sum::default();
        for entry in edges {
            let count = 1024;
            for i in 0..count {
                let a = point(entry.edge, f64::from(i) / f64::from(count));
                let b = point(entry.edge, f64::from(i + 1) / f64::from(count));
                sampled_area.add(cross(a, b) / 2.);
            }
        }
        let expected_area = sampled_area.value() - PI * spec.bore_radius_mm.powi(2);
        close(profile.area_mm2(), expected_area, expected_area * 1e-7);
        close(
            profile.volume_mm3(),
            profile.area_mm2() * spec.face_width_mm,
            1e-12,
        );
    }
}

fn spline_point(spline: &CubicSpline, fraction: f64) -> Point {
    let knots: Vec<_> = spline
        .knots
        .iter()
        .copied()
        .zip(&spline.multiplicities)
        .flat_map(|(knot, &n)| std::iter::repeat_n(knot, n as usize))
        .collect();
    let last = spline.poles_mm.len() - 1;
    let k = if fraction == 1. {
        last
    } else {
        (3..=last)
            .find(|&k| knots[k] <= fraction && fraction < knots[k + 1])
            .unwrap()
    };
    let mut work: [Point; 4] = spline.poles_mm[k - 3..=k].try_into().unwrap();
    for level in 1..=3 {
        for i in (level..=3).rev() {
            let j = k - 3 + i;
            let f = (fraction - knots[j]) / (knots[j + 4 - level] - knots[j]);
            work[i] = add(scale(work[i - 1], 1. - f), scale(work[i], f));
        }
    }
    work[3]
}

#[test]
fn evaluated_composite_spline_stays_within_certified_error() {
    let flank = Involute::new(36., [0., 0.72], -0.12).unwrap();
    for curve in [
        flank,
        flank.reversed(),
        flank.reflected().rotated(1.5).unwrap(),
    ] {
        let spline = curve.cubic_spline(TOLERANCE).unwrap();
        let count = spline.knots.len() - 1;
        assert!((2..=127).contains(&count));
        assert_eq!(spline.poles_mm.len(), 2 * count + 2);
        assert_eq!(spline.multiplicities[0], 4);
        assert_eq!(spline.multiplicities[count], 4);
        assert!(spline.multiplicities[1..count].iter().all(|&n| n == 2));
        assert!(spline.error_bound_mm <= TOLERANCE);
        for i in 0..=1000 {
            let t = f64::from(i) / 1000.;
            assert!(norm(sub(curve.point(t), spline_point(&spline, t))) <= spline.error_bound_mm);
        }
    }
    let short = Involute::new(1., [0.2, 0.20001], 0.).unwrap();
    assert_eq!(short.cubic_spline(TOLERANCE).unwrap().poles_mm.len(), 4);
    close(flank.length_mm(), 36. * 0.72_f64.powi(2) / 2., 1e-12);
    close(
        flank.signed_area_mm2(),
        36_f64.powi(2) * 0.72_f64.powi(3) / 6.,
        1e-12,
    );
    close(
        flank.reflected().signed_area_mm2(),
        -flank.signed_area_mm2(),
        1e-12,
    );
    close(
        flank.reversed().signed_area_mm2(),
        -flank.signed_area_mm2(),
        1e-12,
    );
}

#[test]
fn rejects_invalid_or_unresolvable_geometry() {
    let nominal = spec(20, 20.);
    assert_eq!(
        Profile::new(
            Spec {
                teeth: 129,
                ..nominal
            },
            MINIMUM,
            MAX_COORDINATE_MM
        ),
        Err(Error::Limit)
    );
    for invalid in [
        Spec {
            base_radius_mm: f64::NAN,
            ..nominal
        },
        Spec {
            base_radius_mm: 21.,
            ..nominal
        },
        Spec {
            pitch_radius_mm: 15.,
            ..nominal
        },
        Spec {
            tip_radius_mm: 19.,
            ..nominal
        },
        Spec {
            root_fillet_radius_mm: 5.,
            ..nominal
        },
        Spec {
            half_base_angle_rad: 0.,
            ..nominal
        },
        Spec {
            bore_radius_mm: nominal.root_radius_mm,
            ..nominal
        },
        Spec {
            bore_radius_mm: MINIMUM / 2.,
            ..nominal
        },
    ] {
        assert_eq!(
            Profile::new(invalid, MINIMUM, MAX_COORDINATE_MM),
            Err(Error::InvalidDimensions)
        );
    }
    assert_eq!(
        Profile::new(nominal, 0., MAX_COORDINATE_MM),
        Err(Error::InvalidDimensions)
    );
    assert_eq!(
        Profile::new(nominal, MINIMUM, MINIMUM),
        Err(Error::InvalidDimensions)
    );
    for invalid in [
        Spec {
            half_base_angle_rad: 0.01,
            ..nominal
        },
        Spec {
            half_base_angle_rad: 0.155,
            ..nominal
        },
        Spec {
            base_radius_mm: (nominal.root_radius_mm
                * (nominal.root_radius_mm + 2. * nominal.root_fillet_radius_mm))
                .sqrt()
                + MINIMUM / 2.,
            ..nominal
        },
    ] {
        assert_eq!(
            Profile::new(invalid, MINIMUM, MAX_COORDINATE_MM),
            Err(Error::InvalidProfile)
        );
    }
    let unbored = Profile::new(
        Spec {
            bore_radius_mm: 0.,
            ..nominal
        },
        MINIMUM,
        MAX_COORDINATE_MM,
    )
    .unwrap();
    assert_eq!(unbored.bore(), None);

    assert_eq!(
        Involute::new(0., [0., 1.], 0.),
        Err(Error::InvalidDimensions)
    );
    assert_eq!(
        Involute::new(1., [0.5, 0.2], 0.),
        Err(Error::InvalidDimensions)
    );
    assert_eq!(
        Involute::new(1., [0., 1.], f64::NAN),
        Err(Error::InvalidDimensions)
    );
    let curve = Involute::new(40., [0., 1.], 0.).unwrap();
    assert_eq!(curve.rotated(f64::INFINITY), Err(Error::InvalidDimensions));
    assert_eq!(curve.cubic_spline(f64::NAN), Err(Error::InvalidDimensions));
    assert_eq!(curve.cubic_spline(0.), Err(Error::InvalidDimensions));
    assert_eq!(curve.cubic_spline(1e-13), Err(Error::Limit));
}
