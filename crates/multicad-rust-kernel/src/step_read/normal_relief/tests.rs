use super::*;
use crate::{normal_relief::Kind, normal_sweep::Sweep};
const TOL: f64 = 1e-7;
const MIN: f64 = 1e-5;
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
pub(super) fn rectangle(min: Point2, max: Point2) -> Vec<ProfileEdge> {
    let p = [min, [max[0], min[1]], max, [min[0], max[1]]];
    (0..4)
        .map(|i| ProfileEdge::Line {
            a: p[i],
            b: p[(i + 1) % 4],
        })
        .collect()
}
pub(super) fn fixture(kind: Kind) -> Relief {
    let base = Sweep::new(
        vec![
            rectangle([0.; 2], [20., 16.]),
            vec![ProfileEdge::Circle {
                center: [10., 8.],
                radius_mm: 1.,
                clockwise: true,
            }],
        ],
        4.,
        TOL,
        MIN,
    )
    .unwrap();
    let motifs = vec![
        Sweep::new(
            vec![vec![ProfileEdge::Circle {
                center: [3., 3.],
                radius_mm: 0.75,
                clockwise: false,
            }]],
            1.5,
            TOL,
            MIN,
        )
        .unwrap(),
        Sweep::new(
            vec![vec![
                ProfileEdge::Line {
                    a: [12., 2.],
                    b: [14., 2.],
                },
                ProfileEdge::Arc {
                    center: [14., 3.],
                    a: [14., 2.],
                    b: [14., 4.],
                    clockwise: false,
                },
                ProfileEdge::Line {
                    a: [14., 4.],
                    b: [12., 4.],
                },
                ProfileEdge::Line {
                    a: [12., 4.],
                    b: [12., 2.],
                },
            ]],
            0.6,
            TOL,
            MIN,
        )
        .unwrap(),
    ];
    Relief::new(base, motifs, kind, TOL, MIN).unwrap()
}
#[test]
fn full_shared_analytic_relief_roundtrips_without_source_metadata() {
    for kind in [Kind::Raised, Kind::Recessed] {
        let relief = fixture(kind);
        for (origin, u, v) in [
            ([0.; 3], X, Y),
            ([80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
            ([1e6, -2e6, 3e6], X, Y),
        ] {
            let text = crate::step::write_normal_relief(&relief, origin, u, v).unwrap();
            let volume = verify_normal_relief(&text, &relief, origin, u, v, TOL).unwrap();
            assert!((volume - relief.volume_mm3()).abs() < 1e-5);
            let (readback, decoded) = read_import_solid(&text, TOL).unwrap();
            let ImportProfile::NormalRelief {
                relief: decoded_relief,
            } = decoded.profile
            else {
                panic!("expected independently reconstructed relief")
            };
            assert_eq!(decoded_relief.kind(), kind);
            assert_eq!(decoded_relief.motifs().len(), 2);
            assert_eq!(decoded_relief.base().rings().len(), 2);
            assert!((readback.volume_mm3 - relief.volume_mm3()).abs() < 1e-5);
            assert!(
                verify_normal_relief(
                    &text,
                    &decoded_relief,
                    decoded.origin_mm,
                    decoded.u,
                    decoded.v,
                    TOL
                )
                .is_ok()
            );
        }
    }
}

#[test]
fn descriptor_matcher_proves_world_graph_and_refuses_changed_material() {
    let relief = fixture(Kind::Raised);
    let frame = [[80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]];
    let text = crate::step::write_normal_relief(&relief, frame[0], frame[1], frame[2]).unwrap();
    let (_, mut imported) = read_import_solid(&text, TOL).unwrap();
    assert!(normal_relief_import_matches(&imported, &relief, frame, TOL));
    assert!(!normal_relief_import_matches(
        &imported,
        &fixture(Kind::Recessed),
        frame,
        TOL
    ));
    assert!(!normal_relief_import_matches(
        &imported,
        &relief,
        [[80.001, 60., 20.], frame[1], frame[2]],
        TOL
    ));
    assert!(!normal_relief_import_matches(
        &imported,
        &relief,
        frame,
        f64::NAN
    ));
    assert!(!normal_relief_import_matches(
        &imported,
        &relief,
        [[f64::NAN; 3], X, Y],
        TOL
    ));
    imported.u = [0.; 3];
    assert!(!normal_relief_import_matches(
        &imported, &relief, frame, TOL
    ));
    imported.u = frame[1];
    imported.v = frame[2];
    imported.profile = ImportProfile::Cylinder { radius_mm: 3. };
    assert!(!normal_relief_import_matches(
        &imported, &relief, frame, TOL
    ));
}
#[test]
fn full_graph_proof_rejects_wrong_closure_and_has_bounded_recovery() {
    let relief = fixture(Kind::Raised);
    let text = crate::step::write_normal_relief(&relief, [0.; 3], X, Y).unwrap();
    assert!(verify_normal_relief(&text, &fixture(Kind::Recessed), [0.; 3], X, Y, TOL).is_err());
    assert!(verify_normal_relief(&text, &relief, [0.001, 0., 0.], X, Y, TOL).is_err());
    assert!(verify_normal_relief(&text, &relief, [0.; 3], [0.; 3], Y, TOL).is_err());
    let entities = parse_entities(&text, TOL).unwrap();
    let mut shell = Shell::new(&entities, TOL, true);
    let ids = shell.shell_faces().unwrap();
    let faces = shell.validate_faces(ids.clone()).unwrap();
    let readback = shell.readback(&faces).unwrap();
    assert_eq!(
        proof::verify(
            &shell,
            &ids,
            &faces,
            &readback,
            &relief,
            Frame::new([0.; 3], X, Y).unwrap(),
            &mut Work(0)
        ),
        Err(ReadError::Limit)
    );
    assert!(
        proof::verify(
            &shell,
            &ids,
            &faces,
            &readback,
            &relief,
            Frame::new([0.; 3], X, Y).unwrap(),
            &mut Work(MAX_WORK)
        )
        .is_ok()
    );
    assert!(verify_normal_relief(&text, &relief, [0.; 3], X, Y, TOL).is_ok());
}
#[test]
fn header_and_face_declaration_order_do_not_control_physical_proof() {
    let relief = fixture(Kind::Recessed);
    let text = crate::step::write_normal_relief(&relief, [0.; 3], X, Y).unwrap();
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let index = lines
        .iter()
        .position(|line| line.contains("CLOSED_SHELL("))
        .unwrap();
    let line = &lines[index];
    let (prefix, refs) = line.split_once("('',(").unwrap();
    let refs = refs.strip_suffix("));").unwrap();
    let mut refs = refs.split(',').collect::<Vec<_>>();
    refs.reverse();
    lines[index] = format!("{prefix}('',({}));", refs.join(","));
    let text = lines.join("\n");
    assert!(verify_normal_relief(&text, &relief, [0.; 3], X, Y, TOL).is_ok());
    let (_, decoded) = read_import_solid(&text, TOL).unwrap();
    assert!(normal_relief_import_matches(
        &decoded,
        &relief,
        [[0.; 3], X, Y],
        TOL
    ));
}
#[test]
fn all_192_motifs_have_independent_full_graph_proof_and_blind_import() {
    let base = Sweep::new(vec![rectangle([0.; 2], [100., 80.])], 4., TOL, MIN).unwrap();
    let motifs = (0..192)
        .map(|i| {
            Sweep::new(
                vec![vec![ProfileEdge::Circle {
                    center: [3. + (i % 16) as f64 * 6., 3. + (i / 16) as f64 * 6.],
                    radius_mm: 0.5,
                    clockwise: false,
                }]],
                0.5 + (i % 3) as f64 * 0.1,
                TOL,
                MIN,
            )
            .unwrap()
        })
        .collect();
    let relief = Relief::new(base, motifs, Kind::Raised, TOL, MIN).unwrap();
    let started = std::time::Instant::now();
    let text = crate::step::write_normal_relief(&relief, [80., 60., 20.], X, Y).unwrap();
    assert!(verify_normal_relief(&text, &relief, [80., 60., 20.], X, Y, TOL).is_ok());
    let (_, decoded) = read_import_solid(&text, TOL).unwrap();
    assert!(normal_relief_import_matches(
        &decoded,
        &relief,
        [[80., 60., 20.], X, Y],
        TOL
    ));
    eprintln!(
        "192 motif unoptimized writer + full proof + blind import + matcher: {:?}, {} bytes",
        started.elapsed(),
        text.len()
    );
}
#[test]
fn compound_member_order_and_header_names_do_not_change_full_boundary_matching() {
    let relief = fixture(Kind::Raised);
    let mut writer = crate::step::Writer::new();
    for origin in [[0.; 3], [40., 20., 30.]] {
        writer.append_normal_relief(&relief, origin, X, Y).unwrap();
    }
    let text = writer
        .finish()
        .unwrap()
        .replace("MultiCAD", "untrusted foreign label");
    let mut decoded = read_import_compound(&text, TOL).unwrap();
    decoded.reverse();
    for origin in [[0.; 3], [40., 20., 30.]] {
        assert_eq!(
            decoded
                .iter()
                .filter(|(_, body)| normal_relief_import_matches(
                    body,
                    &relief,
                    [origin, X, Y],
                    TOL
                ))
                .count(),
            1
        );
    }
    assert!(verify_normal_relief(&text, &relief, [0.; 3], X, Y, TOL).is_err());
}
#[test]
fn modified_supports_trim_geometry_and_incidence_are_rejected_before_import() {
    let relief = fixture(Kind::Raised);
    let text = crate::step::write_normal_relief(&relief, [0.; 3], X, Y).unwrap();
    let changes = [
        ("CIRCLE('',", "CYLINDRICAL_SURFACE('',"),
        ("CYLINDRICAL_SURFACE('',", "CONICAL_SURFACE('',"),
        ("ORIENTED_EDGE('',*,*,", "ORIENTED_EDGE('',#999999,*,"),
        ("FACE_OUTER_BOUND('',", "VERTEX_LOOP('',"),
    ];
    for (before, after) in changes {
        assert!(text.contains(before));
        let malformed = text.replacen(before, after, 1);
        assert!(verify_normal_relief(&malformed, &relief, [0.; 3], X, Y, TOL).is_err());
        assert!(read_import_solid(&malformed, TOL).is_err());
    }
}
#[test]
fn close_vertices_alone_do_not_certify_skinny_body_support_directions() {
    let base = Sweep::new(vec![rectangle([0.; 2], [10., 0.001])], 0.00002, TOL, 1e-6).unwrap();
    let motif = Sweep::new(
        vec![vec![ProfileEdge::Circle {
            center: [5., 0.0005],
            radius_mm: 0.0001,
            clockwise: false,
        }]],
        0.00001,
        TOL,
        1e-6,
    )
    .unwrap();
    let relief = Relief::new(base, vec![motif], Kind::Raised, TOL, 1e-6).unwrap();
    let angle = 1e-9_f64;
    let frame = [[0.; 3], X, [0., angle.cos(), angle.sin()]];
    let actual = ImportSolid {
        origin_mm: [0.; 3],
        u: X,
        v: Y,
        height_mm: 0.00002,
        profile: ImportProfile::NormalRelief {
            relief: relief.clone(),
        },
    };
    assert!(normal_relief_import_matches(
        &actual,
        &relief,
        [[0.; 3], X, Y],
        TOL
    ));
    assert!(!normal_relief_import_matches(&actual, &relief, frame, TOL));
    let text = crate::step::write_normal_relief(&relief, [0.; 3], X, Y).unwrap();
    let entities = parse_entities(&text, TOL).unwrap();
    let mut shell = Shell::new(&entities, TOL, true);
    let ids = shell.shell_faces().unwrap();
    shell.validate_faces(ids).unwrap();
    let tilted = Frame::new(frame[0], frame[1], frame[2]).unwrap();
    let vertices = proof::vertices(
        &shell,
        relief.boundary(),
        tilted,
        TOL / 16.,
        &mut Work(MAX_WORK),
    )
    .unwrap();
    assert!(
        proof::edges(
            &shell,
            relief.boundary(),
            &vertices,
            tilted,
            TOL / 16.,
            &mut Work(MAX_WORK)
        )
        .is_ok()
    );
    assert!(verify_normal_relief(&text, &relief, frame[0], frame[1], frame[2], TOL).is_err());
}

pub(super) mod external_fixture;
#[test]
fn external_limit_and_recovery_cover_the_same_reader_instance_as_regular_geometry() {
    let base = Sweep::new(vec![rectangle([0.; 2], [100., 80.])], 4., TOL, MIN).unwrap();
    let motif = |center| {
        Sweep::new(
            vec![vec![ProfileEdge::Circle {
                center,
                radius_mm: 0.5,
                clockwise: false,
            }]],
            0.5,
            TOL,
            MIN,
        )
        .unwrap()
    };
    let many = Relief::new(
        base.clone(),
        (0..192)
            .map(|i| motif([3. + (i % 16) as f64 * 6., 3. + (i / 16) as f64 * 6.]))
            .collect(),
        Kind::Raised,
        TOL,
        MIN,
    )
    .unwrap();
    let one = Relief::new(base, vec![motif([3., 75.])], Kind::Raised, TOL, MIN).unwrap();
    let mut writer = crate::step::Writer::new();
    for relief in [&many, &one] {
        writer.append_normal_relief(relief, [0.; 3], X, Y).unwrap();
    }
    let input = external_fixture::external_193_motif_input(&writer.finish().unwrap());
    assert!(read_analytic_solid(&input, TOL).is_ok());
    assert!(matches!(
        read_import_solid(&input, TOL),
        Err(ReadError::Limit)
    ));
    let valid = crate::step::write_normal_relief(&one, [0.; 3], X, Y).unwrap();
    assert!(read_import_solid(&valid, TOL).is_ok());
}
