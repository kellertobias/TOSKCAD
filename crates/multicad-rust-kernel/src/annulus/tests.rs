use super::*;
use crate::manufacturing::certify;
use std::collections::BTreeMap;

#[test]
fn exact_material_volume_and_support_of_finite_ring() {
    let ring = Annulus::new(3., 5., 2.75).unwrap();
    assert_eq!(ring.inner_radius_mm(), 3.);
    assert_eq!(ring.outer_radius_mm(), 5.);
    assert_eq!(ring.height_mm(), 2.75);
    assert!((ring.cross_section_area_mm2() - 16. * PI).abs() < 1e-14);
    assert!((ring.volume_mm3() - 44. * PI).abs() < 1e-13);
    assert!((ring.support([-3., 4., -12.]) - 25.).abs() < 1e-12);
    assert!((ring.support([-3., 4., 12.]) - 58.).abs() < 1e-12);
    assert_eq!(ring.support([0., 0., 0.]), 0.);
    assert!(ring.support([f64::NAN, 0., 0.]).is_nan());

    for point in [[3., 0., 0.], [0., 5., 2.75], [2.4, 3.2, 1.2]] {
        assert!(ring.contains(point, 0.));
    }
    for point in [
        [0., 0., 1.],
        [2.99, 0., 1.],
        [5.01, 0., 1.],
        [4., 0., -0.01],
        [4., 0., 2.76],
    ] {
        assert!(!ring.contains(point, 0.));
    }
    assert!(ring.contains([2.99, 0., -0.01], 0.02));
    assert!(!ring.contains([4., 0., 1.], -0.01));
    assert!(!ring.contains([4., 0., 1.], f64::INFINITY));
    assert!(!ring.contains([4., 0., f64::NAN], 0.));
}

#[test]
fn invalid_dimensions_and_unrepresentable_ring_fail_closed() {
    for [inner, outer, height] in [
        [0., 5., 1.],
        [6., 5., 1.],
        [5., 5., 1.],
        [3., 5., 0.],
        [3., 5., f64::INFINITY],
        [f64::NAN, 5., 1.],
        [3., f64::NAN, 1.],
    ] {
        assert_eq!(
            Annulus::new(inner, outer, height),
            Err(Error::InvalidLength)
        );
    }
    for [inner, outer, height] in [
        [3., 1e8, 1.],
        [3., 5., 1e8],
        [3., 3_f64.next_up(), 1.],
        [1e-200, 3e-200, 1.],
    ] {
        assert_eq!(Annulus::new(inner, outer, height), Err(Error::OutOfRange));
    }
}

#[test]
fn shared_rims_form_one_closed_shell_with_correct_face_winding_and_chords() {
    let ring = Annulus::new(4.975, 5., 2.75).unwrap();
    let options = Tessellation {
        deflection: 0.003,
        angle: 0.24,
        triangle_limit: 10_000,
    };
    let mesh = tessellate(&ring, options).unwrap();
    let (vertices, _) = mesh.vertices.as_chunks::<3>();
    let (triangles, _) = mesh.indices.as_chunks::<3>();
    let sectors = vertices.len() / 4;
    assert_eq!(vertices.len(), 4 * sectors);
    assert_eq!(triangles.len(), 8 * sectors);

    let mut edges = BTreeMap::new();
    let mut faces = [0; 4];
    let subtract = |a: [f64; 3], b: [f64; 3]| std::array::from_fn(|i| a[i] - b[i]);
    let mut independent_volume = 0.;
    for (&triangle, &face) in triangles.iter().zip(&mesh.face_indices) {
        faces[face as usize] += 1;
        for [a, b] in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            let (count, direction) = edges.entry((a.min(b), a.max(b))).or_insert((0, 0));
            *count += 1;
            *direction += if a < b { 1 } else { -1 };
        }
        let [a, b, c] = triangle.map(|index| vertices[index as usize]);
        let [ux, uy, uz] = subtract(b, a);
        let [vx, vy, vz] = subtract(c, a);
        let normal = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
        independent_volume += normal.into_iter().zip(a).map(|(n, p)| n * p).sum::<f64>() / 6.;
        match face {
            0 => assert!(normal[2] < 0. && [a[2], b[2], c[2]] == [0.; 3]),
            1 => assert!(normal[2] > 0. && [a[2], b[2], c[2]] == [2.75; 3]),
            2 | 3 => {
                assert_eq!(normal[2], 0.);
                let outward = normal[0] * a[0] + normal[1] * a[1];
                let radius = if face == 2 {
                    assert!(outward > 0.);
                    ring.outer_radius_mm()
                } else {
                    assert!(outward < 0.);
                    ring.inner_radius_mm()
                };
                assert!((a[0].hypot(a[1]) - radius).abs() < 1e-12);
                let opposite = if a[..2] == b[..2] { c } else { b };
                let chord_midpoint = [a[0].midpoint(opposite[0]), a[1].midpoint(opposite[1])];
                assert!(
                    radius - chord_midpoint[0].hypot(chord_midpoint[1])
                        <= options.deflection + 1e-12
                );
                let angular_step = (a[0] * opposite[0] + a[1] * opposite[1]) / radius.powi(2);
                assert!(angular_step.acos() <= options.angle + 1e-12);
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(faces, [2 * sectors; 4]);
    assert!(edges.values().all(|&incidence| incidence == (2, 0)));
    assert_eq!(vertices.len() + triangles.len(), edges.len());
    let proof = certify(&mesh).unwrap();
    assert!((proof.signed_volume_mm3 - independent_volume).abs() < 1e-11);
    assert!(independent_volume > 0.);
    assert!(independent_volume < ring.volume_mm3());
    assert!(ring.volume_mm3() - independent_volume <= volume_error_bound(&ring, options));
}

#[test]
fn triangle_budget_and_invalid_tessellation_options_are_reported() {
    let ring = Annulus::new(3., 5., 2.75).unwrap();
    let coarse = Tessellation {
        deflection: 5.,
        angle: 1.,
        triangle_limit: 56,
    };
    assert_eq!(tessellate(&ring, coarse).unwrap().face_indices.len(), 56);
    assert_eq!(
        tessellate(
            &ring,
            Tessellation {
                triangle_limit: 55,
                ..coarse
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        tessellate(
            &ring,
            Tessellation {
                deflection: 0.,
                ..coarse
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        tessellate(
            &ring,
            Tessellation {
                angle: 0.005,
                ..coarse
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        tessellate(
            &ring,
            Tessellation {
                triangle_limit: 0,
                ..coarse
            }
        ),
        Err(MeshError::InvalidOptions)
    );
}
