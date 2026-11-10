use super::*;
use crate::step::write_nonplanar_loft;

const TOL: f64 = 1e-7;

fn fixture(clockwise: bool) -> (NonplanarLoft, String) {
    let low = [[-2., -1.], [2., -1.], [2., 1.], [-2., 1.]];
    let (sin, cos) = std::f64::consts::FRAC_PI_6.sin_cos();
    let high = low.map(|[x, y]| [cos * x - sin * y + 1., sin * x + cos * y + 0.5]);
    let mut rings = [low, high];
    if clockwise {
        rings.iter_mut().for_each(|ring| ring.reverse());
    }
    let loft = NonplanarLoft::new([&rings[0], &rings[1]], 6., TOL).unwrap();
    let step =
        write_nonplanar_loft(&loft, [11., -4., 9.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]).unwrap();
    (loft, step)
}

fn rewrite(step: &str, pattern: &str, transform: impl FnOnce(&str) -> String) -> String {
    let line = step.lines().find(|line| line.contains(pattern)).unwrap();
    step.replacen(line, &transform(line), 1)
}

fn reorder_faces(step: &str, order: impl FnOnce(&mut Vec<&str>)) -> String {
    rewrite(step, "=CLOSED_SHELL", |line| {
        let (prefix, tail) = line.rsplit_once(",(").unwrap();
        let mut faces: Vec<_> = tail.strip_suffix("));").unwrap().split(',').collect();
        order(&mut faces);
        format!("{prefix},({}));", faces.join(","))
    })
}

#[test]
fn replays_windings_and_face_order_from_exact_step_edges_and_poles() {
    for clockwise in [false, true] {
        let (loft, step) = fixture(clockwise);
        let reordered = reorder_faces(&step, |faces| faces.reverse());
        let before = read_analytic_solid(&step, TOL).unwrap();
        assert_eq!(before.face_count, 6);
        assert_eq!(before.edges_mm.len(), 12);
        assert_eq!(before.solid_count, 1);
        assert!(before.circles.is_empty());
        for source in [&step, &reordered] {
            let restored = read_nonplanar_loft_solid(source, TOL).unwrap();
            let after = read_analytic_solid(source, TOL).unwrap();
            let (imported_proof, imported) = read_import_solid(source, TOL).unwrap();
            assert_eq!(after, imported_proof);
            assert!(
                matches!(imported.profile, ImportProfile::NonplanarLoft { geometry } if geometry == restored.geometry)
            );
            let near = |a: f64, b: f64| assert!((a - b).abs() < TOL, "{a} != {b}");
            near(restored.geometry.volume_mm3(), loft.volume_mm3());
            near(after.volume_mm3, loft.volume_mm3());
            assert_eq!(after.bounds_mm, before.bounds_mm);
            assert_eq!(after.edges_mm, before.edges_mm);
            let world = |[x, y, z]: Point| {
                std::array::from_fn::<_, 3, _>(|axis| {
                    restored.origin_mm[axis]
                        + restored.u[axis] * x
                        + restored.v[axis] * y
                        + cross(restored.u, restored.v)[axis] * z
                })
            };
            for &vertex in restored.geometry.rings().iter().flatten() {
                assert!(
                    after
                        .edges_mm
                        .iter()
                        .flatten()
                        .any(|&p| length(sub(p, world(vertex))) < TOL)
                );
            }
        }
    }
}

#[test]
fn rejects_altered_supports_and_duplicate_faces() {
    let (_, step) = fixture(false);
    for altered in [
        rewrite(&step, "=B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            line.replacen("(0.,1.)", "(0.)", 1)
        }),
        rewrite(&step, "=B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            line.replace("(2,2)", "(1,3)")
        }),
        rewrite(&step, "=B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            line.replace("(0.,1.)", "(0.,0.)")
        }),
        rewrite(&step, "=B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            line.replace(".RULED_SURF.", ".CONICAL_SURF.")
        }),
        rewrite(&step, "=B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            let start = line.find("((").unwrap() + 2;
            let end = line[start..].find(')').unwrap() + start;
            let first = line[start..end].split(',').next().unwrap();
            format!("{}{first},{first}{}", &line[..start], &line[end..])
        }),
        reorder_faces(&step, |faces| faces[1] = faces[0]),
    ] {
        assert_eq!(read_analytic_solid(&altered, TOL), Err(ReadError::Invalid));
    }
    assert!(matches!(
        read_nonplanar_loft_solid(&step, 0.),
        Err(ReadError::Invalid)
    ));
}

