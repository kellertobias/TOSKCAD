use super::*;
use crate::manufacturing;

fn samples(geometry: &RoundedCylinder, options: Tessellation) -> f64 {
    let mesh = geometry.mesh(options).unwrap();
    let receipt = manufacturing::certify(&mesh).unwrap();
    let points = mesh.vertices.as_chunks::<3>().0;
    let mut labels = std::collections::BTreeSet::new();
    for (triangle, &index) in mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.face_indices)
    {
        let role = geometry.face_role(index).unwrap();
        labels.insert(index);
        let vertices = triangle.map(|i| points[i as usize]);
        let centroid =
            std::array::from_fn::<_, 3, _>(|i| vertices.iter().map(|p| p[i]).sum::<f64>() / 3.);
        let delta = |a: [f64; 3], b: [f64; 3]| std::array::from_fn::<_, 3, _>(|i| a[i] - b[i]);
        let a = delta(vertices[1], vertices[0]);
        let b = delta(vertices[2], vertices[0]);
        let normal = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = normal.into_iter().map(|x| x * x).sum::<f64>().sqrt();
        let rho = centroid[0].hypot(centroid[1]);
        let (distance, analytic) = match geometry.surface(role).unwrap() {
            Surface::Plane { z_mm, normal_z, .. } => {
                ((centroid[2] - z_mm).abs(), [0., 0., normal_z])
            }
            Surface::Cylinder { radius_mm, .. } => (
                (rho - radius_mm).abs(),
                [centroid[0] / rho, centroid[1] / rho, 0.],
            ),
            Surface::Toroid {
                major_radius_mm: a,
                minor_radius_mm: f,
                center_z_mm: z,
                ..
            } => {
                let radial = rho - a;
                let axial = centroid[2] - z;
                let norm = radial.hypot(axial);
                (
                    (norm - f).abs(),
                    [
                        radial / norm * centroid[0] / rho,
                        radial / norm * centroid[1] / rho,
                        axial / norm,
                    ],
                )
            }
        };
        assert!(
            distance <= options.deflection * (1. + 1e-10),
            "{distance} exceeds chord budget"
        );
        let cosine = normal
            .into_iter()
            .zip(analytic)
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / length;
        assert!(
            cosine >= options.angle.cos() - 1e-12,
            "normal angle {}",
            cosine.acos()
        );
        for point in vertices {
            assert!(geometry.contains(
                point,
                64. * f64::EPSILON * geometry.radius_mm().max(geometry.height_mm()).max(1.)
            ));
        }
    }
    assert_eq!(labels, (0..geometry.active_faces().len() as u32).collect());
    assert!(receipt.signed_volume_mm3 <= geometry.volume_mm3() + 1e-10);
    assert!(
        geometry.volume_mm3() - receipt.signed_volume_mm3
            <= geometry.surface_area_mm2() * options.deflection
    );
    receipt.signed_volume_mm3
}

#[test]
fn shared_ring_mesh_is_manifold_oriented_and_converges_on_every_active_patch() {
    for fillets in [
        [Some(1.), None],
        [None, Some(2.)],
        [Some(1.), Some(3.)],
        [Some(4.9), Some(4.8)],
    ] {
        let g = RoundedCylinder::new(5., 12., fillets, 1e-6, 1e7).unwrap();
        let coarse = samples(
            &g,
            Tessellation {
                deflection: 0.2,
                angle: 0.8,
                triangle_limit: 300_000,
            },
        );
        let fine = samples(
            &g,
            Tessellation {
                deflection: 0.01,
                angle: 0.15,
                triangle_limit: 300_000,
            },
        );
        assert!(fine > coarse);
        assert!(g.volume_mm3() - fine < g.volume_mm3() - coarse);
        for (i, role) in g.active_faces().into_iter().enumerate() {
            assert_eq!(g.face_index(role), Some(i as u32));
            assert_eq!(g.face_role(i as u32), Some(role));
        }
        assert_eq!(g.face_role(99), None);
        let missing = if fillets[0].is_none() {
            Some(Face::StartFillet)
        } else if fillets[1].is_none() {
            Some(Face::EndFillet)
        } else {
            None
        };
        if let Some(face) = missing {
            assert_eq!(g.face_index(face), None);
        }
    }
}

#[test]
fn budgets_are_checked_before_mesh_allocation_and_options_are_validated() {
    let g = RoundedCylinder::new(5., 12., [Some(1.), Some(3.)], 1e-6, 1e7).unwrap();
    let defaults = Tessellation {
        deflection: 0.1,
        angle: 0.3,
        triangle_limit: 100_000,
    };
    for options in [
        Tessellation {
            deflection: 0.,
            ..defaults
        },
        Tessellation {
            angle: f64::NAN,
            ..defaults
        },
        Tessellation {
            triangle_limit: 0,
            ..defaults
        },
    ] {
        assert_eq!(g.mesh(options).unwrap_err(), MeshError::InvalidOptions);
    }
    for options in [
        Tessellation {
            triangle_limit: 1,
            ..defaults
        },
        Tessellation {
            deflection: f64::MIN_POSITIVE,
            ..defaults
        },
        Tessellation {
            deflection: 0.01,
            triangle_limit: 100,
            ..defaults
        },
    ] {
        assert_eq!(g.mesh(options).unwrap_err(), MeshError::BudgetExceeded);
    }
    // Mixed tiny and large arcs cannot bypass the common output budget.
    let skew = RoundedCylinder::new(5., 12., [Some(1e-6), Some(4.)], 1e-6, 1e7).unwrap();
    assert_eq!(
        skew.mesh(Tessellation {
            deflection: 1e-8,
            angle: 0.3,
            triangle_limit: 100_000
        })
        .unwrap_err(),
        MeshError::BudgetExceeded
    );
    let fragile = RoundedCylinder::new(1e7, 1e7, [Some(1e-6), None], 1e-6, 1e7).unwrap();
    assert_eq!(
        fragile
            .mesh(Tessellation {
                deflection: 100.,
                angle: 0.02,
                triangle_limit: 5_000_000
            })
            .unwrap_err(),
        MeshError::UnresolvedBoundary
    );
    let distant = RoundedCylinder::new(1e-3, 1e7, [Some(1e-4), None], 1e-6, 1e7).unwrap();
    assert_eq!(
        distant
            .mesh(Tessellation {
                deflection: 1e-9,
                angle: 0.3,
                triangle_limit: 5_000_000
            })
            .unwrap_err(),
        MeshError::UnresolvedBoundary
    );
    samples(&g, defaults);
}

#[test]
fn extreme_supported_scales_keep_regular_patches_and_manifold_caps() {
    let tiny = RoundedCylinder::new(3e-6, 8e-6, [Some(1e-6); 2], 1e-6, 1e7).unwrap();
    samples(
        &tiny,
        Tessellation {
            deflection: 1e-8,
            angle: 0.3,
            triangle_limit: 300_000,
        },
    );
    // Nearly hemispherical stock still retains its minimum positive cap and
    // wall. The toroidal major radius is far smaller than its minor radius.
    let large = RoundedCylinder::new(1e7, 1e7, [Some(1e7 - 2e-6), None], 1e-6, 1e7).unwrap();
    samples(
        &large,
        Tessellation {
            deflection: 1000.,
            angle: 0.3,
            triangle_limit: 300_000,
        },
    );
    assert!((large.centroid_mm()[2] / 1e7 - 5. / 8.).abs() < 1e-12);
}
