use super::*;

fn geometry(f: [Option<f64>; 2]) -> RoundedCylinder {
    RoundedCylinder::new(5., 12., f, 1e-6, 1e7).unwrap()
}
fn close(a: f64, b: f64) {
    assert!(
        (a - b).abs() < 1e-9 * a.abs().max(b.abs()).max(1.),
        "{a} != {b}"
    );
}
fn numerical(geometry: &RoundedCylinder) -> [f64; 3] {
    let r = geometry.radius_mm();
    let h = geometry.height_mm();
    let [a, b] = geometry.fillet_radii_mm().map(|f| f.unwrap_or(0.));
    let mut result = [
        PI * r * r * (h - a - b),
        PI * r * r * ((h - b).powi(2) - a * a) / 2.,
        TAU * r * (h - a - b) + PI * ((r - a).powi(2) + (r - b).powi(2)),
    ];
    for (i, f) in geometry.fillet_radii_mm().into_iter().enumerate() {
        if let Some(f) = f {
            let n = 2048;
            for j in 0..=n {
                let angle = if i == 0 { -FRAC_PI_2 } else { 0. } + FRAC_PI_2 * j as f64 / n as f64;
                let rho = r - f + f * angle.cos();
                let z = if i == 0 { f } else { h - f } + f * angle.sin();
                let dz = f * angle.cos();
                let weight = if j == 0 || j == n {
                    1.
                } else if j % 2 == 0 {
                    2.
                } else {
                    4.
                };
                for (total, value) in result.iter_mut().zip([
                    PI * rho * rho * dz,
                    PI * rho * rho * z * dz,
                    TAU * rho * f,
                ]) {
                    *total += value * weight * FRAC_PI_2 / (3. * n as f64);
                }
            }
        }
    }
    result
}

#[test]
fn exact_metrics_match_independent_meridian_quadrature_and_scale_symmetry() {
    for radii in [
        [Some(1.), None],
        [None, Some(2.)],
        [Some(1.), Some(3.)],
        [Some(4.9), Some(4.8)],
        [Some(3.), Some(3.)],
    ] {
        let g = geometry(radii);
        assert_eq!(g.radius_mm(), 5.);
        assert_eq!(g.height_mm(), 12.);
        assert_eq!(g.fillet_radii_mm(), radii);
        let [volume, moment, area] = numerical(&g);
        let green_volume = TAU
            * g.meridian()
                .iter()
                .map(|edge| edge.moments([0.; 2])[1])
                .sum::<f64>();
        close(g.volume_mm3(), green_volume);
        close(g.volume_mm3(), volume);
        close(g.centroid_mm()[2], moment / volume);
        close(g.surface_area_mm2(), area);
        assert_eq!(&g.centroid_mm()[..2], &[0., 0.]);
        let flipped = geometry([radii[1], radii[0]]);
        close(g.volume_mm3(), flipped.volume_mm3());
        close(g.surface_area_mm2(), flipped.surface_area_mm2());
        close(g.centroid_mm()[2] + flipped.centroid_mm()[2], 12.);
        let scale = 3.7;
        let scaled = RoundedCylinder::new(
            5. * scale,
            12. * scale,
            radii.map(|f| f.map(|f| f * scale)),
            1e-6,
            1e7,
        )
        .unwrap();
        close(scaled.volume_mm3(), g.volume_mm3() * scale.powi(3));
        close(
            scaled.surface_area_mm2(),
            g.surface_area_mm2() * scale.powi(2),
        );
        close(scaled.centroid_mm()[2], g.centroid_mm()[2] * scale);
        for direction in [
            [0_f64, 0., 1.],
            [0., 0., -1.],
            [1., 0., 0.],
            [0.2, -0.4, 0.7],
            [-0.3, 0.4, -0.8],
        ] {
            let [x, y, z] = direction;
            let radial = x.hypot(y);
            let mut dense = f64::NEG_INFINITY;
            let mut visit = |rho: f64, axial: f64| dense = dense.max(radial * rho + z * axial);
            visit(5. - radii[0].unwrap_or(0.), 0.);
            visit(5. - radii[1].unwrap_or(0.), 12.);
            visit(5., radii[0].unwrap_or(0.));
            visit(5., 12. - radii[1].unwrap_or(0.));
            for (i, f) in radii.into_iter().enumerate() {
                if let Some(f) = f {
                    for j in 0..=2048 {
                        let phase =
                            if i == 0 { -FRAC_PI_2 } else { 0. } + FRAC_PI_2 * j as f64 / 2048.;
                        visit(
                            5. - f + f * phase.cos(),
                            if i == 0 { f } else { 12. - f } + f * phase.sin(),
                        );
                    }
                }
            }
            let support = g.support(direction);
            assert!(support >= dense - 1e-12 && support - dense < 1e-5);
            close(scaled.support(direction), support * scale);
            close(flipped.support([x, y, -z]), support - z * 12.);
        }
    }
    close(geometry([Some(3.); 2]).centroid_mm()[2], 6.);
}

