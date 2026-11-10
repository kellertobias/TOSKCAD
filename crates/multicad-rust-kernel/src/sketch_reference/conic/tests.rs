use super::*;

const TOL: f64 = 1e-7;

fn xy() -> Conic {
    Conic {
        center: [0.; 3],
        axes: [[1., 0., 0.], [0., 1., 0.]],
        semiaxes_mm: [5.; 2],
        start_rad: 0.,
        sweep_rad: TAU,
    }
}

fn edge_on(depth: f64) -> Conic {
    Conic {
        center: [7., 0., depth],
        axes: [[0., 1., 0.], [0., 0., 1.]],
        ..xy()
    }
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}

fn contacts(conic: Conic) -> Vec<([f64; 2], Contact)> {
    let Outcome::Contacts(hits) = conic.query(Method::Intersect, TOL).unwrap() else {
        panic!("Expected finite conic contacts");
    };
    hits
}

#[test]
fn circle_and_mirrored_trim_project_with_exact_orientation() {
    let mut c = xy();
    assert_eq!(c.query(Method::Intersect, TOL), Ok(Outcome::Coincident));
    assert_eq!(
        c.query(Method::Coplanar, TOL),
        Ok(Outcome::Projection(Projection::Circle {
            center: [0., 0.],
            radius_mm: 5.
        }))
    );
    c.center[2] = 4.;
    assert_eq!(c.query(Method::Coplanar, TOL), Err(Error::OffPlane));
    assert!(contacts(c).is_empty());

    c.axes[1][1] = -1.;
    c.start_rad = -FRAC_PI_2;
    c.sweep_rad = -PI;
    let Outcome::Projection(Projection::Arc {
        center,
        a,
        b,
        clockwise,
    }) = c.query(Method::Project, TOL).unwrap()
    else {
        panic!("Expected finite arc");
    };
    assert_eq!(center, [0., 0.]);
    near(a[0], 0.);
    near(a[1], 5.);
    near(b[0], 0.);
    near(b[1], -5.);
    assert!(!clockwise);
}

#[test]
fn affine_rational_poles_preserve_projected_circle_and_ellipse_without_sampling() {
    for minor in [5., 2.] {
        let c = Conic {
            center: [3., -4., 7.],
            axes: [[1., 0., 0.], [0., 0.6, 0.8]],
            semiaxes_mm: [5., minor],
            ..xy()
        };
        let Outcome::Projection(Projection::RationalQuadratic { poles, weights }) =
            c.query(Method::Project, TOL).unwrap()
        else {
            panic!("Expected a rational quadratic basis");
        };
        assert_eq!(poles.len(), 9);
        assert_eq!(poles[0], poles[8]);
        for k in 0..4 {
            near(weights[2 * k + 1], std::f64::consts::FRAC_1_SQRT_2);
            for j in 0..=10 {
                let t = f64::from(j) / 10.;
                let b = [(1. - t).powi(2), 2. * t * (1. - t), t * t];
                let w = std::array::from_fn::<_, 3, _>(|i| b[i] * weights[2 * k + i]);
                let [x, y] = std::array::from_fn(|i| {
                    (0..3).map(|j| w[j] * poles[2 * k + j][i]).sum::<f64>() / w.iter().sum::<f64>()
                });
                near(
                    ((x - 3.) / 5.).powi(2) + ((y + 4.) / (minor * 0.6)).powi(2),
                    1.,
                );
            }
        }
        let trimmed = Conic {
            start_rad: PI / 6.,
            sweep_rad: -FRAC_PI_2,
            ..c
        };
        let Outcome::Projection(Projection::RationalQuadratic { poles, .. }) =
            trimmed.query(Method::Project, TOL).unwrap()
        else {
            panic!("Expected a finite rational arc");
        };
        assert_eq!(poles.len(), 3);
        assert_eq!(poles[0], c.point(PI / 6., 1.));
        assert_eq!(poles[2], c.point(PI / 6. - FRAC_PI_2, 1.));
    }
    assert_eq!(
        edge_on(0.).query(Method::Project, TOL),
        Err(Error::Singular)
    );
}

#[test]
fn full_circle_and_signed_trims_keep_finite_branch_parameters() {
    let c = edge_on(3.);
    let hit = contacts(c);
    assert_eq!(hit.len(), 2);
    near(hit[0].0[0], 7.);
    near(hit[1].0[0], 7.);
    near(hit[0].0[1], -4.);
    near(hit[1].0[1], 4.);
    assert_eq!(hit[0].1.crossing, Some(-1));
    assert_eq!(hit[1].1.crossing, Some(1));

    let clockwise = Conic {
        sweep_rad: -TAU,
        ..c
    };
    let reversed = contacts(clockwise);
    assert_eq!(reversed.len(), 2);
    for i in 0..2 {
        near(reversed[i].1.parameter + hit[1 - i].1.parameter, 1.);
        assert_eq!(reversed[i].1.crossing, hit[1 - i].1.crossing.map(|v| -v));
    }
    let trimmed = Conic {
        start_rad: 0.,
        sweep_rad: -FRAC_PI_2,
        ..c
    };
    assert_eq!(contacts(trimmed).len(), 1);
    assert!(
        contacts(Conic {
            sweep_rad: FRAC_PI_2,
            ..trimmed
        })
        .is_empty()
    );

    let seam = contacts(edge_on(0.));
    assert_eq!(seam.len(), 2);
    assert_eq!(seam[0].1.parameter, 0.);
    near(seam[1].1.parameter, 0.5);
    let endpoints = contacts(Conic {
        sweep_rad: -PI,
        ..edge_on(0.)
    });
    assert_eq!(
        endpoints.iter().map(|h| h.1.parameter).collect::<Vec<_>>(),
        [0., 1.]
    );
}

#[test]
fn tangency_is_flagged_and_uncertain_or_degenerate_inputs_fail_explicitly() {
    for depth in [5., -5.] {
        let hit = contacts(edge_on(depth));
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].1.crossing, None);
        near(hit[0].0[1], 0.);
    }
    assert!(contacts(edge_on(5.1)).is_empty());
    assert_eq!(
        edge_on(5_f64.next_down()).query(Method::Intersect, TOL),
        Err(Error::Unresolved)
    );
    assert_eq!(
        Conic {
            axes: [[1., 0., 0.]; 2],
            ..xy()
        }
        .query(Method::Project, TOL),
        Err(Error::Invalid)
    );
    assert_eq!(
        Conic {
            sweep_rad: TAU + 1e-10,
            ..xy()
        }
        .query(Method::Project, TOL),
        Err(Error::Invalid)
    );
    assert_eq!(xy().query(Method::Project, f64::NAN), Err(Error::Invalid));

    let tiny = Conic {
        sweep_rad: 1e-9,
        ..edge_on(0.)
    };
    assert_eq!(tiny.query(Method::Intersect, TOL), Ok(Outcome::Coincident));
}

#[test]
fn endpoint_roundoff_cannot_move_a_contact_past_modeling_precision() {
    let c = Conic {
        semiaxes_mm: [1e8; 2],
        ..edge_on(-5e-6)
    };
    assert_eq!(c.query(Method::Intersect, TOL), Err(Error::Unresolved));
}
