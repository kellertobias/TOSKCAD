use super::*;
use crate::{
    display::{MeshError, Tessellation},
    manufacturing::certify,
    revolution::length,
};
use std::collections::BTreeSet;

const MINIMUM: f64 = 1e-5;
const MAXIMUM: f64 = 1e7;
const OPTIONS: Tessellation = Tessellation {
    deflection: 0.005,
    angle: 0.15,
    triangle_limit: 100_000,
};

pub(in crate::spur_gear) fn fixture(teeth: u32) -> (Spec, f64) {
    // Recipe of the saved 24/36 continuous pair. These transverse
    // equations are also checked against a direct native STEP readback.
    let beta = 15_f64.to_radians();
    let pressure = (20_f64.to_radians().tan() / beta.cos()).atan();
    let transverse_module = 2. / beta.cos();
    let pitch = f64::from(teeth) * transverse_module / 2.;
    (
        Spec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * pressure.cos(),
            tip_radius_mm: pitch + 2.,
            root_radius_mm: pitch - 2.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: (PI * transverse_module / 2. - 0.05) / (2. * pitch)
                + pressure.tan()
                - pressure,
            face_width_mm: 4.,
            bore_radius_mm: 4.,
        },
        if teeth == 24 { 1. } else { -1. } * beta.tan() / pitch,
    )
}

fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} ±{tolerance}");
}

#[test]
fn single_hand_sweeps_keep_the_full_linear_phase_and_closed_named_shell() {
    for hand in [-1., 1.] {
        let (spec, twist) = fixture(24);
        let twist = hand * twist;
        let gear = Herringbone::new_helical(spec, twist, MINIMUM, MAXIMUM).unwrap();
        assert!(gear.is_helical());
        assert!(
            !Herringbone::new(spec, twist, MINIMUM, MAXIMUM)
                .unwrap()
                .is_helical()
        );
        for band in [Band::Lower, Band::Upper] {
            assert_eq!(gear.band_twist_rad_per_mm(band), twist);
        }
        let point = [spec.tip_radius_mm, 0.];
        for z in [0., 1., 2., 3., 4.] {
            assert_eq!(gear.phase_at(z), twist * z);
            let actual = gear.section_point(point, z);
            let (sin, cos) = (twist * z).sin_cos();
            close(actual[0], spec.tip_radius_mm * cos, 1e-12);
            close(actual[1], spec.tip_radius_mm * sin, 1e-12);
            assert_eq!(actual[2], z);
        }
        let topology = gear.topology();
        assert_eq!(topology.faces.len(), 3 + 2 * gear.profile().outline().len());
        for edge in topology.edges {
            for (fraction, vertex) in [0., 1.].into_iter().zip(edge.vertices) {
                for (actual, expected) in edge
                    .curve
                    .point(&gear, fraction)
                    .into_iter()
                    .zip(topology.vertices_mm[vertex as usize])
                {
                    close(actual, expected, 1e-12);
                }
            }
        }
        let mesh = gear.mesh(OPTIONS).unwrap();
        let certificate = certify(&mesh).unwrap();
        close(certificate.signed_volume_mm3, gear.volume_mm3(), 2.);
        close(gear.volume_mm3(), gear.profile().volume_mm3(), 1e-12);
    }
    let (spec, twist) = fixture(24);
    for invalid in [0., f64::NAN, f64::INFINITY, 2., 1e-10] {
        assert_eq!(
            Herringbone::new_helical(spec, invalid, MINIMUM, MAXIMUM),
            Err(Error::InvalidDimensions)
        );
    }
    assert_eq!(
        Herringbone::new_helical(Spec { teeth: 0, ..spec }, twist, MINIMUM, MAXIMUM),
        Err(Error::Limit)
    );
}

