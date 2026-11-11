use super::*;
use crate::spur_gear::herringbone::tests::fixture;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn coaxial_extensions_keep_tooth_ordinals_full_bores_and_analytic_support() {
    for helical in [false, true] {
        for hand in [-1., 1.] {
            for bore in [0., 4.] {
                let (spec, twist) = fixture(24);
                let spec = Spec {
                    bore_radius_mm: bore,
                    ..spec
                };
                let plain = if helical {
                    Herringbone::new_helical(spec, hand * twist, 1e-5, 1e7)
                } else {
                    Herringbone::new(spec, hand * twist, 1e-5, 1e7)
                }
                .unwrap();
                let original = plain.topology();
                for span in [[-2., 3.], [1., 6.], [-2., 6.], [1., 3.]] {
                    let hub = Hub {
                        radius_mm: 8.,
                        axial_start_mm: span[0],
                        axial_end_mm: span[1],
                    };
                    let gear = plain.clone().with_hub(hub, 1e-5, 1e7).unwrap();
                    let bounds = [span[0].min(0.), span[1].max(4.)];
                    assert_eq!(gear.axial_bounds(), bounds);
                    near(
                        gear.volume_mm3() - plain.volume_mm3(),
                        PI * (64. - bore * bore) * (bounds[1] - bounds[0] - 4.),
                    );
                    near(gear.support([0., 0., 2.]), 2. * bounds[1]);
                    near(gear.support([0., 0., -3.]), -3. * bounds[0]);
                    let extensions = gear.hub_extensions();
                    let count = extensions.iter().flatten().count();
                    let topology = gear.topology();
                    assert_eq!(topology.faces.len(), original.faces.len() + 2 * count);
                    assert_eq!(topology.edges.len(), original.edges.len() + 3 * count);
                    assert_eq!(
                        topology.vertices_mm.len(),
                        original.vertices_mm.len() + 2 * count
                    );
                    assert_eq!(
                        &topology.faces[..original.faces.len()],
                        original.faces.as_slice()
                    );
                    for extension in extensions.into_iter().flatten() {
                        assert_eq!(
                            topology.faces[extension.wall_face as usize],
                            Face::HubWall { extension }
                        );
                        assert_eq!(
                            topology.faces[extension.cap_face as usize],
                            Face::HubCap { extension }
                        );
                        assert_eq!(
                            topology.faces[extension.wall_face as usize].role(),
                            "gear/hub.side"
                        );
                        assert_eq!(
                            topology.faces[extension.cap_face as usize].role(),
                            if extension.end_z_mm < extension.gear_z_mm {
                                "gear/hub.end.start"
                            } else {
                                "gear/hub.end.end"
                            }
                        );
                    }
                    for edge in &topology.edges {
                        assert!(
                            edge.faces
                                .into_iter()
                                .all(|face| (face as usize) < topology.faces.len())
                        );
                        for (fraction, vertex) in [0., 1.].into_iter().zip(edge.vertices) {
                            for (actual, expected) in edge
                                .curve
                                .point(&gear, fraction)
                                .into_iter()
                                .zip(topology.vertices_mm[vertex as usize])
                            {
                                near(actual, expected);
                            }
                        }
                        if let Curve::BoreRim { end, .. } = edge.curve {
                            let i = usize::from(end);
                            let cap =
                                extensions[i].map_or(i as u32, |extension| extension.cap_face);
                            assert!(edge.faces.contains(&cap));
                            assert_eq!(edge.curve.point(&gear, 0.25)[2], bounds[i]);
                        }
                        if let Curve::HubRim { radius_mm, z_mm } = edge.curve {
                            let point = edge.curve.point(&gear, 0.25);
                            near(point[0], 0.);
                            near(point[1], radius_mm);
                            assert_eq!(point[2], z_mm);
                        }
                        if let Curve::HubSeam {
                            radius_mm,
                            span_mm: [lo, hi],
                        } = edge.curve
                        {
                            assert_eq!(edge.faces[0], edge.faces[1]);
                            assert_eq!(
                                edge.curve.point(&gear, 0.5),
                                [radius_mm, 0., (lo + hi) / 2.]
                            );
                        }
                    }
                    for direction in [[1_f64, 0.3, 20.], [-0.2, 1., -20.], [1., 0., 0.]] {
                        let [dx, dy, dz] = direction;
                        let cylinder =
                            dx.hypot(dy) * hub.radius_mm + (dz * span[0]).max(dz * span[1]);
                        near(
                            gear.support(direction),
                            plain.support(direction).max(cylinder),
                        );
                    }
                    assert_eq!(gear.bounds()[0][2], bounds[0]);
                    assert_eq!(gear.bounds()[1][2], bounds[1]);
                }
            }
        }
    }
}

#[test]
fn invalid_hub_keeps_the_same_profile_failure_contract() {
    let (spec, twist) = fixture(24);
    let gear = Herringbone::new_helical(spec, twist, 1e-5, 1e7).unwrap();
    let valid = Hub {
        radius_mm: 8.,
        axial_start_mm: -2.,
        axial_end_mm: 6.,
    };
    assert_eq!(
        gear.clone().with_hub(
            Hub {
                radius_mm: f64::NAN,
                ..valid
            },
            1e-5,
            1e7
        ),
        Err(Error::InvalidDimensions)
    );
    assert_eq!(
        gear.clone().with_hub(
            Hub {
                radius_mm: spec.root_radius_mm,
                ..valid
            },
            1e-5,
            1e7
        ),
        Err(Error::InvalidProfile)
    );
    assert_eq!(
        gear.with_hub(
            Hub {
                axial_start_mm: 5.,
                axial_end_mm: 6.,
                ..valid
            },
            1e-5,
            1e7
        ),
        Err(Error::InvalidProfile)
    );
}
