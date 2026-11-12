use super::*;
use crate::{
    Circle, Rational, chamfered_bores::ChamferedBores, polygon::PolygonPrism, step::Writer,
};
fn fixture(cuts: &[Counterbore], reverse: bool) -> String {
    let q = |n| Rational::new(n, 1).unwrap();
    let p = PolygonPrism::with_circular_holes(
        vec![
            vec![[0, 0], [20, 0], [20, 16], [0, 16]]
                .into_iter()
                .map(|p| p.map(q))
                .collect(),
        ],
        vec![
            Circle::new([q(5), q(8)], q(1)).unwrap(),
            Circle::new([q(14), q(8)], q(1)).unwrap(),
        ],
        q(6),
    )
    .unwrap();
    let b = ChamferedBores::counterbored(&p, cuts, 1e-7, 1e-5).unwrap();
    let mut w = Writer::new();
    w.append_treated_polygon_prism(
        &p,
        &b,
        [40., -8., 12.],
        [0.8, 0.6, 0.],
        [-0.36, 0.48, 0.8],
        reverse,
    )
    .unwrap();
    w.finish().unwrap()
}

#[test]
fn complete_chain_recognition_refuses_untrusted_missing_extra_and_displaced_bounds() {
    let source = fixture(&[cut(false, 3., 1.), cut(true, 2., 1.5)], false);
    for mode in 0..9 {
        let entities = parse_entities(&source, 1e-7).unwrap();
        let mut shell = Shell::new(&entities, 1e-7, true);
        let mut faces = shell.validated_faces().unwrap();
        let mut edges = shell.indexed_edges(faces.len());
        let lower = (0..faces.len())
            .find(|&i| {
                if let Face::Planar { origin, normal, .. } = faces[i] {
                    let direction = normal.map(|n| -n);
                    preferred_axis(direction)
                        && shell
                            .match_counterbored_polygon(&faces, &edges, i, origin, direction)
                            .is_some()
                } else {
                    false
                }
            })
            .unwrap();
        let Face::Planar { origin, normal, .. } = faces[lower] else {
            unreachable!()
        };
        let direction = normal.map(|n| -n);
        let cap = shell
            .polygon_caps(&faces, &edges, lower, origin, direction)
            .unwrap();
        let (rim, mouth) = edges[lower]
            .iter()
            .find_map(|&id| match shell.edges[&id].curve {
                Curve::Circle(c) if c.radius_mm > 1.5 => Some((id, c)),
                _ => None,
            })
            .unwrap();
        let wall = shell.adjacent_face(rim, lower).unwrap();
        let far = *edges[wall]
            .iter()
            .find(|&&id| id != rim && matches!(shell.edges[&id].curve, Curve::Circle(_)))
            .unwrap();
        let shoulder = shell.adjacent_face(far, wall).unwrap();
        let shoulder_far = *edges[shoulder].iter().find(|&&id| id != far).unwrap();
        let mut caps = [lower, cap.upper];
        let mut height = cap.height;
        let mut covered = BTreeSet::new();
        match mode {
            0 => {
                covered.insert(wall);
            }
            1 => edges[wall].retain(|&id| id != far),
            2 => {
                let other = *edges[lower]
                    .iter()
                    .find(|&&id| id != rim && matches!(shell.edges[&id].curve, Curve::Circle(_)))
                    .unwrap();
                edges[wall].push(other);
            }
            3 => {
                faces[wall] = Face::Cylindrical { seam: 0, flux: 1. };
                assert!(shell.counterbore_source_keys(&faces, &edges).is_none());
            }
            4 => faces[shoulder] = Face::Cylindrical { seam: 0, flux: -1. },
            5 => {
                let Face::Planar { normal, .. } = &mut faces[shoulder] else {
                    unreachable!()
                };
                *normal = normal.map(|n| -n);
            }
            6 => {
                let Curve::Circle(c) = &mut shell.edges.get_mut(&shoulder_far).unwrap().curve
                else {
                    unreachable!()
                };
                for (p, n) in c.center_mm.iter_mut().zip(direction) {
                    *p += n * 1e-11;
                }
            }
            7 => caps[1] = shell.adjacent_face(shoulder_far, shoulder).unwrap(),
            8 => height += 0.1,
            _ => unreachable!(),
        }
        if (1..=6).contains(&mode) {
            assert!(
                preferred_axis(normal.map(|n| -n))
                    && shell
                        .match_counterbored_polygon(&faces, &edges, lower, origin, direction)
                        .is_none()
            );
        }
        assert!(
            shell
                .counterbored_bore(
                    &faces,
                    &edges,
                    caps,
                    (rim, mouth),
                    direction,
                    height,
                    &mut covered
                )
                .is_none(),
            "case {mode}"
        );
    }
}
fn cut(end: bool, radius_mm: f64, depth_mm: f64) -> Counterbore {
    Counterbore {
        hole: 0,
        end,
        radius_mm,
        depth_mm,
    }
}