#[test]
fn continuous_shell_shares_its_apex_and_keeps_named_native_faces() {
    for teeth in [24, 36] {
        let (spec, twist) = fixture(teeth);
        let gear = Herringbone::new(spec, twist, MINIMUM, MAXIMUM).unwrap();
        let topology = gear.topology();
        let count = gear.profile().outline().len();
        assert_eq!(topology.faces.len(), 3 + 2 * count);
        assert_eq!(topology.edges.len(), 5 * count + 3);
        assert_eq!(topology.vertices_mm.len(), 3 * count + 2);
        let unique = topology
            .faces
            .iter()
            .map(|face| face.role())
            .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), topology.faces.len());
        for band in ["lower", "upper"] {
            assert!(unique.contains(&format!("gear/{band}/tooth/0/involute.left")));
        }
        for (index, edge) in topology.edges.iter().enumerate() {
            for (fraction, vertex) in [0., 1.].into_iter().zip(edge.vertices) {
                let actual = edge.curve.point(&gear, fraction);
                let expected = topology.vertices_mm[vertex as usize];
                for (a, b) in actual.into_iter().zip(expected) {
                    close(a, b, 1e-12);
                }
            }
            if let Curve::Rim {
                ring: Ring::Apex, ..
            } = edge.curve
            {
                assert!(matches!(
                    topology.faces[edge.faces[0] as usize],
                    Face::Tooth {
                        band: Band::Upper,
                        ..
                    }
                ));
                assert!(matches!(
                    topology.faces[edge.faces[1] as usize],
                    Face::Tooth {
                        band: Band::Lower,
                        ..
                    }
                ));
                assert_eq!(edge.curve.point(&gear, 0.5)[2], 2.);
            } else if index == topology.edges.len() - 1 {
                assert_eq!(edge.faces[0], edge.faces[1], "periodic bore seam");
            }
        }
        assert_eq!(gear.phase_at(0.), 0.);
        assert_eq!(gear.phase_at(1.), gear.phase_at(3.));
        assert_eq!(gear.phase_at(2.), 2. * gear.twist_rad_per_mm());
        assert_eq!(gear.phase_at(4.), 0.);
    }
}

