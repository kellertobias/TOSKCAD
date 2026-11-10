use super::*;
use crate::step::{Writer, write_warped_union};

const TOL: f64 = 1e-7;
const ORIGIN: Point = [12., -43., 19.];
const U: Point = [0.6, 0.8, 0.];
const V: Point = [-0.48, 0.36, 0.8];

fn line(a: [f64; 2], b: [f64; 2]) -> Edge {
    Edge::Line { a, b }
}

fn ring(points: [[f64; 2]; 4]) -> Vec<(Edge, usize)> {
    (0..4)
        .map(|i| (line(points[i], points[(i + 1) % 4]), i))
        .collect()
}

fn section(leg: f64) -> Vec<(Edge, usize)> {
    let arc = |a, b, clockwise| Edge::Arc {
        center: [0., -1.5],
        a,
        b,
        clockwise,
    };
    [
        arc([0., 0.], [1.5, -1.5], true),
        line([1.5, -1.5], [1.5, -1.5 - leg]),
        line([1.5, -1.5 - leg], [3., -1.5 - leg]),
        line([3., -1.5 - leg], [3., -1.5]),
        arc([3., -1.5], [0., 1.5], false),
        line([0., 1.5], [0., 0.]),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, edge)| (edge, i + 20))
    .collect()
}