fn step_line(step: &str, index: usize, kind: &str) -> (u32, String) {
    let line = step
        .lines()
        .filter(|line| line.contains(kind))
        .nth(index)
        .unwrap();
    let (id, args) = line.split_once('=').unwrap();
    (id[1..].parse().unwrap(), args.to_string())
}

fn replace_entity(step: &str, id: u32, args: &str) -> String {
    rewrite(step, &format!("#{id}="), |_| format!("#{id}={args}"))
}

fn flip_last_sense(args: &str) -> String {
    let (fields, tail) = args.rsplit_once(',').unwrap();
    let opposite = if tail == ".T.);" { ".F.);" } else { ".T.);" };
    format!("{fields},{opposite}")
}

#[test]
fn rejects_wrong_cap_bounds_wall_incidence_and_short_loops() {
    let (_, step) = fixture(false);
    let entities = parse_entities(&step, TOL).unwrap();
    let mut shell = Shell::new(&entities, TOL, true);
    let faces = shell
        .member_faces(shell.represented_solids(true).unwrap()[0])
        .unwrap();
    let [first, last, wall, adjacent, ..] = faces.as_slice() else {
        panic!("expected loft faces");
    };
    let fields = |id| {
        let entity = surface(&shell, id).unwrap().unwrap();
        (entity.id, entity.bound)
    };
    let [
        (first_surface, first_bound),
        (last_surface, _),
        (_, wall_bound),
        (_, adjacent_bound),
    ] = [*first, *last, *wall, *adjacent].map(fields);
    let (_, first_args) = step_line(&step, 0, "=ADVANCED_FACE");
    let (_, wall_args) = step_line(&step, 2, "=ADVANCED_FACE");
    let (_, adjacent_args) = step_line(&step, 3, "=ADVANCED_FACE");
    let (_, _, forward) = shell.bound(first_bound, true).unwrap();
    assert!(forward);

    let (wire, _) = step_line(&step, 0, "=EDGE_LOOP");
    let (_, adjacent_bound_args) = step_line(&step, 3, "=FACE_OUTER_BOUND");
    let reversed_wall = replace_entity(&step, *adjacent, &flip_last_sense(&adjacent_args));
    let reversed_wall = replace_entity(
        &reversed_wall,
        adjacent_bound,
        &flip_last_sense(&adjacent_bound_args),
    );

    let (_, wall_wire, _) = shell.bound(wall_bound, true).unwrap();
    let edges = refs(&shell.args(wall_wire, "EDGE_LOOP").unwrap()[1]).unwrap();
    let oriented = edges[0];
    let curve = reference(&shell.args(oriented, "ORIENTED_EDGE").unwrap()[3]).unwrap();
    let curve_args = step
        .lines()
        .find_map(|line| line.strip_prefix(&format!("#{curve}=")))
        .unwrap();
    let oriented_args = step
        .lines()
        .find_map(|line| line.strip_prefix(&format!("#{oriented}=")))
        .unwrap();
    let duplicated_edge = replace_entity(
        &step,
        oriented,
        &oriented_args.replace(&format!("#{curve},"), "#900000,"),
    )
    .replacen(
        "ENDSEC;\nEND-ISO",
        &format!("#900000={curve_args}\nENDSEC;\nEND-ISO"),
        1,
    );

    for altered in [
        replace_entity(&step, *first, "ADVANCED_FACE('');"),
        replace_entity(&step, *last, &first_args),
        replace_entity(
            &step,
            *wall,
            &wall_args.replace(&format!("(#{wall_bound})"), &format!("(#{first_bound})")),
        ),
        reversed_wall,
        duplicated_edge,
        replace_entity(&step, wire, "EDGE_LOOP('',());"),
        replace_entity(&step, first_surface, "PLANE('');"),
        replace_entity(&step, *first, &flip_last_sense(&first_args)),
        replace_entity(
            &step,
            *first,
            &first_args.replace(&format!("#{first_surface},"), &format!("#{last_surface},")),
        ),
    ] {
        assert_eq!(
            read_analytic_solid(&altered, TOL),
            Err(ReadError::Invalid),
            "{altered}"
        );
    }

    let bounds = refs(&shell.args(*first, "ADVANCED_FACE").unwrap()[1]).unwrap();
    let (_, wire, sense) = shell.bound(bounds[0], true).unwrap();
    assert_eq!(shell.traversals(wire, sense, 0, 4).unwrap().len(), 4);
    shell.edges.get_mut(&curve).unwrap().curve = Curve::Circle(CircleEdge {
        center_mm: [0.; 3],
        normal: [0., 0., 1.],
        radius_mm: 1.,
    });
    assert_eq!(
        loop_vertices(&mut shell, *wall, wall_bound, 4),
        Err(ReadError::Invalid)
    );
}