#[test]
fn saved_pair_exact_metrics_and_manufacturing_mesh_agree_with_native_readback() {
    for (teeth, native, aabb) in [
        (
            24,
            7_409.916_164_956_88,
            [
                [-26.84645216052, -26.84645216052, -1e-7],
                [26.846452160529, 26.846452160529, 4.0000001],
            ],
        ),
        (
            36,
            17_069.514_940_050_398,
            [
                [22.867880802376, -39.24869002222, -1e-7],
                [101.36526084683, 39.248690022229, 4.0000001],
            ],
        ),
    ] {
        let (spec, twist) = fixture(teeth);
        let gear = Herringbone::new(spec, twist, MINIMUM, MAXIMUM).unwrap();
        close(gear.volume_mm3(), native, 1e-5);
        let [lo, hi] = gear.bounds();
        assert_eq!([lo[2], hi[2]], [0., 4.]);
        close(hi[0], spec.tip_radius_mm, 1e-10);
        let drive = 12_f64.to_radians();
        let (x, phase) = if teeth == 24 {
            (0., drive)
        } else {
            (
                60. / 15_f64.to_radians().cos(),
                PI + PI / 36. - drive * 24. / 36.,
            )
        };
        for (sign, expected) in [-1., 1.].into_iter().zip(aabb) {
            for (axis, expected) in expected.into_iter().enumerate() {
                let d = match axis {
                    0 => rotate([sign, 0.], -phase),
                    1 => rotate([0., sign], -phase),
                    _ => [0.; 2],
                };
                let actual = if axis == 0 { x } else { 0. }
                    + sign * gear.support([d[0], d[1], if axis == 2 { sign } else { 0. }]);
                close(actual, expected, 1.01e-7);
            }
        }

        let mesh = gear.mesh(OPTIONS).unwrap();
        let proof = certify(&mesh).unwrap();
        close(proof.signed_volume_mm3, native, 2.);
        let vertices = mesh.vertices.as_chunks::<3>().0;
        let topology = gear.topology();
        let mut seen = BTreeSet::new();
        for (indices, face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(mesh.face_indices)
        {
            let heights = indices.map(|i| vertices[i as usize][2]);
            let kind = topology.faces[face as usize];
            let first_sample = seen.insert(face);
            match kind {
                Face::Cap { end } => assert_eq!(heights, [if end { 4. } else { 0. }; 3]),
                Face::Tooth { band, edge } => {
                    let [lo, hi] = band.axial_interval_mm(4.);
                    assert!(heights.into_iter().all(|z| (lo..=hi).contains(&z)));
                    if first_sample {
                        let centroid = std::array::from_fn::<_, 3, _>(|axis| {
                            indices
                                .map(|i| vertices[i as usize][axis])
                                .into_iter()
                                .sum::<f64>()
                                / 3.
                        });
                        let projected =
                            rotate([centroid[0], centroid[1]], -gear.phase_at(centroid[2]));
                        let distance = |fraction| {
                            length(super::super::sub(edge.edge.point(fraction), projected))
                        };
                        let (mut start, mut end) = (0., 1.);
                        // Independent transverse distance check for a wall
                        // triangle in every named patch, including fillets.
                        for _ in 0..40 {
                            let a = (2. * start + end) / 3.;
                            let b = (start + 2. * end) / 3.;
                            if distance(a) < distance(b) {
                                end = b;
                            } else {
                                start = a;
                            }
                        }
                        assert!(distance((start + end) * 0.5) <= OPTIONS.deflection);
                    }
                }
                Face::Bore | Face::HubWall { .. } | Face::HubCap { .. } => {}
            }
        }
        assert_eq!(seen.len(), topology.faces.len());
    }
}

#[test]
fn support_encloses_all_dense_axial_sections_in_oblique_directions() {
    for teeth in [24, 36] {
        let (spec, twist) = fixture(teeth);
        for gear in [
            Herringbone::new(spec, twist, MINIMUM, MAXIMUM).unwrap(),
            Herringbone::new_helical(spec, twist, MINIMUM, MAXIMUM).unwrap(),
        ] {
            for [dx, dy, dz] in [
                [1., 0.2, 0.12],
                [0.1, -1., -0.21],
                [-1., 0.9, 0.],
                [0.5, 0.5, 0.6],
            ] {
                let support = gear.support([dx, dy, dz]);
                let intervals = 1000;
                let sampled = (0..=intervals)
                    .map(|i| {
                        let z = i as f64 * spec.face_width_mm / intervals as f64;
                        let [x, y] = rotate([dx, dy], -gear.phase_at(z));
                        gear.profile.support([x, y, 0.]) + dz * z
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                let maximum_sampling_gap = spec.face_width_mm / (2. * intervals as f64)
                    * (length([dx, dy]) * spec.tip_radius_mm * twist.abs() + dz.abs());
                assert!(support + 1e-12 >= sampled);
                assert!(support - sampled <= maximum_sampling_gap + 1e-12);
            }
            assert!(gear.support([0., 0., f64::NAN]).is_nan());
            assert_eq!(gear.support([0.; 3]), 0.);
        }
    }
}

#[test]
fn bounded_model_refuses_degenerate_twist_or_unavailable_mesh_budget() {
    let (spec, twist) = fixture(24);
    for invalid in [0., f64::INFINITY, 2., 1e-10] {
        assert_eq!(
            Herringbone::new(spec, invalid, MINIMUM, MAXIMUM),
            Err(Error::InvalidDimensions)
        );
    }
    let gear = Herringbone::new(
        Spec {
            bore_radius_mm: 0.,
            ..spec
        },
        twist,
        MINIMUM,
        MAXIMUM,
    )
    .unwrap();
    assert_eq!(
        gear.topology().faces.len(),
        2 + 2 * gear.profile().outline().len()
    );
    assert!(certify(&gear.mesh(OPTIONS).unwrap()).is_ok());
    let section = Tessellation {
        deflection: OPTIONS.deflection / 3.,
        angle: OPTIONS.angle / 2.,
        ..OPTIONS
    };
    let count = gear.profile.outline_samples(section, None).unwrap().len();
    assert_eq!(
        gear.mesh(Tessellation {
            triangle_limit: 5 * count,
            ..OPTIONS
        }),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        gear.mesh(Tessellation {
            triangle_limit: 6 * count - 4,
            ..OPTIONS
        }),
        Err(MeshError::BudgetExceeded)
    );

    assert_eq!(
        gear.mesh(Tessellation {
            angle: -1.,
            ..OPTIONS
        }),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        gear.mesh(Tessellation {
            triangle_limit: 50,
            ..OPTIONS
        }),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        gear.mesh(Tessellation {
            deflection: 1e-10,
            ..OPTIONS
        }),
        Err(MeshError::BudgetExceeded)
    );
}
