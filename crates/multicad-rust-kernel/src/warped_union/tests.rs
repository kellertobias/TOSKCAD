use super::*;
use crate::{
    display::{MeshError, Tessellation},
    manufacturing::certify,
    planar_boolean::ConvexPiece,
};
use std::f64::consts::{PI, SQRT_2};

const BAND: Band = Band {
    center_z_mm: 10.,
    radius_mm: 2.,
};
const TOL: f64 = 1e-7;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < TOL, "{actual} != {expected}");
}

fn prism() -> ConvexPiece {
    ConvexPiece::new(
        vec![
            vec![[0., 0., 0.], [2., 0., 0.], [2., 0., 2.]],
            vec![[2., 10., 2.], [2., 10., 0.], [0., 10., 0.]],
            vec![[0., 10., 0.], [2., 10., 0.], [2., 0., 0.], [0., 0., 0.]],
            vec![[2., 10., 0.], [2., 10., 2.], [2., 0., 2.], [2., 0., 0.]],
            vec![[2., 10., 2.], [0., 10., 0.], [0., 0., 0.], [2., 0., 2.]],
        ],
        TOL,
    )
    .unwrap()
}

fn double_wedge() -> ConvexPiece {
    let [a, b, c, d, peak] = [
        [0., 0., 0.],
        [2., 0., 0.],
        [2., 2., 0.],
        [0., 2., 0.],
        [2., 2., 2.],
    ];
    ConvexPiece::new(
        vec![
            vec![d, c, b, a],
            vec![a, b, peak],
            vec![b, c, peak],
            vec![c, d, peak],
            vec![d, a, peak],
        ],
        TOL,
    )
    .unwrap()
}

#[test]
fn planar_boolean_boundary_keeps_coordinates_and_accepts_an_exact_blind_bore() {
    let piece = ConvexPiece::from_box([-4., -4., -3.], [4., 4., 2.], TOL).unwrap();
    let union = Union::new(&[piece], TOL).unwrap();
    let solid = from_planar_union(&union, TOL).unwrap();
    assert_eq!(solid.bounds_mm(), union.bounds());
    assert_eq!(solid.vertices(), union.vertices());
    assert_eq!(solid.volume_mm3(), union.volume_mm3());
    let bore = AxialCut {
        start_mm: [0., 0., 3.],
        end_mm: [0., 0., 0.],
        radius_mm: 1.,
        source: 1,
        wall_source_face: 2,
        cap_source_faces: [0, 1],
        recess: None,
    };
    let cut = solid.cut_axial(&[bore], TOL).unwrap();
    near(cut.volume_mm3(), 320. - 2. * PI);
    assert_eq!(cut.bounds_mm(), solid.bounds_mm());
    assert_eq!(cut.solid_count(), 1);
    let mesh = cut
        .mesh(Tessellation {
            deflection: 0.01,
            angle: 0.1,
            triangle_limit: 2000,
        })
        .unwrap();
    certify(&mesh).unwrap();
}

