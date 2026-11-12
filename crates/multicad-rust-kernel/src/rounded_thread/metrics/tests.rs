use super::*;
use crate::rounded_thread::Spec;
fn thread(turns: f64, left_hand: bool) -> Thread {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    Thread::new(Spec {
        major_radius_mm: 3.175,
        pitch_mm: p,
        length_mm: turns * p,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand,
    })
    .unwrap()
}
#[test]
fn fractional_turns_keep_centroid_and_all_mixed_moments_with_hand_reflection() {
    for turns in [1., 2., 3.37] {
        let t = thread(turns, false);
        let r = t.properties().unwrap();
        let l = thread(turns, true).properties().unwrap();
        assert_eq!(r.faces.len(), 7);
        assert!((r.centroid_mm[2] - t.spec().length_mm / 2.).abs() < 1e-13);
        assert!(
            (r.central_second_moment_mm5[2][2] - t.axial_second_moment_mm5() / 4.).abs() < 1e-10
        );
        let shift = t.volume_mm3() * (r.centroid_mm[0].powi(2) + r.centroid_mm[1].powi(2));
        assert!(
            (r.central_second_moment_mm5[0][0] + r.central_second_moment_mm5[1][1] + shift
                - t.polar_second_moment_mm5())
            .abs()
                < 1e-10
        );
        for i in 0..3 {
            let sign = if i == 1 { -1. } else { 1. };
            assert!((r.centroid_mm[i] * sign - l.centroid_mm[i]).abs() < 1e-11);
            for j in 0..3 {
                let sign = if (i == 1) ^ (j == 1) { -1. } else { 1. };
                assert!(
                    (r.central_second_moment_mm5[i][j] * sign - l.central_second_moment_mm5[i][j])
                        .abs()
                        < 1e-10
                );
            }
        }
        if turns.fract() == 0. {
            assert!(r.centroid_mm[0].hypot(r.centroid_mm[1]) < 1e-12);
            assert!(r.central_second_moment_mm5[1][2].abs() > 1e-4);
        } else {
            assert!(r.centroid_mm[0].hypot(r.centroid_mm[1]) > 1e-4);
        }
        for ((a, x), (b, y)) in r.faces.iter().zip(&l.faces) {
            assert!((x - y).abs() < 1e-10);
            assert!((a[0] - b[0]).abs() < 1e-10);
            assert!((a[1] + b[1]).abs() < 1e-10);
        }
    }
}
#[test]
fn independent_polar_volume_quadrature_checks_fractional_centroid_and_tensor() {
    let t = thread(2.37, false);
    let p = t.properties().unwrap();
    let height = t.spec().length_mm;
    // Independently integrate radial volume moments, using uniform angular
    // midpoints and composite Simpson in z, not the profile/Fourier formulas.
    let nt = 4096;
    let nz = 512;
    let dt = std::f64::consts::TAU / nt as f64;
    let dz = height / nz as f64;
    let mut sum = [0_f64; 10];
    for j in 0..=nz {
        let z = j as f64 * dz;
        let weight = if j == 0 || j == nz {
            1.
        } else if j % 2 == 0 {
            2.
        } else {
            4.
        };
        for i in 0..nt {
            let angle = (i as f64 + 0.5) * dt;
            let (si, co) = angle.sin_cos();
            let r = t.radius(angle, z);
            let area = r * r / 2.;
            let m = r.powi(3) / 3.;
            let q = r.powi(4) / 4.;
            let sample = [
                area,
                m * co,
                m * si,
                area * z,
                q * co * co,
                q * si * si,
                q * co * si,
                z * m * co,
                z * m * si,
                area * z * z,
            ];
            for (k, value) in sample.into_iter().enumerate() {
                sum[k] += weight * value * dt * dz / 3.;
            }
        }
    }
    let volume = sum[0];
    let center = [sum[1], sum[2], sum[3]].map(|x| x / volume);
    let raw = [
        [sum[4], sum[6], sum[7]],
        [sum[6], sum[5], sum[8]],
        [sum[7], sum[8], sum[9]],
    ];
    assert!((volume - t.volume_mm3()).abs() < 1e-7);
    for i in 0..3 {
        assert!((center[i] - p.centroid_mm[i]).abs() < 1e-7);
        for j in 0..3 {
            let value = raw[i][j] - volume * center[i] * center[j];
            assert!(
                (value - p.central_second_moment_mm5[i][j]).abs() < 1e-5,
                "{i}/{j}: {value} vs {}",
                p.central_second_moment_mm5[i][j]
            );
        }
    }
}
#[test]
fn invalid_derived_records_cannot_escape_validation_and_source_remains_reusable() {
    let t = thread(2.37, false);
    let p = t.properties().unwrap();
    assert_eq!(
        Properties::checked(vec![], p.centroid_mm, p.central_second_moment_mm5),
        Err(Error::Unresolved)
    );
    let mut faces = p.faces.clone();
    faces[0].1 = -1.;
    assert_eq!(
        Properties::checked(faces, p.centroid_mm, p.central_second_moment_mm5),
        Err(Error::Unresolved)
    );
    let mut center = p.centroid_mm;
    center[0] = f64::NAN;
    assert_eq!(
        Properties::checked(p.faces.clone(), center, p.central_second_moment_mm5),
        Err(Error::Unresolved)
    );
    let mut matrix = p.central_second_moment_mm5;
    matrix[0][0] = -1.;
    assert_eq!(
        Properties::checked(p.faces.clone(), p.centroid_mm, matrix),
        Err(Error::Unresolved)
    );
    assert_eq!(t.properties().unwrap(), p);
}
