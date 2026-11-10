use super::*;

const TOLERANCE: f64 = 1e-7;

fn spec(start_height_mm: f64) -> Spec {
    Spec {
        stock_radius_mm: 8.,
        stock_height_mm: 30.,
        tube_radius_mm: 0.6,
        pitch_mm: 2.,
        path_height_mm: 15.,
        start_height_mm,
        start_angle_rad: 0.,
    }
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}

fn point(cut: Cut, theta: f64, x: f64, y: f64) -> [f64; 3] {
    let (s, c) = (theta + cut.spec.start_angle_rad).sin_cos();
    let radial = cut.spec.stock_radius_mm + x;
    [
        radial * c - cut.tangent * y * s,
        radial * s + cut.tangent * y * c,
        cut.spec.start_height_mm + cut.rise * theta - cut.vertical * y,
    ]
}

#[test]
fn occt_1543_cuts_material_and_keeps_both_lifted_branches_through_the_stock_cap() {
    // Analytic reference volumes, independently integrated over the circular
    // normal disk clipped by the stock radius. See OCCT issue #1543.
    for (start, expected) in [
        (16.0, 5839.062685224),
        (16.2, 5841.816902505),
        (16.6, 5847.325337067),
    ] {
        for phase in [-PI, -0.1, 0., 0.1, PI / 2., PI] {
            let cut = Cut::new(
                Spec {
                    start_angle_rad: phase,
                    ..spec(start)
                },
                TOLERANCE,
            )
            .unwrap();
            near(cut.volume_mm3(), expected);
            let stock = PI * 8_f64.powi(2) * 30.;
            assert!(cut.removed_volume_mm3() > 180. && cut.volume_mm3() < stock);
            let [a, b] = cut.cylinder_trims();
            assert!(a.height_mm[0] > b.height_mm[0]);
            let [x, y] = cut.rim;
            for (y, trim) in [(-y, a), (y, b)] {
                near(x.hypot(y), cut.spec.tube_radius_mm);
                for (azimuth, height) in trim.azimuth_rad.into_iter().zip(trim.height_mm) {
                    let (s, c) = azimuth.sin_cos();
                    let p = [8. * c, 8. * s, height];
                    assert_eq!(cut.classify(p), Ok(Location::Boundary));
                }
                near(trim.height_mm[1], 30.);
                assert!(trim.azimuth_rad[1] - trim.azimuth_rad[0] > 6. * TAU);
            }

            let top = (30. - start) / cut.rise;
            for theta in [0.05, TAU - 0.01, TAU + 0.01, top - 0.01] {
                assert_eq!(
                    cut.classify(point(cut, theta, -0.3, 0.)),
                    Ok(Location::Exterior)
                );
                assert_eq!(
                    cut.classify(point(cut, theta, -0.8, 0.)),
                    Ok(Location::Interior)
                );
                assert_eq!(
                    cut.classify(point(cut, theta, -0.6, 0.)),
                    Ok(Location::Boundary)
                );
            }
            assert_eq!(
                cut.classify(point(cut, top, -0.3, 0.)),
                Ok(Location::Exterior)
            );
            assert_eq!(
                cut.classify(point(cut, top + 0.01, -0.8, 0.)),
                Ok(Location::Exterior)
            );
            assert_eq!(
                cut.classify(point(cut, -0.05, -0.3, 0.)),
                Ok(Location::Interior)
            );
            assert_eq!(
                cut.classify(point(cut, 0., -0.3, 0.)),
                Ok(Location::Boundary)
            );
            let [x, y, z] = point(cut, 2., -0.3, 0.);
            assert_eq!(cut.classify([x, y, z + 0.99]), Ok(Location::Interior));
            assert_eq!(cut.classify([0., 0., 30.]), Ok(Location::Boundary));
        }
    }
}

#[test]
fn invalid_or_unproven_charts_refuse_before_producing_material() {
    let fixture = spec(16.2);
    for spec in [
        Spec {
            stock_radius_mm: f64::NAN,
            ..fixture
        },
        Spec {
            pitch_mm: -2.,
            ..fixture
        },
        Spec {
            start_angle_rad: TAU * 2.,
            ..fixture
        },
    ] {
        assert_eq!(Cut::new(spec, TOLERANCE).unwrap_err(), Error::InvalidInput);
    }
    for spec in [
        Spec {
            stock_radius_mm: 2e7,
            ..fixture
        },
        Spec {
            path_height_mm: 1024.,
            ..fixture
        },
    ] {
        assert_eq!(Cut::new(spec, TOLERANCE).unwrap_err(), Error::Limit);
    }
    for spec in [
        Spec {
            pitch_mm: 1.,
            ..fixture
        },
        Spec {
            pitch_mm: 60.,
            ..fixture
        },
        Spec {
            path_height_mm: 14.,
            ..fixture
        },
        Spec {
            start_height_mm: 0.5,
            ..fixture
        },
        Spec {
            start_height_mm: 29.5,
            ..fixture
        },
        Spec {
            tube_radius_mm: 4.01,
            ..fixture
        },
        Spec {
            tube_radius_mm: 1e-8,
            ..fixture
        },
    ] {
        assert_eq!(
            Cut::new(spec, TOLERANCE).unwrap_err(),
            Error::UnsupportedGeometry
        );
    }
    assert_eq!(
        Cut::new(
            Spec {
                stock_radius_mm: 1e7,
                ..fixture
            },
            1e-10
        )
        .unwrap_err(),
        Error::UnsupportedGeometry
    );
    assert_eq!(Cut::new(fixture, 0.).unwrap_err(), Error::InvalidInput);
    let cut = Cut::new(fixture, TOLERANCE).unwrap();
    assert_eq!(
        cut.classify([f64::INFINITY, 0., 0.]),
        Err(Error::InvalidInput)
    );
}

#[test]
fn uniform_scale_preserves_material_and_cubic_volume() {
    let cut = Cut::new(spec(16.2), TOLERANCE).unwrap();
    let scaled = Cut::new(
        Spec {
            stock_radius_mm: 16.,
            stock_height_mm: 60.,
            tube_radius_mm: 1.2,
            pitch_mm: 4.,
            path_height_mm: 30.,
            start_height_mm: 32.4,
            start_angle_rad: 0.,
        },
        2. * TOLERANCE,
    )
    .unwrap();
    near(scaled.volume_mm3(), 8. * cut.volume_mm3());
    let p = point(cut, 10., -0.3, 0.2);
    assert_eq!(scaled.classify(p.map(|x| 2. * x)), cut.classify(p));
}