#[test]
fn genuine_multistep_revolved_bores_are_not_mistaken_for_single_level_seats() {
    for ring in [
        vec![
            [1., 0.],
            [10., 0.],
            [10., 4.],
            [4., 4.],
            [4., 3.],
            [3., 3.],
            [3., 2.],
            [2., 2.],
            [2., 1.],
            [1., 1.],
        ],
        vec![
            [3., 0.],
            [10., 0.],
            [10., 3.],
            [1., 3.],
            [1., 2.],
            [2., 2.],
            [2., 1.],
            [3., 1.],
        ],
    ] {
        let r = crate::revolution::Revolution::new(vec![ring], std::f64::consts::TAU).unwrap();
        let source =
            crate::step::write_cylindrical_revolution(&r, [0.; 3], [1., 0., 0.], [0., 1., 0.])
                .unwrap();
        let entities = parse_entities(&source, 1e-7).unwrap();
        let mut shell = Shell::new(&entities, 1e-7, true);
        let faces = shell.validated_faces().unwrap();
        let edges = shell.indexed_edges(faces.len());
        let lower = faces
            .iter()
            .position(|f| matches!(f,Face::Planar {normal,..} if normal[2]< -0.9))
            .unwrap();
        let upper = faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| match f {
                Face::Planar { origin, normal, .. } if normal[2] > 0.9 => Some((i, origin[2])),
                _ => None,
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let (rim, mouth) = edges[lower]
            .iter()
            .filter_map(|&id| match shell.edges[&id].curve {
                Curve::Circle(c) => Some((id, c)),
                _ => None,
            })
            .min_by(|a, b| a.1.radius_mm.total_cmp(&b.1.radius_mm))
            .unwrap();
        assert!(
            shell
                .counterbored_bore(
                    &faces,
                    &edges,
                    [lower, upper.0],
                    (rim, mouth),
                    [0., 0., 1.],
                    upper.1,
                    &mut BTreeSet::new()
                )
                .is_none()
        );
    }
}

#[test]
fn extra_faces_unsupported_curves_and_duplicate_profile_edges_never_disappear() {
    let source = fixture(&[cut(false, 3., 1.)], false);
    let entities = parse_entities(&source, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    let mut faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let lower = (0..faces.len())
        .find(|&i| {
            if let Face::Planar { origin, normal, .. } = faces[i] {
                preferred_axis(normal.map(|n| -n))
                    && shell
                        .match_counterbored_polygon(&faces, &edges, i, origin, normal.map(|n| -n))
                        .is_some()
            } else {
                false
            }
        })
        .unwrap();
    let Face::Planar { origin, normal, .. } = faces[lower] else {
        unreachable!()
    };
    let direction = normal.map(|n| -n);
    faces.push(Face::Planar {
        origin: [0.; 3],
        normal: [0., 0., 1.],
        area: 1.,
    });
    assert!(
        shell
            .match_counterbored_polygon(&faces, &edges, lower, origin, direction)
            .is_none()
    );
    faces.pop();
    let edge = *edges[lower]
        .iter()
        .find(|&&id| matches!(shell.edges[&id].curve, Curve::Line))
        .unwrap();
    let mut duplicated = edges[lower].clone();
    duplicated.push(edge);
    let keys = shell.counterbore_source_keys(&faces, &edges).unwrap();
    let (_, body) = read_import_solid(&source, 1e-7).unwrap();
    let ImportProfile::CounterboredPolygon { rings, .. } = &body.profile else {
        unreachable!()
    };
    let project = |p| {
        let d = sub(p, body.origin_mm);
        [dot(d, body.u), dot(d, body.v)]
    };
    let caps = shell
        .polygon_caps(&faces, &edges, lower, origin, direction)
        .unwrap();
    let mut covered = caps.covered;
    let bores = edges[lower]
        .iter()
        .filter_map(|&id| match shell.edges[&id].curve {
            Curve::Circle(c) => Some(
                shell
                    .counterbored_bore(
                        &faces,
                        &edges,
                        [lower, caps.upper],
                        (id, c),
                        direction,
                        caps.height,
                        &mut covered,
                    )
                    .unwrap(),
            ),
            _ => None,
        })
        .collect::<Vec<_>>();
    let (_, cuts, mapped) = shell
        .counterbore_profile(
            rings,
            &bores,
            project,
            lower,
            &edges[lower],
            &keys,
            [lower, caps.upper],
        )
        .unwrap();
    assert_eq!(cuts.len(), 1);
    assert_eq!(mapped.len(), keys.len());
    assert!(
        shell
            .counterbore_profile(&[], &[], project, lower, &duplicated, &keys, [lower, lower])
            .is_none()
    );
    shell.edges.get_mut(&edge).unwrap().curve = Curve::Semicircle(SemicircleEdge {
        center_mm: [0.; 3],
        normal: [0., 0., 1.],
        radius_mm: 1.,
        radial_mid: [1., 0., 0.],
    });
    assert!(shell.counterbore_source_keys(&faces, &edges).is_none());
}

pub(crate) fn reverse_shell_faces(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            if !line.contains("CLOSED_SHELL") {
                return line.to_string();
            }
            let start = line.find(",(").unwrap() + 2;
            let end = line.rfind("))").unwrap();
            let faces = line[start..end]
                .split(',')
                .rev()
                .collect::<Vec<_>>()
                .join(",");
            format!("{}{faces}{}", &line[..start], &line[end..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn shell_face_order_does_not_change_any_complete_source_face_key() {
    let source = fixture(&[cut(false, 3., 1.), cut(true, 2., 1.5)], false);
    let (_, a) = read_import_solid(&source, 1e-7).unwrap();
    let (_, b) = read_import_solid(&reverse_shell_faces(&source), 1e-7).unwrap();
    assert_eq!(a, b);
}
#[test]
fn blind_import_retains_every_seat_and_plain_bore_in_both_sweep_directions() {
    for cuts in [
        vec![cut(false, 3., 1.)],
        vec![cut(true, 2., 1.5)],
        vec![cut(false, 3., 1.), cut(true, 2., 1.5)],
    ] {
        for reverse in [false, true] {
            let source = fixture(&cuts, reverse);
            let (read, body) = read_import_solid(&source, 1e-7).unwrap();
            let ImportProfile::CounterboredPolygon {
                rings,
                circular_holes,
                counterbores,
                source_faces,
            } = body.profile
            else {
                panic!("wrong profile");
            };
            assert_eq!(source_faces.len(), 8 + 2 * cuts.len());
            assert_eq!(rings.len(), 1);
            assert_eq!(circular_holes.len(), 2);
            assert!(circular_holes.iter().all(|h| h.1 == 1.));
            let mut expected: Vec<_> = cuts.iter().map(|c| (c.radius_mm, c.depth_mm)).collect();
            let mut actual: Vec<_> = counterbores
                .iter()
                .map(|c| (c.radius_mm, c.depth_mm))
                .collect();
            expected.sort_by(|a, b| a.0.total_cmp(&b.0));
            actual.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (a, b) in actual.iter().zip(&expected) {
                assert!((a.0 - b.0).abs() < 1e-12);
                assert!((a.1 - b.1).abs() < 1e-12);
            }
            assert_eq!(actual.len(), expected.len());
            let loss: f64 = cuts
                .iter()
                .map(|c| std::f64::consts::PI * (c.radius_mm * c.radius_mm - 1.) * c.depth_mm)
                .sum();
            assert!((read.volume_mm3 - (1920. - 12. * std::f64::consts::PI - loss)).abs() < 1e-8);
        }
    }
}