fn fixture(up: f64, count: usize, cutouts: bool) -> warped_union::Shell {
    let corners = [
        [0., 0.],
        [12., 0.],
        [12., 4.],
        [14., 4.],
        [14., 0.],
        [226., 0.],
        [226., 4.],
        [228., 4.],
        [228., 0.],
        [240., 0.],
        [240., 160.],
        [228., 160.],
        [228., 156.],
        [226., 156.],
        [226., 160.],
        [14., 160.],
        [14., 156.],
        [12., 156.],
        [12., 160.],
        [0., 160.],
    ];
    let web = (0..corners.len())
        .map(|i| SheetWebEdge {
            ends: [corners[i], corners[(i + 1) % corners.len()]],
            source: i,
            flange: match i {
                4 => Some(0),
                14 if count == 2 => Some(1),
                _ => None,
            },
        })
        .collect::<Vec<_>>();
    let sections = [section(50.), section(30.)];
    let flanges = std::array::from_fn::<_, 2, _>(|i| SheetFlange {
        section: &sections[i],
        ends: [30 + i * 2, 31 + i * 2],
    });
    let holes = if cutouts {
        vec![
            ring([[100., 59.], [100., 81.], [140., 81.], [140., 59.]]),
            vec![(
                Edge::Circle {
                    center: [23., 90.],
                    radius_mm: 4.,
                    clockwise: true,
                },
                42,
            )],
        ]
    } else {
        vec![]
    };
    let rect = ring([[-21., -4.], [21., -4.], [21., 4.], [-21., 4.]]);
    let circle = [(
        Edge::Circle {
            center: [0.; 2],
            radius_mm: 2.1,
            clockwise: false,
        },
        43,
    )];
    let aperture = |center: f64, outline| SheetFlangeOpening {
        walls: [1, 3],
        origin_mm: [center, -1.5, -15. * up],
        u: [1., 0., 0.],
        v: [0., 0., up],
        through_mm: [0., -1.5, 0.],
        outline,
    };
    let cuts = [
        aperture(106., &rect),
        aperture(44., &circle),
        aperture(196., &circle),
    ];
    let lists: [&[_]; 2] = [&cuts, &[]];
    formed_sheet_with_openings(
        &web,
        &holes,
        [40, 41],
        &flanges[..count],
        if cutouts { &lists[..count] } else { &[] },
        1.5 * up,
        TOL,
        2. * TOL,
    )
    .unwrap()
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn inset_formed_sheet_replays_closed_rectangular_and_circular_tunnels() {
    for (up, count, cutouts, origin, u, v) in [
        (1., 1, false, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
        (1., 2, false, ORIGIN, U, V),
        (1., 2, true, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
        (-1., 2, true, ORIGIN, U, V),
    ] {
        let expected = fixture(up, count, cutouts);
        let source = write_warped_union(&expected, origin, u, v).unwrap();
        let imported = read_warped_solid(&source, TOL)
            .unwrap_or_else(|error| panic!("{up}, {count}, {cutouts}: {error:?}"));
        let area = |shell: &warped_union::Shell| shell.faces().iter().map(|f| f.area_mm2).sum();
        assert_eq!(imported.shell.faces().len(), expected.faces().len());
        near(imported.shell.volume_mm3(), expected.volume_mm3());
        near(area(&imported.shell), area(&expected));
        let mut writer = Writer::new();
        writer
            .append_warped_union(&imported.shell, imported.origin_mm, imported.u, imported.v)
            .unwrap();
        let reexport = writer.finish().unwrap();
        let decoded = read_analytic_compound(&source, TOL).unwrap();
        let reopened = read_analytic_compound(&reexport, TOL).unwrap();
        for (a, b) in decoded[0]
            .bounds_mm
            .into_iter()
            .flatten()
            .zip(reopened[0].bounds_mm.into_iter().flatten())
        {
            near(a, b);
        }
        near(decoded[0].volume_mm3, reopened[0].volume_mm3);
    }
}

#[test]
fn mixed_compound_retains_the_formed_sheet_as_one_closed_member() {
    use crate::{Rational, prism::Prism};
    let sheet = fixture(-1., 2, true);
    let stock = Prism::rectangular(1, [3, 4, 5].map(|n| Rational::new(n, 1).unwrap())).unwrap();
    let mut writer = Writer::new();
    writer.append_warped_union(&sheet, ORIGIN, U, V).unwrap();
    writer
        .append_rectangular_prism(&stock, [500.; 3], [1., 0., 0.], [0., 1., 0.])
        .unwrap();
    let source = writer.finish().unwrap();
    let members = read_import_compound(&source, TOL).unwrap();
    let [(sheet_proof, imported), (box_proof, _)] = members.as_slice() else {
        panic!("expected two independently certified solids");
    };
    let ImportProfile::WarpedUnion { shell } = &imported.profile else {
        panic!("expected reconstructed inset shell");
    };
    near(shell.volume_mm3(), sheet.volume_mm3());
    near(sheet_proof.volume_mm3, sheet.volume_mm3());
    near(box_proof.volume_mm3, 60.);
}

#[test]
fn replay_refuses_changes_to_trim_ownership_and_oriented_support() {
    let sheet = fixture(1., 2, true);
    let source = write_warped_union(&sheet, ORIGIN, U, V).unwrap();

    // Keep all directed EDGE_CURVEs and bounds intact while inverting
    // only the surface orientation of an aperture wall.
    let planar_wall = sheet
        .faces()
        .iter()
        .position(|face| {
            matches!(face.support, warped_union::Support::Plane { .. })
                && face.point_mm[1] == -1.5
                && (85. ..=127.).contains(&face.point_mm[0])
                && (-19. ..=-11.).contains(&face.point_mm[2])
        })
        .unwrap();
    let face = source
        .lines()
        .filter(|line| line.contains("=ADVANCED_FACE("))
        .nth(planar_wall)
        .unwrap();
    assert!(face.ends_with(",.T.);"));
    let wrong_normal = source.replacen(face, &face.replace(",.T.);", ",.F.);"), 1);
    assert!(read_import_solid(&wrong_normal, TOL).is_err());

    let mut holes = source
        .lines()
        .filter_map(|line| line.split_once("=FACE_BOUND("));
    let (first, first_bound) = holes.next().unwrap();
    let (second, second_bound) = holes.next().unwrap();
    let wrong_loop = source.replacen(
        &format!("{first}=FACE_BOUND({first_bound}"),
        &format!("{first}=FACE_BOUND({second_bound}"),
        1,
    );
    assert_ne!(first, second);
    assert!(read_import_solid(&wrong_loop, TOL).is_err());
}

fn inspect(check: impl FnOnce(&mut Shell<'_, '_>, Boundary)) {
    let source =
        write_warped_union(&fixture(1., 2, true), [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let entities = parse_entities(&source, TOL).unwrap();
    let mut reader = Shell::new(&entities, TOL, true);
    let id = reader.represented_solids(true).unwrap()[0];
    let boundary = Boundary::parse(&mut reader, id).unwrap();
    check(&mut reader, boundary);
}

const UP: Point = [0., 0., 1.];
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];

fn web_faces(boundary: &Boundary) -> [usize; 2] {
    [-1., 1.].map(|sign| {
        boundary
            .faces
            .iter()
            .position(|face| {
                face.loop_.len() == 20
                    && matches!(face.surface, Surface::Plane { normal, .. } if normal[2] == sign)
            })
            .unwrap()
    })
}

#[test]
fn refuses_nonplanar_curved_or_unbounded_sheet_recipes() {
    inspect(|reader, boundary| {
        let wire = Wire {
            reader,
            boundary: &boundary,
        };
        let incidence = Incidence::new(&boundary);
        let (wall, mut bend) = (0..boundary.faces.len())
            .find_map(|i| wire.bend(i, &incidence).unwrap().map(|bend| (i, bend)))
            .unwrap();
        let [lower, upper] = web_faces(&boundary);
        let arc = *boundary.faces[wall]
            .loop_
            .iter()
            .find(|&&(edge, _)| matches!(boundary.curves[&edge].1, TrimCurve::Arc(_)))
            .unwrap();
        let [a, b] = wire.points(arc);
        assert_eq!(wire.line(arc), Err(ReadError::Unsupported));
        assert_eq!(
            wire.outline(arc, [[0., 0., 100.], X, Y], std::convert::identity),
            Err(ReadError::Invalid)
        );
        assert_eq!(
            wire.outline(
                arc,
                [a, unit(sub(b, a)).unwrap(), X],
                std::convert::identity
            ),
            Err(ReadError::Unsupported),
        );
        assert_eq!(wire.normal(wall), Err(ReadError::Unsupported));

        let rail = boundary.faces[wall]
            .loop_
            .iter()
            .position(|&(id, _)| matches!(boundary.curves[&id].1, TrimCurve::Line))
            .unwrap();
        let mut bad = boundary.clone();
        bad.faces[wall].loop_.rotate_left(rail);
        let Surface::Cylinder { axis, .. } = &mut bad.faces[wall].surface else {
            unreachable!()
        };
        *axis = UP;
        assert!(
            Wire {
                reader,
                boundary: &bad
            }
            .bend(wall, &incidence)
            .unwrap()
            .is_none()
        );

        bad = boundary.clone();
        let rails = bad.faces[wall]
            .loop_
            .iter()
            .copied()
            .filter(|&(id, _)| matches!(boundary.curves[&id].1, TrimCurve::Line))
            .collect::<Vec<_>>();
        bad.faces[wall].loop_ = rails.repeat(2);
        assert!(
            Wire {
                reader,
                boundary: &bad
            }
            .bend(wall, &incidence)
            .unwrap()
            .is_none()
        );

        let hinge = reverse(&boundary.faces[lower].loop_)
            .find(|usage| bend.rails.contains(&(usage.0, lower)))
            .unwrap();
        let points = wire.points(hinge);
        let (_, caps) = flange_section(&wire, &incidence, &bend, points, UP, 1.5).unwrap();
        bad = boundary.clone();
        let cap = &mut bad.faces[caps[0]];
        cap.holes.push(cap.loop_.clone());
        assert_eq!(
            flange_section(
                &Wire {
                    reader,
                    boundary: &bad
                },
                &incidence,
                &bend,
                points,
                UP,
                1.5
            ),
            Err(ReadError::Unsupported),
        );

        bend.rails = [(hinge.0, upper); 2];
        assert_eq!(
            replay(&wire, &incidence, &[bend], lower, upper).unwrap_err(),
            ReadError::Unsupported,
        );
        bad = boundary.clone();
        for face in [lower, upper] {
            bad.faces[face].holes = vec![bad.faces[face].holes[0].clone(); MAX_VERTICES];
        }
        let bends = (0..boundary.faces.len())
            .filter_map(|i| wire.bend(i, &incidence).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            replay(
                &Wire {
                    reader,
                    boundary: &bad
                },
                &incidence,
                &bends,
                lower,
                upper
            )
            .unwrap_err(),
            ReadError::Limit,
        );
    });
}

#[test]
fn refuses_shifted_unpaired_or_reused_flange_mouths() {
    inspect(|reader, boundary| {
        let wire = Wire {
            reader,
            boundary: &boundary,
        };
        let incidence = Incidence::new(&boundary);
        let [lower, _] = web_faces(&boundary);
        let bend = (0..boundary.faces.len())
            .filter_map(|i| wire.bend(i, &incidence).unwrap())
            .find(|bend| bend.rails.iter().any(|&(_, host)| host == lower))
            .unwrap();
        let hinge = reverse(&boundary.faces[lower].loop_)
            .find(|usage| bend.rails.contains(&(usage.0, lower)))
            .unwrap();
        let (section, _) =
            flange_section(&wire, &incidence, &bend, wire.points(hinge), UP, 1.5).unwrap();
        let [entry, exit] = [section[1].1, section[3].1];
        let chart = Chart {
            origin: [0.; 3],
            u: X,
            v: Y,
            up: UP,
            band: Band {
                center_z_mm: 0.,
                radius_mm: 1.,
            },
        };
        let failure = |reader: &Shell<'_, '_>, boundary: &Boundary, section: &[(Edge, usize)]| {
            match openings::read(&Wire { reader, boundary }, &incidence, chart, section) {
                Ok(_) => panic!("malformed tunnel passed"),
                Err(error) => error,
            }
        };

        let mut altered = section.clone();
        altered[1].0 = section[0].0;
        assert_eq!(failure(reader, &boundary, &altered), ReadError::Unsupported);
        altered = section.clone();
        altered.remove(3);
        assert_eq!(failure(reader, &boundary, &altered), ReadError::Unsupported);

        let mut bad = boundary.clone();
        bad.faces[exit].surface = bad.faces[entry].surface;
        assert_eq!(failure(reader, &bad, &section), ReadError::Unsupported);
        bad = boundary.clone();
        for index in [entry, exit] {
            let Surface::Plane { normal, .. } = &mut bad.faces[index].surface else {
                unreachable!()
            };
            *normal = normal.map(|value| -value);
        }
        assert_eq!(failure(reader, &bad, &section), ReadError::Invalid);
        bad = boundary.clone();
        let duplicate = bad.faces[entry].holes[0].clone();
        bad.faces[entry].holes.push(duplicate);
        assert_eq!(failure(reader, &bad, &section), ReadError::Invalid);

        let id = boundary.curves[&boundary.faces[exit].holes[0][0].0].0[0];
        reader.points.get_mut(&id).unwrap()[0] += 0.5;
        assert_eq!(failure(reader, &boundary, &section), ReadError::Unsupported);
    });
}

fn reverse_bend(mut section: Vec<(Edge, usize)>) -> Vec<(Edge, usize)> {
    let reflect = |[out, height]: [f64; 2]| [out, 1.5 - height];
    for (edge, _) in &mut section {
        *edge = match *edge {
            Edge::Line { a, b } => line(reflect(b), reflect(a)),
            Edge::Arc {
                center,
                a,
                b,
                clockwise,
            } => Edge::Arc {
                center: reflect(center),
                a: reflect(b),
                b: reflect(a),
                clockwise,
            },
            Edge::Circle { .. } => unreachable!("bends have bounded arc trims"),
        };
    }
    section.reverse();
    section.rotate_left(1);
    section
}

#[test]
fn adjacent_inset_flange_groups_replay_as_one_verified_step_solid() {
    let web_outline = [
        [0., 0.],
        [10., 0.],
        [90., 0.],
        [100., 0.],
        [100., 10.],
        [100., 60.],
        [100., 70.],
        [90., 70.],
        [10., 70.],
        [0., 70.],
        [0., 60.],
        [0., 10.],
    ];
    let sections = [
        section(20.),
        reverse_bend(section(16.)),
        section(18.),
        reverse_bend(section(12.)),
    ];
    let flanges = std::array::from_fn::<_, 4, _>(|i| SheetFlange {
        section: &sections[i],
        ends: [30 + i * 2, 31 + i * 2],
    });
    let hole = ring([[-2., -1.], [2., -1.], [2., 1.], [-2., 1.]]);
    let cut = [SheetFlangeOpening {
        walls: [1, 3],
        origin_mm: [50., -1.5, -10.],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
        through_mm: [0., -1.5, 0.],
        outline: &hole,
    }];
    let openings: [&[_]; 4] = [&cut, &[], &[], &[]];
    let web_hole = vec![(
        Edge::Circle {
            center: [25., 30.],
            radius_mm: 3.,
            clockwise: true,
        },
        42,
    )];
    for count in [3, 4] {
        let web = std::array::from_fn::<_, 12, _>(|i| SheetWebEdge {
            ends: [web_outline[i], web_outline[(i + 1) % web_outline.len()]],
            source: i,
            flange: (i % 3 == 1 && i / 3 < count).then_some(i / 3),
        });
        let shell = formed_sheet_with_openings(
            &web,
            std::slice::from_ref(&web_hole),
            [40, 41],
            &flanges[..count],
            &openings[..count],
            1.5,
            TOL,
            2. * TOL,
        )
        .unwrap();

        let bend_area = (3_f64.powi(2) - 1.5_f64.powi(2)) * std::f64::consts::FRAC_PI_4;
        let legs: f64 = [80., 50., 80., 50.][..count]
            .iter()
            .zip([20., 16., 18., 12.])
            .map(|(span, leg)| span * (1.5 * leg + bend_area))
            .sum();
        let expected = (100. * 70. - 9. * std::f64::consts::PI - 8.) * 1.5 + legs;
        let source = write_warped_union(&shell, ORIGIN, U, V).unwrap();
        let (proof, imported) = read_import_solid(&source, TOL).unwrap();
        assert_eq!(proof.solid_count, 1);
        assert_eq!(proof.face_count, shell.faces().len());
        near(proof.volume_mm3, expected);
        let ImportProfile::WarpedUnion { shell: restored } = &imported.profile else {
            panic!("expected the reconstructed inset sheet");
        };
        near(restored.volume_mm3(), expected);
        let area =
            |shell: &warped_union::Shell| shell.faces().iter().map(|face| face.area_mm2).sum();
        near(area(restored), area(&shell));

        // Keep the exact trims while reversing only one adjacent bend wall.
        let last_bend = shell
            .faces()
            .iter()
            .rposition(|face| {
                face.source_face != 42
                    && matches!(face.support, warped_union::Support::Cylinder { .. })
            })
            .unwrap();
        let face = source
            .lines()
            .filter(|line| line.contains("=ADVANCED_FACE("))
            .nth(last_bend)
            .unwrap();
        let altered = if face.ends_with(",.T.);") {
            face.replace(",.T.);", ",.F.);")
        } else {
            face.replace(",.F.);", ",.T.);")
        };
        assert_ne!(altered, face);
        assert!(read_import_solid(&source.replacen(face, &altered, 1), TOL).is_err());
    }
}
