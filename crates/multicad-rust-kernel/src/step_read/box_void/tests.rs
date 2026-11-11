use super::*;
use crate::orthogonal::{Box3, Operation, boolean};
fn fixture() -> String {
    let outer = Box3 {
        min: [0.; 3],
        max: [15., 10., 8.],
    };
    let inner = Box3 {
        min: [2., 3., 1.],
        max: [7., 7., 4.],
    };
    let solid = boolean(&[outer], &[inner], Operation::Subtract, 1e-7)
        .unwrap()
        .unwrap();
    let boundary = crate::orthogonal_boundary::build(solid.quads()).unwrap();
    crate::step::write_orthogonal(&boundary, |p| p).unwrap()
}
#[test]
fn writes_and_reads_one_box_void_solid_without_inventing_a_second_part() {
    let text = fixture();
    assert_eq!(text.matches("=BREP_WITH_VOIDS(").count(), 1);
    let proof = read_planar_solid(&text, 1e-7).unwrap();
    assert_eq!(proof.solid_count, 1);
    assert_eq!(proof.face_count, 12);
    assert_eq!(proof.volume_mm3, 1140.);
    let (_, import) = read_import_solid(&text, 1e-7).unwrap();
    let ImportProfile::Orthogonal { solid } = import.profile else {
        panic!()
    };
    assert_eq!(solid.volume_mm3(), 1140.);
}

