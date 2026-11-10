use super::*;
use crate::step::{Writer, write_axial_profile_stack};

const TOL: f64 = 1e-7;
const ORIGIN: Point = [13., -5., 41.];
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];

fn fixture(min: [f64; 2], max: [f64; 2]) -> ProfileStack {
    let section = |span_mm, profile| Operand { span_mm, profile };
    let circle = Profile::Circle {
        center: [0.; 2],
        radius_mm: 6.,
    };
    let rectangle = Profile::Rectangle { min, max };
    ProfileStack::union(
        &[
            section([5., 17.], rectangle),
            section([133., 145.], rectangle),
            section([0., 150.], circle),
        ],
        TOL,
        1e-5,
    )
    .unwrap()
}

fn assert_rebuilt(readback: Readback, imported: ImportSolid, stack: &ProfileStack) {
    let ImportProfile::AxialProfileStack { stack: rebuilt } = imported.profile else {
        panic!("expected axial profile stack");
    };
    assert!((imported.height_mm - 150.).abs() < TOL);
    assert_eq!(rebuilt.faces().len(), stack.faces().len());
    assert_eq!(rebuilt.edges().len(), stack.edges().len());
    assert_eq!(rebuilt.vertices().len(), stack.vertices().len());
    assert_eq!(readback.face_count, stack.faces().len());
    assert!((rebuilt.volume_mm3() - stack.volume_mm3()).abs() < TOL);
    assert!((readback.volume_mm3 - rebuilt.volume_mm3()).abs() < TOL);
    let frame = Frame {
        origin: imported.origin_mm,
        axes: [imported.u, imported.v, cross(imported.u, imported.v)],
    };
    assert!(stack_metrics_match(&rebuilt, &frame, &readback, TOL));
}

#[test]
fn round_trips_the_toolcase_handle_obliquely_and_opposed_tangencies() {
    let handle = fixture([-6., -29.], [6., 6.]);
    let two_adjacent = fixture([-8., -29.], [6., 6.]);
    let two_opposed = fixture([-6., -29.], [6., 8.]);
    for (stack, origin, u, v) in [
        (&handle, ORIGIN, U, V),
        (&two_adjacent, ORIGIN, U, V.map(|x| -x)),
        (&two_opposed, [0.; 3], X, Y),
    ] {
        let text = write_axial_profile_stack(stack, origin, u, v).unwrap();
        let (readback, imported) = read_import_solid(&text, TOL).unwrap();
        assert_rebuilt(readback, imported, stack);
    }
    let mut writer = Writer::new();
    writer
        .append_axial_profile_stack(&handle, ORIGIN, U, V)
        .unwrap();
    writer
        .append_axial_profile_stack(&two_adjacent, [300., 0., 0.], X, Y)
        .unwrap();
    let text = writer.finish().unwrap();
    let members = read_import_compound(&text, TOL).unwrap();
    assert_eq!(members.len(), 2);
    for ((readback, imported), stack) in members.into_iter().zip([&handle, &two_adjacent]) {
        assert_rebuilt(readback, imported, stack);
    }
}

