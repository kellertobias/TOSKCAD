use super::*;
fn work() -> Work<'static> {
    Work {
        remaining: 100_000,
        interrupt: None,
    }
}
fn axis() -> Axis {
    Axis {
        origin: [0.; 3],
        normal: [0., 0., 1.],
        reference: [1., 0., 0.],
    }
}
#[test]
fn whole_curve_cone_support_refines_bernstein_hulls_and_checks_each_source_span() {
    let delta = 1e-8;
    let poles = (0..=9)
        .map(|i| {
            [
                1. + 4. * delta * (i * (9 - i)) as f64 / 72.,
                0.,
                i as f64 / 9.,
            ]
        })
        .collect::<Vec<_>>();
    let curve = Curve::Spline { spans: 1, poles };
    let error = cone_curve(&curve, axis(), 1., 0., 2.01e-8, &mut work()).unwrap();
    assert!(error >= delta && error <= 2.01e-8);
    assert!(cone_curve(&curve, axis(), 1., 0., delta / 4., &mut work()).is_err());
    assert_eq!(
        cone_curve(
            &curve,
            axis(),
            1.,
            0.,
            2.01e-8,
            &mut Work {
                remaining: 1,
                interrupt: None
            }
        ),
        Err(Error::Limit)
    );
    assert_eq!(
        cone_curve(
            &curve,
            axis(),
            1.,
            0.,
            2.01e-8,
            &mut Work {
                remaining: 1000,
                interrupt: Some(&|| true)
            }
        ),
        Err(Error::Cancelled)
    );
    assert!(cone_curve(&curve, axis(), 1., 0., 2.01e-8, &mut work()).is_ok());
}
#[test]
fn plane_and_cone_checks_do_not_drop_unsupported_or_uncertain_curves() {
    let circle = Curve::Circle {
        axis: Axis {
            normal: [0., 1., 0.],
            ..axis()
        },
        radius_mm: 1.,
    };
    assert!(plane_curve(&circle, axis(), &mut work()).unwrap() > 0.9);
    assert_eq!(
        cone_curve(&circle, axis(), 1., 0., 1e-7, &mut work()),
        Err(Error::Unresolved)
    );
    let invalid = Curve::Spline {
        spans: 1,
        poles: vec![[f64::INFINITY, 0., 0.]; 10],
    };
    assert_eq!(
        plane_curve(&invalid, axis(), &mut work()),
        Err(Error::Unresolved)
    );
    let source = crate::radial_tensor::tests::fixture();
    assert_eq!(
        source.clone().bind_surfaces(0., None).err(),
        Some(Error::Invalid)
    );
    assert_eq!(
        source.clone().bind_surfaces(1e-30, None).err(),
        Some(Error::Unresolved)
    );
    let tensor = source
        .input()
        .faces
        .iter()
        .position(|f| matches!(source.input().surfaces[f.surface], Surface::Tensor(_)))
        .unwrap();
    let mut trim = source.input().faces[tensor].clone();
    trim.outer.truncate(3);
    let Surface::Tensor(t) = &source.input().surfaces[trim.surface] else {
        unreachable!()
    };
    assert_eq!(
        tensor_wire(source.input(), &trim, t, &mut work()).err(),
        Some(Error::Invalid)
    );
    let mut shifted = source.input().clone();
    if let Surface::Plane(a) = &mut shifted.surfaces[0] {
        a.origin[2] += 0.01;
    }
    assert_eq!(
        BoundaryOwnership::new(shifted, 1e-7, None)
            .unwrap()
            .bind_surfaces(1e-7, None)
            .err(),
        Some(Error::Unresolved)
    );
    assert!(source.bind_surfaces(1e-7, None).is_ok());
}
#[test]
fn endpoint_coincidence_does_not_replace_the_actual_tensor_iso_curve() {
    let original = crate::radial_tensor::tests::fixture().input().clone();
    let curve = original
        .curves
        .iter()
        .position(|c| matches!(c, Curve::Spline { .. }))
        .unwrap();
    let Curve::Spline { poles, .. } = &original.curves[curve] else {
        unreachable!()
    };
    let a = poles[0];
    let b = poles[poles.len() - 1];
    let center = std::array::from_fn(|i| (a[i] + b[i]) * 0.5);
    let radius = a
        .iter()
        .zip(center)
        .map(|(a, c)| (a - c).powi(2))
        .sum::<f64>()
        .sqrt();
    let mut circle = original.clone();
    circle.curves[curve] = Curve::Circle {
        axis: Axis {
            origin: center,
            normal: [0., 0., 1.],
            reference: std::array::from_fn(|i| a[i] - center[i]),
        },
        radius_mm: radius,
    };
    let source = BoundaryOwnership::new(circle, 1e-7, None).unwrap();
    assert_eq!(
        source.bind_surfaces(1e-7, None).err(),
        Some(Error::Unresolved)
    );
    let mut moved = original.clone();
    if let Curve::Spline { poles, .. } = &mut moved.curves[curve] {
        poles[1][0] += 0.001;
    }
    let source = BoundaryOwnership::new(moved, 1e-7, None).unwrap();
    assert_eq!(
        source.bind_surfaces(1e-7, None).err(),
        Some(Error::Unresolved)
    );
    assert!(
        BoundaryOwnership::new(original, 1e-7, None)
            .unwrap()
            .bind_surfaces(1e-7, None)
            .is_ok()
    );
}

#[test]
fn bounded_support_work_cancels_inside_each_face_kind_and_rejects_cone_apex_crossings() {
    let source = crate::radial_tensor::tests::fixture();
    for kind in 0..3 {
        let mut input = source.input().clone();
        let face = input
            .faces
            .iter()
            .position(|f| {
                matches!(
                    (&input.surfaces[f.surface], kind),
                    (Surface::Plane(_), 0) | (Surface::Cone { .. }, 1) | (Surface::Tensor(_), 2)
                )
            })
            .unwrap();
        input.faces.swap(0, face);
        let edge = input.faces[0]
            .outer
            .iter()
            .position(|&(e, _)| matches!(input.curves[input.edges[e].curve], Curve::Spline { .. }))
            .unwrap();
        input.faces[0].outer.rotate_left(edge);
        let raw = BoundaryOwnership::new(input, 1e-7, None).unwrap();
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() == 3
        };
        assert_eq!(
            raw.bind_surfaces(1e-7, Some(&cancel)).err(),
            Some(Error::Cancelled)
        );
        assert_eq!(calls.get(), 3);
    }
    let beyond_apex = Curve::Spline {
        spans: 1,
        poles: vec![[1., 0., -2.]; 10],
    };
    assert_eq!(
        cone_curve(
            &beyond_apex,
            axis(),
            1.,
            std::f64::consts::FRAC_PI_4,
            1e-7,
            &mut work()
        ),
        Err(Error::Unresolved)
    );
    assert!(source.bind_surfaces(1e-7, None).is_ok());
}