#[test]
fn rejects_incomplete_bilinear_nets_and_invalid_reconstructed_sections() {
    let (_, step) = fixture(false);
    for row_count in [true, false] {
        let altered = rewrite(&step, "=B_SPLINE_SURFACE_WITH_KNOTS", |line| {
            let start = line.find("((").unwrap() + 2;
            let end = line[start..].find(')').unwrap() + start;
            if row_count {
                let other_end = line[end + 1..].find("))").unwrap() + end + 1;
                format!("{}){}", &line[..end + 1], &line[other_end + 2..])
            } else {
                let first = line[start..end].split(',').next().unwrap();
                format!("{}{first}{}", &line[..start], &line[end..])
            }
        });
        assert_eq!(read_analytic_solid(&altered, TOL), Err(ReadError::Invalid));
    }

    let entities = BTreeMap::new();
    let mut shell = Shell::new(&entities, TOL, true);
    let [first, last] = [[1, 2, 3, 4], [5, 6, 7, 8]];
    let caps = |end_normal| {
        [
            Plane {
                origin: [0.; 3],
                normal: [0., 0., -1.],
                u: [1., 0., 0.],
                v: [0., -1., 0.],
            },
            Plane {
                origin: [0., 0., 4.],
                normal: end_normal,
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
        ]
    };
    for (id, position) in first.into_iter().chain(last).zip([
        [-1., -1., 0.],
        [1., -1., 0.],
        [1., 1., 0.],
        [-1., 1., 0.],
        [1., 1., 4.],
        [-1., 1., 4.],
        [-1., -1., 4.],
        [1., -1., 4.],
    ]) {
        shell.points.insert(id, position);
    }
    let check = |shell: &Shell<'_, '_>, normal| {
        let [low, high] = caps(normal);
        assert!(matches!(
            reconstruct(shell, low, high, [&first, &last]),
            Err(ReadError::Invalid)
        ));
    };
    check(&shell, [0., 1., 0.]);
    check(&shell, [0., 0., 1.]);
    shell.points.get_mut(&last[1]).unwrap()[2] += 0.2;
    check(&shell, [0., 0., 1.]);
    assert_eq!(verify_cycle(&first, &last), Err(ReadError::Invalid));
}

fn decorate_edge(step: &str, rail: bool, spline: bool, reverse: bool) -> String {
    let entities = parse_entities(step, TOL).unwrap();
    let shell = Shell::new(&entities, TOL, true);
    let solid = shell.represented_solids(true).unwrap()[0];
    let face = shell.member_faces(solid).unwrap()[2];
    let wall = surface(&shell, face).unwrap().unwrap();
    let (_, wire, _) = shell.bound(wall.bound, true).unwrap();
    let edges = refs(&shell.args(wire, "EDGE_LOOP").unwrap()[1]).unwrap();
    let oriented = edges[usize::from(rail)];
    let edge = reference(&shell.args(oriented, "ORIENTED_EDGE").unwrap()[3]).unwrap();
    let args = shell.args(edge, "EDGE_CURVE").unwrap();
    let [start, end, line] = [1, 2, 3].map(|i| reference(&args[i]).unwrap());
    let mut poles = [start, end]
        .map(|vertex| reference(&shell.args(vertex, "VERTEX_POINT").unwrap()[1]).unwrap());
    if reverse {
        poles.reverse();
    }
    let [a, b] = poles;
    let (origin, direction, scale) = if rail {
        ("1.,0.", "0.,1.", 1.)
    } else {
        ("0.,0.", "1.,0.", 0.25)
    };
    let curve = if spline { 900008 } else { line };
    let side = if reverse { ".F." } else { ".T." };
    let master = if reverse { "CURVE_3D" } else { "PCURVE_S1" };
    let mut extra = format!(
        "#900001=SURFACE_CURVE('',#{curve},(#900002),.{master}.);\n\
         #900002=PCURVE('',#{},#900003);\n\
         #900003=DEFINITIONAL_REPRESENTATION('',(#900004),#900005);\n\
         #900004=LINE('',#900006,#900007);\n\
         #900005=GEOMETRIC_REPRESENTATION_CONTEXT(2);\n\
         #900006=CARTESIAN_POINT('',({origin}));\n\
         #900007=VECTOR('',#900009,{scale});\n\
         #900009=DIRECTION('',({direction}));\n",
        wall.id
    );
    if spline {
        use std::fmt::Write;
        writeln!(extra,
            "#900008=B_SPLINE_CURVE_WITH_KNOTS('',1,(#{a},#{b}),.UNSPECIFIED.,.F.,.F.,(2,2),(0.,1.),.PIECEWISE_BEZIER_KNOTS.);"
        ).unwrap();
    }
    replace_entity(
        step,
        edge,
        &format!("EDGE_CURVE('',#{start},#{end},#900001,{side});"),
    )
    .replacen("ENDSEC;\nEND-ISO", &format!("{extra}ENDSEC;\nEND-ISO"), 1)
}

#[test]
fn verifies_bilinear_master_pcurves_and_straight_spline_rails() {
    let (_, step) = fixture(false);
    let expected = read_analytic_solid(&step, TOL).unwrap();
    let bottom = decorate_edge(&step, false, false, false);
    let rail = decorate_edge(&step, true, true, false);
    let reversed = decorate_edge(&step, true, true, true);
    for source in [&bottom, &rail, &reversed] {
        assert_eq!(read_analytic_solid(source, TOL).unwrap(), expected);
    }
    let diagonal = replace_entity(&rail, 900009, "DIRECTION('',(1.,1.));");
    let interior = replace_entity(&rail, 900006, "CARTESIAN_POINT('',(0.5,0.));");
    let singular = rewrite(&bottom, "=B_SPLINE_SURFACE_WITH_KNOTS", |line| {
        line.replacen("(0.,1.)", "(0.,1.E-310)", 1)
    });
    let wrong_master = replace_entity(&rail, 900007, "VECTOR('',#900009,0.5);");
    let wrong_trim = rewrite(&reversed, "#900008=", |line| {
        let start = line.find("(#").unwrap() + 1;
        let end = line[start..].find(')').unwrap() + start;
        let poles: Vec<_> = line[start..end].split(',').collect();
        format!(
            "{}{},{}{}",
            &line[..start],
            poles[1],
            poles[0],
            &line[end..]
        )
    });
    for source in [diagonal, interior] {
        assert_eq!(
            read_analytic_solid(&source, TOL),
            Err(ReadError::Unsupported)
        );
    }
    for source in [singular, wrong_master, wrong_trim] {
        assert_eq!(read_analytic_solid(&source, TOL), Err(ReadError::Invalid));
    }
}
