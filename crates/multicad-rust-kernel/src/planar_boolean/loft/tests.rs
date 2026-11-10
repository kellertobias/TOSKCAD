use super::*;
use miter_sweep::certify_section;

const TOL: f64 = 1e-7;

fn rectangle([x, y]: [f64; 2], [w, h]: [f64; 2]) -> Vec<[f64; 2]> {
    vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

#[test]
fn offset_loft_keeps_authored_source_edges_in_both_windings() {
    let mut sections = [rectangle([0., 0.], [4., 3.]), rectangle([2., 7.], [4., 3.])];
    for reverse in [false, true] {
        if reverse {
            sections.iter_mut().for_each(|ring| ring.reverse());
        }
        let (loft, cells) = ruled_loft(&sections, &[-2., 6.], TOL).unwrap();
        assert_eq!(loft.bounds(), [[0., 0., -2.], [6., 10., 6.]]);
        assert!((loft.volume_mm3() - 96.).abs() < TOL);
        assert_eq!((loft.solid_count(), loft.shell_count()), (1, 1));
        assert_eq!(cells.len(), 1);
        assert_eq!(loft.faces().len(), 6);
        assert_eq!(loft.faces()[0].normal, [0., 0., -1.]);
        assert_eq!(loft.faces()[1].normal, [0., 0., 1.]);
        for (edge, face) in loft.faces()[2..].iter().enumerate() {
            assert_eq!((face.source, face.source_face), (0, 2 + edge));
            let end = (edge + 1) % sections[0].len();
            for (section, z) in sections.iter().zip([-2., 6.]) {
                for [x, y] in [section[edge], section[end]] {
                    assert!(
                        face.vertices
                            .iter()
                            .any(|&id| loft.vertices()[id] == [x, y, z])
                    );
                }
            }
        }
    }
}

#[test]
fn multi_station_taper_has_no_internal_caps_and_keeps_convex_cells() {
    let sections = [
        rectangle([0., 0.], [4., 3.]),
        rectangle([1., 2.], [4., 3.]),
        rectangle([2., 3.], [2., 1.]),
    ];
    let (loft, cells) = ruled_loft(&sections, &[0., 2., 8.], TOL).unwrap();
    assert_eq!(loft.vertices().len(), 12);
    assert_eq!(loft.faces().len(), 10);
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].piece.bounds, [[0., 0., 0.], [5., 5., 2.]]);
    assert_eq!(cells[1].piece.bounds, [[1., 2., 2.], [5., 5., 8.]]);
    let caps: Vec<_> = loft
        .faces()
        .iter()
        .filter(|face| face.source_face < 2)
        .map(|face| (face.source, face.source_face))
        .collect();
    assert_eq!(caps, [(0, 0), (1, 1)]);
    let mesh = loft
        .mesh(Tessellation {
            deflection: 0.1,
            angle: 0.2,
            triangle_limit: 1_000,
        })
        .unwrap();
    let measured = manufacturing::certify(&mesh).unwrap();
    // The first slab is 4×3×2. In the second slab, section area varies
    // quadratically as (4−2t)(3−2t), giving 38 mm³ over 6 mm.
    assert!((loft.volume_mm3() - 62.).abs() < TOL);
    assert!((measured.signed_volume_mm3 - 62.).abs() < TOL);
    let pieces: Vec<_> = cells.into_iter().map(|cell| cell.piece).collect();
    let same = Union::new(&pieces, TOL).unwrap();
    assert_eq!(same.solid_count(), 1);
    assert!((same.volume_mm3() - loft.volume_mm3()).abs() < TOL);
}

#[test]
fn noncardinal_triangular_frustum_and_parallel_roundoff() {
    let base = vec![[0.1, 0.2], [3.1, 1.2], [1.1, 2.2]];
    let top = base
        .iter()
        .map(|&[x, y]| [2. * x + 0.3, 2. * y - 0.4])
        .collect();
    let (loft, _) = ruled_loft(&[base, top], &[0., 3.], TOL).unwrap();
    assert_eq!(loft.faces().len(), 5);
    assert!((loft.volume_mm3() - 17.5).abs() < TOL);
}