#[test]
fn recovers_plane_and_cylinder_faces_with_exact_metrics() {
    let shell = remap(&Union::new(&[prism()], TOL).unwrap(), BAND, TOL).unwrap();
    assert_eq!(shell.vertices().len(), 6);
    assert_eq!(shell.faces().len(), 5);
    assert_eq!(shell.edges().len(), 9);
    assert_eq!(shell.bounds_mm(), [[0., 0., 8.], [2., 10., 10.]]);
    assert_eq!(shell.solid_count(), 1);
    assert_eq!(shell.shell_count(), 1);
    near(shell.volume_mm3(), 10. * (4. - PI));
    for face in shell.faces() {
        assert_eq!(face.source, 0);
        match face.support {
            Support::Cylinder {
                axis_origin_mm,
                axis_direction,
                radius_mm,
                outward,
            } => {
                assert_eq!(axis_origin_mm, [0., 0., 10.]);
                assert_eq!(axis_direction, [0., 1., 0.]);
                assert_eq!(radius_mm, 2.);
                assert!(!outward);
                near(
                    length3(cross(sub(face.point_mm, axis_origin_mm), axis_direction)),
                    2.,
                );
                near(face.area_mm2, 10. * PI);
            }
            Support::Plane { normal } if normal[1] != 0. => near(face.area_mm2, 4. - PI),
            Support::Plane { .. } => near(face.area_mm2, 20.),
        }
    }
    let tessellation = Tessellation {
        deflection: 0.001,
        angle: 0.1,
        triangle_limit: 1000,
    };
    let mesh = shell.mesh(tessellation).unwrap();
    let proof = certify(&mesh).unwrap();
    let chord_error = proof.signed_volume_mm3 - shell.volume_mm3();
    assert!(chord_error > 0. && chord_error < 10. * PI * tessellation.deflection);
    assert_eq!(
        shell.mesh(Tessellation {
            triangle_limit: 4,
            ..tessellation
        }),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn perpendicular_cylinder_patches_share_the_exact_elliptic_edge() {
    let shell = remap(&Union::new(&[double_wedge()], TOL).unwrap(), BAND, TOL).unwrap();
    near(shell.volume_mm3(), 8. * (5. / 3. - PI * 0.5));
    let shared = shell
        .edges()
        .iter()
        .find_map(|edge| match edge.curve {
            Curve::Ellipse(arc) if arc.ellipse().semiaxes_mm()[0] > 2.1 => Some(arc),
            _ => None,
        })
        .unwrap();
    let ellipse = shared.ellipse();
    near(ellipse.semiaxes_mm()[0], 2. * SQRT_2);
    near(ellipse.semiaxes_mm()[1], 2.);
    let point = ellipse.point_at(shared.start_rad() + 0.5 * shared.sweep_rad());
    near(point[0], point[1]);
    near(point[0].hypot(point[2] - 10.), 2.);
    assert_eq!(
        shell
            .faces()
            .iter()
            .filter(|face| matches!(face.support, Support::Cylinder { .. }))
            .count(),
        2
    );
    for face in shell.faces() {
        if matches!(face.support, Support::Cylinder { .. }) {
            near(face.area_mm2, 4. * (PI * 0.5 - 1.));
        }
    }
    let mesh = shell
        .mesh(Tessellation {
            deflection: 0.0003,
            angle: 0.1,
            triangle_limit: 2000,
        })
        .unwrap();
    let proof = certify(&mesh).unwrap();
    assert!((proof.signed_volume_mm3 - shell.volume_mm3()).abs() < 0.005);
}

#[test]
fn rigid_bands_join_the_round_band_without_exposed_interface_caps() {
    let slabs = [[-3., 0.], [0., 2.], [2., 4.]]
        .map(|[lo, hi]| ConvexPiece::from_box([0., 0., lo], [4., 3., hi], TOL).unwrap());
    let shell = remap(&Union::new(&slabs, TOL).unwrap(), BAND, TOL).unwrap();
    assert_eq!(shell.bounds_mm(), [[0., 0., 5.], [4., 3., 12.]]);
    near(shell.volume_mm3(), 4. * 3. * 7.);
    assert_eq!(
        shell
            .faces()
            .iter()
            .filter(|face| matches!(
                face.support,
                Support::Plane {
                    normal: [0., 0., _]
                }
            ))
            .count(),
        2
    );
    let mesh = shell
        .mesh(Tessellation {
            deflection: 0.01,
            angle: 0.1,
            triangle_limit: 1000,
        })
        .unwrap();
    near(
        certify(&mesh).unwrap().signed_volume_mm3,
        shell.volume_mm3(),
    );
}

#[test]
fn unsupported_warps_stop_before_publishing_a_shell() {
    let stock = Union::new(&[prism()], TOL).unwrap();
    for (band, tolerance, error) in [
        (BAND, 0., Error::Invalid),
        (
            Band {
                radius_mm: 0.,
                ..BAND
            },
            TOL,
            Error::Invalid,
        ),
        (
            Band {
                center_z_mm: 2e8,
                ..BAND
            },
            TOL,
            Error::Limit,
        ),
        (
            Band {
                radius_mm: 1.,
                ..BAND
            },
            TOL,
            Error::Unsupported,
        ),
        (
            Band {
                radius_mm: f64::NAN,
                ..BAND
            },
            TOL,
            Error::Invalid,
        ),
    ] {
        assert_eq!(remap(&stock, band, tolerance).unwrap_err(), error);
    }
    assert_eq!(metric_work(MAX_METRIC_WORK - 16, 4), Ok(MAX_METRIC_WORK));
    assert_eq!(metric_work(MAX_METRIC_WORK - 16, 5), Err(Error::Limit));
    assert_eq!(positive_volume(0.), Err(Error::Invalid));
    assert_eq!(positive_volume(f64::NAN), Err(Error::Invalid));
}

#[test]
fn unsupported_supports_require_analytic_residual_checks() {
    let horizontal = [[0., 0., 1.], [1., 0., 1.], [0., 1., 1.]];
    let support_at = |ring: &[[f64; 3]], normal| support(BAND, Region::Round, ring, normal, TOL);
    assert_eq!(
        support_at(&horizontal, [0., 0., 1.]),
        Ok(Support::Plane {
            normal: [0., 0., 1.]
        })
    );
    let shifted = [[0., 0., 1.], [1., 0., 1.1], [0., 1., 1.]];
    assert_eq!(support_at(&shifted, [0., 0., 1.]), Err(Error::Unsupported));
    assert_eq!(
        support_at(&horizontal, [1., 0., 0.]),
        Err(Error::Unsupported)
    );
    assert_eq!(
        support_at(&horizontal, [1., 1., -1.]),
        Err(Error::Unsupported)
    );
    let r = std::f64::consts::FRAC_1_SQRT_2;
    assert_eq!(
        support_at(&horizontal, [r, 0., -r]),
        Err(Error::Unsupported)
    );
    assert!(matches!(
        support_at(&[[1., 0., 1.], [2., 1., 2.], [2., 0., 2.]], [r, 0., -r]),
        Ok(Support::Cylinder { outward: true, .. })
    ));
}

#[test]
fn oblique_edge_orientation_and_residual_are_verified() {
    let chart = [[0., 0., 0.], [0.5, 0., 2.]];
    let Curve::Ellipse(arc) = edge_curve(BAND, chart, TOL).unwrap() else {
        panic!("a shorter horizontal semi-axis is still an ellipse");
    };
    near(arc.ellipse().semiaxes_mm()[0], 2.);
    near(arc.ellipse().semiaxes_mm()[1], 0.5);
    let [first, last] = arc.endpoints();
    near(length3(sub(first, BAND.point(chart[0]))), 0.);
    near(length3(sub(last, BAND.point(chart[1]))), 0.);
    assert_eq!(edge_curve(BAND, chart, 0.), Err(Error::Unsupported));
}

#[test]
fn narrow_intervals_and_tessellation_limits_are_independent_of_volume() {
    let spans = [[0., 1.], [1., 1.0001], [1.0001, 2.]]
        .map(|[lo, hi]| ConvexPiece::from_box([0., 0., lo], [4., 3., hi], TOL).unwrap());
    let mut shell = remap(&Union::new(&spans, TOL).unwrap(), BAND, TOL).unwrap();
    near(shell.volume_mm3(), 24.);
    let fine = Tessellation {
        deflection: 0.01,
        angle: 0.1,
        triangle_limit: 1000,
    };
    assert_eq!(
        shell.mesh(Tessellation {
            triangle_limit: 1,
            ..fine
        }),
        Err(MeshError::BudgetExceeded)
    );
    let proof = certify(&shell.mesh(fine).unwrap()).unwrap();
    near(proof.signed_volume_mm3, 24.);
    shell.edges.push(shell.edges[0]);
    assert_eq!(shell.mesh(fine), Err(MeshError::UnresolvedBoundary));
}