#[test]
fn named_patches_and_rims_retain_original_ordinals_and_allow_large_valid_fillets() {
    for radii in [[Some(1.), None], [None, Some(3.)], [Some(4.), Some(3.)]] {
        let g = geometry(radii);
        let rims = g.rims();
        assert_eq!(rims.len(), 2 + radii.into_iter().flatten().count());
        for (i, f) in radii.into_iter().enumerate() {
            let cap = if i == 0 { Face::StartCap } else { Face::EndCap };
            let patch = if i == 0 {
                Face::StartFillet
            } else {
                Face::EndFillet
            };
            let z = if i == 0 { 0. } else { 12. };
            assert_eq!(
                g.surface(cap),
                Some(Surface::Plane {
                    radius_mm: 5. - f.unwrap_or(0.),
                    z_mm: z,
                    normal_z: if i == 0 { -1. } else { 1. }
                })
            );
            if let Some(f) = f {
                assert_eq!(
                    g.surface(patch),
                    Some(Surface::Toroid {
                        major_radius_mm: 5. - f,
                        minor_radius_mm: f,
                        center_z_mm: if i == 0 { f } else { 12. - f },
                        meridian_rad: if i == 0 {
                            [-FRAC_PI_2, 0.]
                        } else {
                            [0., FRAC_PI_2]
                        }
                    })
                );
                let cap_rim = rims
                    .iter()
                    .find(|rim| rim.ordinal == 2 + 2 * i as u32)
                    .unwrap();
                assert_eq!(cap_rim.faces, [cap, patch]);
                assert_eq!(cap_rim.radius_mm, 5. - f);
                assert_eq!(cap_rim.z_mm, z);
                let wall_rim = rims
                    .iter()
                    .find(|rim| rim.ordinal == 3 + 2 * i as u32)
                    .unwrap();
                assert_eq!(wall_rim.faces, [Face::Wall, patch]);
                assert_eq!(wall_rim.radius_mm, 5.);
                assert_eq!(wall_rim.z_mm, if i == 0 { f } else { 12. - f });
            } else {
                assert_eq!(g.surface(patch), None);
                assert_eq!(g.face_area_mm2(patch), None);
                let rim = rims.iter().find(|r| r.ordinal == i as u32).unwrap();
                assert_eq!(rim.faces, [cap, Face::Wall]);
            }
        }
        assert_eq!(
            g.surface(Face::Wall),
            Some(Surface::Cylinder {
                radius_mm: 5.,
                span_mm: [radii[0].unwrap_or(0.), 12. - radii[1].unwrap_or(0.)]
            })
        );
        assert_eq!(Face::StartCap as u32, 0);
        assert_eq!(Face::EndCap as u32, 1);
        assert_eq!(Face::Wall as u32, 2);
        assert_eq!(Face::StartFillet as u32, 3);
        assert_eq!(Face::EndFillet as u32, 4);
        let edges = g.meridian();
        for (a, b) in edges.iter().zip(edges.iter().cycle().skip(1)) {
            assert_eq!(a.end(), b.start());
        }
    }
}

#[test]
fn occupied_material_and_support_handle_boundaries_invalid_queries_and_scale_extremes() {
    let g = geometry([Some(4.), Some(3.)]);
    for p in [
        [0., 0., 0.],
        [1., 0., 0.],
        [5., 0., 4.],
        [5., 0., 9.],
        [2., 0., 12.],
        [0., 0., 6.],
    ] {
        assert!(g.contains(p, 0.), "{p:?}");
    }
    for p in [
        [5., 0., 0.],
        [5., 0., 12.],
        [5.1, 0., 6.],
        [0., 0., -0.1],
        [0., 0., 12.1],
        [f64::MAX, f64::MAX, 6.],
    ] {
        assert!(!g.contains(p, 0.), "{p:?}");
    }
    for (p, tol) in [
        ([f64::NAN, 0., 0.], 0.),
        ([0.; 3], f64::NAN),
        ([0.; 3], -1.),
    ] {
        assert!(!g.contains(p, tol));
    }
    for radii in [[Some(1.), None], [None, Some(2.)]] {
        let g = geometry(radii);
        assert!(g.contains([4., 0., 6.], 0.));
        assert!(g.contains([5. + 1e-8, 0., 6.], 1e-7));
    }
    assert_eq!(g.support([0.; 3]), 0.);
    assert!(g.support([f64::NAN, 0., 0.]).is_nan());
    assert!(g.support([0., f64::INFINITY, 0.]).is_nan());
    close(g.support([1e-200, 0., 0.]) / 1e-200, 5.);
    let tiny = RoundedCylinder::new(3e-6, 8e-6, [Some(1e-6), Some(1e-6)], 1e-6, 1e7).unwrap();
    assert!(tiny.volume_mm3() > 0.);
    assert!(tiny.contains([0., 0., 4e-6], 0.));
    let large = RoundedCylinder::new(1e7, 1e7, [Some(1.), None], 1e-6, 1e7).unwrap();
    assert!(large.volume_mm3().is_finite());
    close(large.centroid_mm()[2], 5e6);
}

#[test]
fn invalid_dimensions_unproved_topology_and_limits_are_explicit_and_recoverable() {
    for (r, h, f, min, max) in [
        (f64::NAN, 12., [Some(1.), None], 1e-6, 1e7),
        (5., 0., [Some(1.), None], 1e-6, 1e7),
        (5., 12., [None, None], 1e-6, 1e7),
        (5., 12., [Some(0.), None], 1e-6, 1e7),
        (5., 12., [Some(f64::INFINITY), None], 1e-6, 1e7),
        (5., 12., [Some(1.), None], 0., 1e7),
        (5., 12., [Some(1.), None], 1., 0.5),
    ] {
        assert_eq!(
            RoundedCylinder::new(r, h, f, min, max).unwrap_err(),
            Error::InvalidDimensions
        );
    }
    for (r, h, max) in [(5., 12., 1e8), (11., 12., 10.), (5., 11., 10.)] {
        assert_eq!(
            RoundedCylinder::new(r, h, [Some(1.), None], 1e-6, max).unwrap_err(),
            Error::Limit
        );
    }
    for f in [[Some(5.), None], [Some(6.), None], [Some(4.), Some(4.)]] {
        assert_eq!(
            RoundedCylinder::new(5., 8., f, 1e-6, 1e7).unwrap_err(),
            Error::Degenerate
        );
    }
    assert!(geometry([Some(1.), None]).volume_mm3() > 0.);
}