#[test]
fn concave_u_loft_preserves_exact_caps_walls_and_boolean_material() {
    let mut sections = [
        vec![
            [0., 0.],
            [10., 0.],
            [10., 8.],
            [7., 8.],
            [7., 2.],
            [3., 2.],
            [3., 8.],
            [0., 8.],
        ],
        vec![
            [1., -1.],
            [12., -1.],
            [12., 10.],
            [9., 10.],
            [9., 3.],
            [4., 3.],
            [4., 10.],
            [1., 10.],
        ],
        Vec::new(),
    ];
    sections[2] = sections[1]
        .iter()
        .map(|&[x, y]| [0.75 * x + 1., 0.75 * y + 1.])
        .collect();
    // Each station interval varies linearly in width and height. Simpson
    // integration of the rectangle minus its notch is exact.
    let expected =
        4. * (56. + 4. * 70.5 + 86.) / 6. + 6. * 86. * (1. + 0.75 + 0.75_f64.powi(2)) / 3.;
    for reversed in [false, true] {
        if reversed {
            sections.iter_mut().for_each(|s| s.reverse());
        }
        let (shell, cells) = ruled_loft(&sections, &[0., 4., 10.], TOL).unwrap();
        assert_eq!(shell.faces().len(), 18);
        assert_eq!((shell.solid_count(), shell.shell_count()), (1, 1));
        for (i, face) in shell.faces().iter().enumerate() {
            let (section, role) = match i {
                0 => (0, 0),
                9 => (1, 1),
                1..=8 => (0, i + 1),
                _ => (1, i - 8),
            };
            assert_eq!((face.source, face.source_face), (section, role));
            assert_eq!(face.vertices.len(), if role < 2 { 8 } else { 4 });
        }
        assert_eq!(cells.iter().filter(|cell| cell.segment == 0).count(), 18);
        assert_eq!(cells.iter().filter(|cell| cell.segment == 1).count(), 6);
        assert!((shell.volume_mm3() - expected).abs() < TOL);
        let mesh = shell
            .mesh(Tessellation {
                deflection: 0.1,
                angle: 0.2,
                triangle_limit: 1_000,
            })
            .unwrap();
        assert!((manufacturing::certify(&mesh).unwrap().signed_volume_mm3 - expected).abs() < TOL);
        let pieces: Vec<_> = cells.into_iter().map(|cell| cell.piece).collect();
        let material = Union::new(&pieces, TOL).unwrap();
        assert_eq!(material.solid_count(), 1);
        assert!((material.volume_mm3() - expected).abs() < TOL);
    }
}

#[test]
fn rejects_oversized_nonmatching_and_unresolved_section_intervals() {
    let rect = rectangle([0., 0.], [4., 3.]);
    let rings = [rect.clone(), rect.clone()];
    let error = |sections: &[Vec<[f64; 2]>], heights: &[f64], tolerance| {
        ruled_loft(sections, heights, tolerance).unwrap_err()
    };
    assert_eq!(error(&rings, &[0., 2.], f64::NAN), Error::Invalid);
    assert_eq!(error(&rings[..1], &[0.], TOL), Error::Invalid);
    assert_eq!(error(&rings, &[0.], TOL), Error::Invalid);
    for invalid in [[1., 0.], [0., TOL], [0., f64::INFINITY], [0., 1e9]] {
        assert_eq!(error(&rings, &invalid, TOL), Error::Invalid);
    }
    assert_eq!(error(&vec![rect.clone(); 17], &[0.; 17], TOL), Error::Limit);
    assert_eq!(
        error(&[vec![[0.; 2]; 127], rect.clone()], &[0., 1.], TOL),
        Error::Limit
    );
    assert_eq!(
        error(&[rect.clone(), rect[..3].to_vec()], &[0., 1.], TOL),
        Error::Invalid
    );
    let short = vec![[0., 0.]; 2];
    assert_eq!(
        error(&[short.clone(), short], &[0., 1.], TOL),
        Error::Invalid
    );
    let mut invalid = rect.clone();
    invalid[0][1] = f64::NAN;
    assert_eq!(error(&[rect, invalid], &[0., 1.], TOL), Error::Invalid);
    let sub_resolution = rectangle([0., 0.], [TOL / 2., 3.]);
    assert_eq!(
        error(&[sub_resolution.clone(), sub_resolution], &[0., 1.], TOL),
        Error::Invalid
    );
}