fn entity(text: &str, id: u32, args: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with(&format!("#{id}=")) {
                format!("#{id}={args};")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
fn ids(text: &str) -> (u32, u32, u32, u32) {
    let entities = parse_entities(text, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, false);
    let member = shell.represented_solids(true).unwrap()[0];
    let [_, outer, voids] = shell.args(member, "BREP_WITH_VOIDS").unwrap() else {
        panic!()
    };
    let wrapper = refs(voids).unwrap()[0];
    let [_, _, inner, _] = shell.args(wrapper, "ORIENTED_CLOSED_SHELL").unwrap() else {
        panic!()
    };
    (
        member,
        reference(outer).unwrap(),
        reference(inner).unwrap(),
        wrapper,
    )
}
fn reverse_shell(text: &str, shell_id: u32) -> String {
    let entities = parse_entities(text, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, false);
    let [_, faces] = shell.args(shell_id, "CLOSED_SHELL").unwrap() else {
        panic!()
    };
    let mut ids = Vec::new();
    for face in refs(faces).unwrap() {
        ids.push(face);
        let [_, bounds, _, _] = shell.args(face, "ADVANCED_FACE").unwrap() else {
            panic!()
        };
        ids.extend(refs(bounds).unwrap());
    }
    let mut result = text.to_owned();
    for id in ids {
        let old = result
            .lines()
            .find(|line| line.starts_with(&format!("#{id}=")))
            .unwrap()
            .to_owned();
        let new = old.strip_suffix(",.T.);").unwrap().to_owned() + ",.F.);";
        result = result.replace(&old, &new);
    }
    result
}
fn assert_refused(text: &str) {
    assert!(read_planar_solid(text, 1e-7).is_err());
    assert!(read_analytic_compound(text, 1e-7).is_err());
    assert!(read_import_solid(text, 1e-7).is_err());
}

#[test]
fn accepts_both_consistent_void_orientations_and_rejects_inverted_material() {
    let text = fixture();
    let (member, outer, inner, wrapper) = ids(&text);
    let reversed = reverse_shell(&text, inner);
    let alternative = entity(
        &reversed,
        wrapper,
        &format!("ORIENTED_CLOSED_SHELL('',*,#{inner},.T.)"),
    );
    assert!((read_planar_solid(&alternative, 1e-7).unwrap().volume_mm3 - 1140.).abs() < 1e-7);
    assert_eq!(
        read_analytic_compound_with_helical_cuts(&alternative, 1e-7).unwrap()[0].1,
        None
    );
    let (_, imported) = read_import_solid(&alternative, 1e-7).unwrap();
    let ImportProfile::Orthogonal { solid } = imported.profile else {
        panic!()
    };
    assert_eq!(solid.solid_count(), 1);
    assert_refused(&entity(
        &text,
        wrapper,
        &format!("ORIENTED_CLOSED_SHELL('',*,#{inner},.T.)"),
    ));
    assert_refused(&reverse_shell(&text, outer));
    // Valid material with the declaration's exterior and cavity roles swapped
    // must still fail instead of silently repairing a malformed BRep.
    let swapped = entity(
        &reversed,
        member,
        &format!("BREP_WITH_VOIDS('',#{inner},(#{wrapper}))"),
    );
    assert_refused(&entity(
        &swapped,
        wrapper,
        &format!("ORIENTED_CLOSED_SHELL('',*,#{outer},.T.)"),
    ));
}

#[test]
fn malformed_shell_records_shared_topology_and_orphans_fail_closed() {
    let text = fixture();
    let (member, outer, inner, wrapper) = ids(&text);
    for args in [
        "BREP_WITH_VOIDS('')".to_owned(),
        format!("BREP_WITH_VOIDS('',#{outer},())"),
        format!("BREP_WITH_VOIDS('',#{outer},(#{wrapper},#{wrapper}))"),
    ] {
        assert_refused(&entity(&text, member, &args));
    }
    for args in [
        "ORIENTED_CLOSED_SHELL('')".to_owned(),
        format!("ORIENTED_CLOSED_SHELL('',*,#{outer},.F.)"),
        format!("ORIENTED_CLOSED_SHELL('',$,#{inner},.F.)"),
        format!("ORIENTED_CLOSED_SHELL('',*,#{inner},.UNKNOWN.)"),
        format!("OPEN_SHELL('',#{inner})"),
    ] {
        assert_refused(&entity(&text, wrapper, &args));
    }
    assert_refused(&entity(&text, inner, "CLOSED_SHELL('')"));
    assert_refused(&entity(&text, member, "SHELL_BASED_SURFACE_MODEL('')"));
    let entities = parse_entities(&text, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, false);
    let [_, outer_faces] = shell.args(outer, "CLOSED_SHELL").unwrap() else {
        panic!()
    };
    let outer_faces = refs(outer_faces).unwrap();
    let face_list = outer_faces
        .iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",");
    assert_refused(&entity(
        &text,
        inner,
        &format!("CLOSED_SHELL('',({face_list}))"),
    ));
    let [_, inner_faces] = shell.args(inner, "CLOSED_SHELL").unwrap() else {
        panic!()
    };
    let inner_faces = refs(inner_faces).unwrap();
    let mut malformed = inner_faces.clone();
    malformed.push(inner_faces[0]);
    assert_refused(&entity(
        &text,
        inner,
        &format!(
            "CLOSED_SHELL('',({}))",
            malformed
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
    ));
    assert_refused(&entity(
        &text,
        inner,
        &format!(
            "CLOSED_SHELL('',({}))",
            inner_faces[1..]
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
    ));
    let extra = entities.keys().max().unwrap() + 1;
    let orphan = text.replace(
        "ENDSEC;\nEND-ISO-10303-21;",
        &format!("#{extra}=CLOSED_SHELL('',({face_list}));\nENDSEC;\nEND-ISO-10303-21;"),
    );
    assert_refused(&orphan);
}

fn move_inner(text: &str, delta: Point) -> String {
    let (_, _, inner, _) = ids(text);
    let entities = parse_entities(text, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, false);
    let [_, faces] = shell.args(inner, "CLOSED_SHELL").unwrap() else {
        panic!()
    };
    let mut raw = Shell::new(&entities, 1e-7, false);
    raw.validate_faces(refs(faces).unwrap()).unwrap();
    let mut result = text.to_owned();
    let mut points = BTreeSet::new();
    let mut placements = BTreeSet::new();
    for vertex in raw.points.keys() {
        let [_, point] = shell.args(*vertex, "VERTEX_POINT").unwrap() else {
            panic!()
        };
        points.insert(reference(point).unwrap());
    }
    let [_, faces] = shell.args(inner, "CLOSED_SHELL").unwrap() else {
        panic!()
    };
    for face in refs(faces).unwrap() {
        let [_, _, surface, _] = shell.args(face, "ADVANCED_FACE").unwrap() else {
            panic!()
        };
        let [_, placement] = shell.args(reference(surface).unwrap(), "PLANE").unwrap() else {
            panic!()
        };
        placements.insert(reference(placement).unwrap());
    }
    for placement in placements {
        let [_, origin, _, _] = shell.args(placement, "AXIS2_PLACEMENT_3D").unwrap() else {
            panic!()
        };
        points.insert(reference(origin).unwrap());
    }
    for point in points {
        let p = shell.point(point).unwrap();
        result = entity(
            &result,
            point,
            &format!(
                "CARTESIAN_POINT('',({},{},{}))",
                p[0] + delta[0],
                p[1] + delta[1],
                p[2] + delta[2]
            ),
        );
    }
    result
}

#[test]
fn touching_crossing_and_external_cavities_are_rejected_without_repair() {
    let text = fixture();
    for delta in [
        [-2., 0., 0.],
        [-3., 0., 0.],
        [20., 0., 0.],
        [0., 3., 0.],
        [0., 0., 7.],
    ] {
        assert_refused(&move_inner(&text, delta));
    }
}

#[test]
fn arbitrary_rigid_placements_recover_material_and_reexport_the_physical_void() {
    let outer = Box3 {
        min: [-4., 2., 1.],
        max: [16., 14., 10.],
    };
    let inner = Box3 {
        min: [1., 4., 3.],
        max: [8., 10., 8.],
    };
    let solid = boolean(&[outer], &[inner], Operation::Subtract, 1e-7)
        .unwrap()
        .unwrap();
    let boundary = crate::orthogonal_boundary::build(solid.quads()).unwrap();
    let origin = [13., -5., 41.];
    let u = [0.8, 0.6, 0.];
    let v = [-0.36, 0.48, 0.8];
    let w = cross(u, v);
    let text = crate::step::write_orthogonal(&boundary, |p| {
        std::array::from_fn(|i| origin[i] + u[i] * p[0] + v[i] * p[1] + w[i] * p[2])
    })
    .unwrap();
    let (proof, imported) = read_import_solid(&text, 1e-7).unwrap();
    assert!((proof.volume_mm3 - (2160. - 210.)).abs() < 1e-7);
    assert_eq!(proof.solid_count, 1);
    assert_eq!(proof.edges_mm.len(), 24);
    let ImportProfile::Orthogonal { solid } = imported.profile else {
        panic!()
    };
    let boundary = crate::orthogonal_boundary::build(solid.quads()).unwrap();
    let w = cross(imported.u, imported.v);
    let rewritten = crate::step::write_orthogonal(&boundary, |p| {
        std::array::from_fn(|i| {
            imported.origin_mm[i] + imported.u[i] * p[0] + imported.v[i] * p[1] + w[i] * p[2]
        })
    })
    .unwrap();
    let replay = read_analytic_solid(&rewritten, 1e-7).unwrap();
    assert!((replay.volume_mm3 - proof.volume_mm3).abs() < 1e-7);
    for (a, b) in replay
        .bounds_mm
        .into_iter()
        .flatten()
        .zip(proof.bounds_mm.into_iter().flatten())
    {
        assert!((a - b).abs() < 1e-7);
    }
    for edge in &proof.edges_mm {
        assert!(
            replay
                .edges_mm
                .iter()
                .any(|candidate| [false, true].into_iter().any(|reverse| edge
                    .iter()
                    .enumerate()
                    .all(
                        |(i, p)| length(sub(*p, candidate[if reverse { 1 - i } else { i }])) < 1e-7
                    )))
        );
    }
    assert!(!solid.cells().iter().any(|cell| {
        let world = [4., 6., 5.];
        let p = std::array::from_fn::<_, 3, _>(|i| {
            origin[i] + u[i] * world[0] + v[i] * world[1] + cross(u, v)[i] * world[2]
        });
        let local = [imported.u, imported.v, w].map(|axis| dot(sub(p, imported.origin_mm), axis));
        (0..3).all(|i| local[i] > cell.min[i] && local[i] < cell.max[i])
    }));
}

fn enclose_pair(text: &str) -> String {
    let entities = parse_entities(text, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, false);
    let members = shell.represented_solids(false).unwrap();
    assert_eq!(members.len(), 2);
    let [_, outer] = shell.args(members[0], "MANIFOLD_SOLID_BREP").unwrap() else {
        panic!()
    };
    let [_, inner] = shell.args(members[1], "MANIFOLD_SOLID_BREP").unwrap() else {
        panic!()
    };
    let mut result = entity(
        text,
        members[0],
        &format!(
            "BREP_WITH_VOIDS('',#{},(#{}))",
            reference(outer).unwrap(),
            members[1]
        ),
    );
    result = entity(
        &result,
        members[1],
        &format!(
            "ORIENTED_CLOSED_SHELL('',*,#{},.F.)",
            reference(inner).unwrap()
        ),
    );
    let (&id, representation) = entities
        .iter()
        .find_map(|(id, parts)| {
            (parts[0].name == "ADVANCED_BREP_SHAPE_REPRESENTATION").then_some((id, &parts[0]))
        })
        .unwrap();
    let [_, items, context] = representation.args.as_slice() else {
        panic!()
    };
    let items = refs(items)
        .unwrap()
        .into_iter()
        .filter(|&id| id != members[1])
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",");
    entity(
        &result,
        id,
        &format!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('',({items}),#{})",
            reference(context).unwrap()
        ),
    )
}
#[test]
fn nonrectangular_and_multiple_wire_shells_are_rejected_as_cavities() {
    use crate::{Rational, polygon::PolygonPrism, step::Writer};
    let n = |value| Rational::new(value, 1).unwrap();
    let mut writer = Writer::new();
    writer
        .append_box([n(15), n(10), n(8)], [0.; 3], [1., 0., 0.], [0., 1., 0.])
        .unwrap();
    let triangle =
        PolygonPrism::new(vec![vec![[n(0), n(0)], [n(2), n(0)], [n(0), n(2)]]], n(2)).unwrap();
    writer
        .append_polygon_prism(&triangle, [2., 3., 1.], [1., 0., 0.], [0., 1., 0.], false)
        .unwrap();
    assert_refused(&enclose_pair(&writer.finish().unwrap()));
    let mut writer = Writer::new();
    let profile = PolygonPrism::new(
        vec![
            vec![[n(0), n(0)], [n(15), n(0)], [n(15), n(10)], [n(0), n(10)]],
            vec![[n(1), n(1)], [n(1), n(2)], [n(2), n(2)], [n(2), n(1)]],
        ],
        n(8),
    )
    .unwrap();
    writer
        .append_polygon_prism(&profile, [0.; 3], [1., 0., 0.], [0., 1., 0.], false)
        .unwrap();
    writer
        .append_box([n(2); 3], [5., 5., 2.], [1., 0., 0.], [0., 1., 0.])
        .unwrap();
    assert_refused(&enclose_pair(&writer.finish().unwrap()));
}