#[test]
fn refuses_disagreeing_sections_metrics_and_face_support() {
    let stack = fixture([-6., -29.], [6., 6.]);
    let text = write_axial_profile_stack(&stack, [0.; 3], X, Y).unwrap();
    let entities = parse_entities(&text, TOL).unwrap();
    let mut shell = Shell::new(&entities, TOL, true);
    let mut faces = shell.validated_faces().unwrap();
    let mut metrics = shell.readback(&faces).unwrap();
    let face_edges = shell.indexed_edges(faces.len());
    let frame = Frame {
        origin: [0.; 3],
        axes: [X, Y, cross(X, Y)],
    };
    let levels = Levels::from_points(&shell.points, &frame, TOL);
    assert!(
        shell
            .stack_operands(&faces, &face_edges, &levels, &frame, 6.)
            .is_some()
    );
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &face_edges)
            .is_some()
    );

    metrics.bounds_mm[1][0] += 0.2;
    assert!(shell.axial_profile_stack(&faces, &metrics).is_none());
    metrics.volume_mm3 += 2.;
    assert!(shell.axial_profile_stack(&faces, &metrics).is_none());
    let mut wrong_circle = shell.readback(&faces).unwrap();
    wrong_circle.arcs[0].radius_mm = 10.;
    assert!(shell.axial_profile_stack(&faces, &wrong_circle).is_none());

    let other = fixture([-8., -29.], [6., 6.]);
    assert!(
        shell
            .match_stack(&other, &frame, &levels, &faces, &face_edges)
            .is_none()
    );

    let wall = faces
        .iter()
        .position(|face| matches!(face, Face::Planar { normal, .. } if normal[2] == 0.))
        .unwrap();
    let wrong = Face::Planar {
        origin: [0.; 3],
        normal: [
            std::f64::consts::FRAC_1_SQRT_2,
            0.,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
        area: 1.,
    };
    let wall_face = std::mem::replace(&mut faces[wall], wrong);
    assert!(
        shell
            .stack_operands(&faces, &face_edges, &levels, &frame, 6.)
            .is_none()
    );
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &face_edges)
            .is_none()
    );

    faces[wall] = Face::Cylindrical { seam: 0, flux: 1. };
    assert!(
        shell
            .stack_operands(&faces, &face_edges, &levels, &frame, 6.)
            .is_none()
    );
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &face_edges)
            .is_none()
    );
    faces[wall] = wall_face;

    let arc = *shell
        .edges
        .iter()
        .find(|(_, edge)| matches!(edge.curve, Curve::Arc(_)))
        .unwrap()
        .0;
    let curve = std::mem::replace(&mut shell.edges.get_mut(&arc).unwrap().curve, Curve::Line);
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &face_edges)
            .is_none()
    );
    shell.edges.get_mut(&arc).unwrap().curve = curve;

    let line = *shell
        .edges
        .iter()
        .find(|(_, edge)| matches!(edge.curve, Curve::Line))
        .unwrap()
        .0;
    shell.edges.get_mut(&line).unwrap().curve = curve;
    assert!(shell.axial_profile_stack(&faces, &metrics).is_none());
    shell.edges.get_mut(&line).unwrap().curve = Curve::Line;

    let edge = &mut shell.edges.get_mut(&arc).unwrap().uses[0];
    edge.forward = !edge.forward;
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &face_edges)
            .is_none()
    );
    let edge = &mut shell.edges.get_mut(&arc).unwrap().uses[0];
    edge.forward = !edge.forward;

    let duplicate = *shell
        .edges
        .iter()
        .find(|(_, edge)| matches!(edge.curve, Curve::Line))
        .unwrap()
        .0;
    let edge = &shell.edges[&duplicate];
    let endpoints = edge.vertices;
    let next = *shell
        .edges
        .iter()
        .find(|&(&id, edge)| id != duplicate && matches!(edge.curve, Curve::Line))
        .unwrap()
        .0;
    let original = std::mem::replace(&mut shell.edges.get_mut(&next).unwrap().vertices, endpoints);
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &face_edges)
            .is_none()
    );
    shell.edges.get_mut(&next).unwrap().vertices = original;

    let unused = *shell
        .points
        .keys()
        .find(|&&id| !endpoints.contains(&id))
        .unwrap();
    shell.edges.get_mut(&duplicate).unwrap().vertices = [unused, unused];
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &face_edges)
            .is_none()
    );
    shell.edges.get_mut(&duplicate).unwrap().vertices = endpoints;

    let mut wrong_face_edges = face_edges.clone();
    wrong_face_edges[0][0] = *shell
        .edges
        .keys()
        .find(|id| !face_edges[0].contains(id))
        .unwrap();
    assert!(
        shell
            .match_stack(&stack, &frame, &levels, &faces, &wrong_face_edges)
            .is_none()
    );

    let displaced = Frame {
        origin: [0.5, 0., 0.],
        axes: frame.axes,
    };
    let mismatched_levels = Levels::from_points(&shell.points, &displaced, TOL);
    assert!(
        shell
            .match_stack(&stack, &frame, &mismatched_levels, &faces, &face_edges)
            .is_none()
    );
}

#[test]
fn one_profile_is_not_a_mixed_stack() {
    let stock = ProfileStack::union(
        &[Operand {
            span_mm: [0., 9.],
            profile: Profile::Rectangle {
                min: [-6., -29.],
                max: [6., 6.],
            },
        }],
        TOL,
        1e-5,
    )
    .unwrap();
    let text = write_axial_profile_stack(&stock, [0.; 3], X, Y).unwrap();
    let entities = parse_entities(&text, TOL).unwrap();
    let mut shell = Shell::new(&entities, TOL, true);
    let faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let frame = Frame {
        origin: [0.; 3],
        axes: [X, Y, cross(X, Y)],
    };
    let levels = Levels::from_points(&shell.points, &frame, TOL);
    assert!(
        shell
            .stack_operands(&faces, &edges, &levels, &frame, 6.)
            .is_none()
    );
    let (_, imported) = read_import_solid(&text, TOL).unwrap();
    assert!(matches!(imported.profile, ImportProfile::Polygon { .. }));
}

#[test]
fn one_round_profile_has_no_rectangular_frame() {
    let stock = ProfileStack::union(
        &[Operand {
            span_mm: [0., 9.],
            profile: Profile::Circle {
                center: [0.; 2],
                radius_mm: 6.,
            },
        }],
        TOL,
        1e-5,
    )
    .unwrap();
    let text = write_axial_profile_stack(&stock, [0.; 3], X, Y).unwrap();
    let entities = parse_entities(&text, TOL).unwrap();
    let mut shell = Shell::new(&entities, TOL, true);
    let faces = shell.validated_faces().unwrap();
    let readback = shell.readback(&faces).unwrap();
    assert!(shell.axial_profile_stack(&faces, &readback).is_none());
}