#[test]
fn rejects_crossed_and_nonplanar_ruled_faces() {
    let base = rectangle([0., 0.], [4., 3.]);
    let mut mismatches = [base.clone(), base.clone(), base.clone()];
    mismatches[0].rotate_left(2);
    mismatches[1][1][1] += 1e-4;
    mismatches[2].reverse();
    for top in mismatches {
        assert_eq!(
            ruled_loft(&[base.clone(), top], &[0., 2.], 1e-3).unwrap_err(),
            Error::Invalid
        );
    }
    let mut crossed = base.clone();
    crossed.swap(1, 2);
    assert_eq!(
        ruled_loft(&[crossed.clone(), crossed], &[0., 2.], TOL).unwrap_err(),
        Error::Invalid
    );
}

#[test]
fn rejects_two_simple_profiles_whose_side_walls_cross_between_stations() {
    let opposed_notches = |right: [f64; 2], left: [f64; 2]| {
        vec![
            [0., 0.],
            [12., 0.],
            [12., right[0]],
            [4., right[0]],
            [4., right[1]],
            [12., right[1]],
            [12., 12.],
            [0., 12.],
            [0., left[1]],
            [8., left[1]],
            [8., left[0]],
            [0., left[0]],
        ]
    };
    let a = opposed_notches([2., 4.], [8., 10.]);
    let b = opposed_notches([8., 10.], [2., 4.]);
    for profile in [&a, &b] {
        certify_section(profile, false, TOL).unwrap();
    }
    assert_eq!(
        ruled_loft(&[a, b], &[0., 4.], TOL).unwrap_err(),
        Error::Invalid
    );
}

#[test]
fn a_second_station_can_supply_the_common_loft_partition() {
    let profile = |[a, b, c, d, e, h, p, q]: [f64; 8]| {
        let [u, v, w, x, width] = [a, a + b, a + b + c, a + b + c + d, a + b + c + d + e];
        let y = h + p.max(q);
        vec![
            [0., 0.],
            [width, 0.],
            [width, y],
            [x, y],
            [x, p],
            [w, p],
            [w, y],
            [v, y],
            [v, q],
            [u, q],
            [u, y],
            [0., y],
        ]
    };
    let sections = [
        profile([8., 9., 12., 21., 28., 3., 25., 12.]),
        profile([21., 26., 20., 4., 21., 24., 18., 20.]),
    ];
    let (shell, cells) = ruled_loft(&sections, &[0., 10.], TOL).unwrap();
    let volume = 10. * (1977. + 4. * 2528.75 + 3320.) / 6.;
    assert!((shell.volume_mm3() - volume).abs() < TOL);
    assert_eq!(shell.faces().len(), 14);
    assert_eq!(shell.solid_count(), 1);
    let pieces: Vec<_> = cells.into_iter().map(|cell| cell.piece).collect();
    assert!((Union::new(&pieces, TOL).unwrap().volume_mm3() - volume).abs() < TOL);
}

#[test]
fn loft_reports_the_material_limit_after_trying_the_station_partitions() {
    let a = vec![
        [0., 0.],
        [10., 0.],
        [10., 8.],
        [7., 8.],
        [7., 2.],
        [3., 2.],
        [3., 8.],
        [0., 8.],
    ];
    let b = vec![
        [1., -1.],
        [12., -1.],
        [12., 10.],
        [9., 10.],
        [9., 3.],
        [4., 3.],
        [4., 10.],
        [1., 10.],
    ];
    let stations: Vec<_> = [a, b].into_iter().cycle().take(16).collect();
    let heights: Vec<_> = (0..16).map(|i| f64::from(i) * 4.).collect();
    assert_eq!(
        ruled_loft(&stations, &heights, TOL).unwrap_err(),
        Error::Limit
    );
}
