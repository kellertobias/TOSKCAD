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
