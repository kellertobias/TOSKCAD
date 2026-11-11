use super::*;
#[test]
fn specialized_reader_keeps_generic_import_admission_separate() {
    let geometry = RoundedCylinder::new(5., 12., [Some(1.), Some(3.)], 1e-6, 1e7).unwrap();
    let text =
        crate::step::write_rounded_cylinder(&geometry, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
            .unwrap();
    assert!(read_rounded_cylinder(&text, 1e-7).is_ok());
    assert!(read_analytic_solid(&text, 1e-7).is_err());
    assert!(read_import_solid(&text, 1e-7).is_err());
}

fn specimen() -> String {
    crate::step::write_rounded_cylinder(
        &RoundedCylinder::new(5., 12., [Some(1.), Some(3.)], 1e-6, 1e7).unwrap(),
        [17., -8., 13.],
        [0.8, 0.6, 0.],
        [-0.36, 0.48, 0.8],
        1e-7,
    )
    .unwrap()
}
fn ids(source: &str, name: &str) -> Vec<u32> {
    parse_entities(source, 1e-7)
        .unwrap()
        .iter()
        .filter_map(|(&id, parts)| {
            matches!(parts.as_slice(),[entity] if entity.name==name).then_some(id)
        })
        .collect()
}
fn render(value: &Value<'_>) -> String {
    match value {
        Value::String(s) => format!("'{s}'"),
        Value::Enum(s) => format!(".{s}."),
        Value::Reference(id) => format!("#{id}"),
        Value::Number(n) => format!("{n:.16E}"),
        Value::List(values) => format!(
            "({})",
            values.iter().map(render).collect::<Vec<_>>().join(",")
        ),
        Value::Omitted => "$".into(),
        Value::Derived => "*".into(),
        Value::Typed(entity) => format!(
            "{}({})",
            entity.name,
            entity.args.iter().map(render).collect::<Vec<_>>().join(",")
        ),
    }
}
fn edit(source: &str, id: u32, change: impl FnOnce(&mut Entity<'_>)) -> String {
    let mut parsed = parse_entities(source, 1e-7).unwrap();
    let entity = &mut parsed.get_mut(&id).unwrap()[0];
    change(entity);
    let replacement = format!(
        "#{id}={}({});",
        entity.name,
        entity.args.iter().map(render).collect::<Vec<_>>().join(",")
    );
    let original = source
        .lines()
        .find(|line| line.starts_with(&format!("#{id}=")))
        .unwrap();
    source.replacen(original, &replacement, 1)
}
fn refused(source: &str) {
    assert!(
        read_rounded_cylinder(source, 1e-7).is_err(),
        "accepted malformed rounded BREP"
    );
}
#[test]
fn record_arity_types_missing_entities_and_unsupported_surfaces_are_rejected() {
    let source = specimen();
    for name in [
        "ADVANCED_FACE",
        "EDGE_LOOP",
        "ORIENTED_EDGE",
        "EDGE_CURVE",
        "CIRCLE",
        "SEAM_CURVE",
        "TOROIDAL_SURFACE",
        "CYLINDRICAL_SURFACE",
        "PLANE",
        "FACE_OUTER_BOUND",
        "CARTESIAN_POINT",
        "PCURVE",
        "LINE",
    ] {
        for id in ids(&source, name) {
            refused(&edit(&source, id, |e| e.args[0] = Value::Number(0.)));
        }
    }
    let face = ids(&source, "ADVANCED_FACE")[0];
    refused(&edit(&source, face, |e| {
        e.args[2] = Value::Reference(u32::MAX)
    }));
    let torus = ids(&source, "TOROIDAL_SURFACE")[0];
    refused(&edit(&source, torus, |e| e.name = "SPHERICAL_SURFACE"));
    assert_eq!(
        read_rounded_cylinder(&source, 0.).unwrap_err(),
        ReadError::Invalid
    );
    assert_eq!(
        read_rounded_cylinder_compound(&source, f64::NAN).unwrap_err(),
        ReadError::Invalid
    );
    assert!(read_rounded_cylinder(&source, 1e-7).is_ok());
}
#[test]
fn topology_counts_orientation_ownership_and_periodic_domains_are_strict() {
    let source = specimen();
    let shell = ids(&source, "CLOSED_SHELL")[0];
    for count in [1, 6] {
        let bad = edit(&source, shell, |e| {
            let Value::List(items) = &mut e.args[1] else {
                panic!()
            };
            let first = reference(&items[0]).unwrap();
            *items = (0..count).map(|_| Value::Reference(first)).collect();
        });
        assert_eq!(
            read_rounded_cylinder(&bad, 1e-7).unwrap_err(),
            if count == 6 {
                ReadError::Limit
            } else {
                ReadError::Unsupported
            }
        );
    }
    refused(&edit(&source, shell, |e| {
        let Value::List(items) = &mut e.args[1] else {
            panic!()
        };
        items[4] = Value::Reference(reference(&items[0]).unwrap());
    }));
    for index in [0, 2] {
        refused(&edit(&source, shell, |e| {
            let Value::List(items) = &mut e.args[1] else {
                panic!()
            };
            items.remove(index);
        }));
    }
    for id in ids(&source, "ADVANCED_FACE") {
        refused(&edit(&source, id, |e| e.args[3] = Value::Enum("F")));
    }
    for id in ids(&source, "EDGE_LOOP") {
        refused(&edit(&source, id, |e| e.args[1] = Value::List(vec![])));
        let bad = edit(&source, id, |e| {
            let Value::List(items) = &mut e.args[1] else {
                panic!()
            };
            let first = reference(&items[0]).unwrap();
            *items = (0..5).map(|_| Value::Reference(first)).collect();
        });
        assert_eq!(
            read_rounded_cylinder(&bad, 1e-7).unwrap_err(),
            ReadError::Limit
        );
    }
    for id in ids(&source, "ORIENTED_EDGE") {
        refused(&edit(&source, id, |e| {
            e.args[4] = Value::Enum(if sense(&e.args[4]).unwrap() { "F" } else { "T" });
        }));
    }
    for id in ids(&source, "EDGE_CURVE") {
        refused(&edit(&source, id, |e| e.args[4] = Value::Enum("F")));
    }
    let orphan = source
        .lines()
        .find(|l| l.starts_with(&format!("#{}=", ids(&source, "ORIENTED_EDGE")[0])))
        .unwrap();
    let orphan = orphan.replacen(
        &format!("#{}=", ids(&source, "ORIENTED_EDGE")[0]),
        "#1000000=",
        1,
    );
    refused(&source.replace(
        "ENDSEC;\nEND-ISO-10303-21;",
        &format!("{orphan}\nENDSEC;\nEND-ISO-10303-21;"),
    ));
    let g = RoundedCylinder::new(5., 12., [Some(1.), None], 1e-6, 1e7).unwrap();
    let mut writer = crate::step::Writer::new();
    for _ in 0..2 {
        writer
            .append_rounded_cylinder(&g, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
            .unwrap();
    }
    let compound = writer.finish().unwrap();
    let solids = ids(&compound, "MANIFOLD_SOLID_BREP");
    let original = parse_entities(&compound, 1e-7).unwrap();
    let shared = reference(&original[&solids[0]][0].args[1]).unwrap();
    let bad = edit(&compound, solids[1], |e| {
        e.args[1] = Value::Reference(shared)
    });
    assert_eq!(
        read_rounded_cylinder_compound(&bad, 1e-7).unwrap_err(),
        ReadError::Invalid
    );
    assert!(read_rounded_cylinder(&compound, 1e-7).is_err());
    assert_eq!(
        read_rounded_cylinder_compound(&compound, 1e-7)
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn radii_axes_shared_endpoints_and_pcurve_parameterization_cannot_be_forged() {
    let source = specimen();
    for name in ["CIRCLE", "TOROIDAL_SURFACE", "CYLINDRICAL_SURFACE"] {
        for id in ids(&source, name) {
            let parsed = parse_entities(&source, 1e-7).unwrap();
            let n = parsed[&id][0].args.len();
            for index in 2..n {
                for value in [-1., 0., 1e8] {
                    refused(&edit(&source, id, |e| e.args[index] = Value::Number(value)));
                }
            }
        }
    }
    for id in ids(&source, "SEAM_CURVE") {
        refused(&edit(&source, id, |e| e.args[3] = Value::Enum("CURVE_3D")));
        refused(&edit(&source, id, |e| {
            let Value::List(items) = &mut e.args[2] else {
                panic!()
            };
            items[1] = Value::Reference(reference(&items[0]).unwrap());
        }));
    }
    let parsed = parse_entities(&source, 1e-7).unwrap();
    for id in ids(&source, "PCURVE") {
        refused(&edit(&source, id, |e| {
            e.args[1] = Value::Reference(ids(&source, "PLANE")[0])
        }));
        let (_, curve) = Shell::new(&parsed, 1e-7, true)
            .pcurve_definition(id)
            .unwrap();
        let line = &parsed[&curve][0];
        let point = reference(&line.args[1]).unwrap();
        let vector = reference(&line.args[2]).unwrap();
        refused(&edit(&source, point, |e| {
            e.args[1] = Value::List(vec![Value::Number(0.), Value::Number(0.1)])
        }));
        refused(&edit(&source, vector, |e| e.args[2] = Value::Number(2.)));
        let direction = reference(&parsed[&vector][0].args[1]).unwrap();
        refused(&edit(&source, direction, |e| {
            e.args[1] = Value::List(vec![Value::Number(1.), Value::Number(0.)])
        }));
    }
    let point = ids(&source, "CARTESIAN_POINT")[0];
    assert_eq!(
        read_rounded_cylinder(
            &edit(&source, point, |e| e.args[1] =
                Value::List((0..3).map(|_| Value::Number(1e13)).collect())),
            1e-7
        )
        .unwrap_err(),
        ReadError::Limit
    );
}

fn append_entity(source: &str, id: u32, entity: &Entity<'_>) -> String {
    let record = format!(
        "#{id}={}({});\n",
        entity.name,
        entity.args.iter().map(render).collect::<Vec<_>>().join(",")
    );
    let at = source.rfind("ENDSEC;").unwrap();
    format!("{}{}{}", &source[..at], record, &source[at..])
}
fn clone_entity(source: &str, id: u32, change: impl FnOnce(&mut Entity<'_>)) -> (String, u32) {
    let mut parsed = parse_entities(source, 1e-7).unwrap();
    let next = parsed.keys().max().unwrap() + 1;
    let entity = &mut parsed.get_mut(&id).unwrap()[0];
    change(entity);
    (append_entity(source, next, entity), next)
}
fn args_ref(source: &str, id: u32, index: usize) -> u32 {
    reference(&parse_entities(source, 1e-7).unwrap()[&id][0].args[index]).unwrap()
}
fn coordinates(source: &str, point: u32) -> Point {
    let parsed = parse_entities(source, 1e-7).unwrap();
    numbers(&parsed[&point][0].args[1]).unwrap()
}
fn shift_point(source: &str, point: u32, delta: Point) -> String {
    edit(source, point, |e| {
        let Value::List(values) = &mut e.args[1] else {
            panic!()
        };
        for (value, offset) in values.iter_mut().zip(delta) {
            let Value::Number(n) = value else { panic!() };
            *n += offset;
        }
    })
}
fn rim_data(source: &str, index: usize) -> (u32, u32, u32, u32) {
    let circle = ids(source, "CIRCLE")[index];
    let parsed = parse_entities(source, 1e-7).unwrap();
    let axis = reference(&parsed[&circle][0].args[1]).unwrap();
    let center = reference(&parsed[&axis][0].args[1]).unwrap();
    let edge = ids(source, "EDGE_CURVE")
        .into_iter()
        .find(|id| reference(&parsed[id][0].args[3]).unwrap() == circle)
        .unwrap();
    let vertex = reference(&parsed[&edge][0].args[1]).unwrap();
    let point = reference(&parsed[&vertex][0].args[1]).unwrap();
    (circle, edge, center, point)
}
fn wire(source: &str, face: u32) -> (u32, u32, Vec<u32>) {
    let parsed = parse_entities(source, 1e-7).unwrap();
    let bound = refs(&parsed[&face][0].args[1]).unwrap()[0];
    let wire = reference(&parsed[&bound][0].args[1]).unwrap();
    (bound, wire, refs(&parsed[&wire][0].args[1]).unwrap())
}
#[test]
fn valid_cyclic_and_reversed_bound_encodings_keep_the_same_material() {
    let source = specimen();
    let expected = read_rounded_cylinder(&source, 1e-7).unwrap().0;
    for face in ids(&source, "ADVANCED_FACE") {
        let (bound, loop_id, uses) = wire(&source, face);
        let rotated = edit(&source, loop_id, |e| {
            let Value::List(items) = &mut e.args[1] else {
                panic!()
            };
            let count = items.len();
            items.rotate_left(2 % count);
        });
        assert_eq!(read_rounded_cylinder(&rotated, 1e-7).unwrap().0, expected);
        let mut reversed = source.clone();
        for usage in uses {
            reversed = edit(&reversed, usage, |e| {
                e.args[4] = Value::Enum(if sense(&e.args[4]).unwrap() { "F" } else { "T" });
            });
        }
        reversed = edit(&reversed, loop_id, |e| {
            let Value::List(items) = &mut e.args[1] else {
                panic!()
            };
            items.reverse();
        });
        reversed = edit(&reversed, bound, |e| e.args[2] = Value::Enum("F"));
        assert_eq!(read_rounded_cylinder(&reversed, 1e-7).unwrap().0, expected);
    }
    let swapped = source.replace(".PCURVE_S1.", ".PCURVE_S2.");
    assert_eq!(read_rounded_cylinder(&swapped, 1e-7).unwrap().0, expected);
    let cap = ids(&source, "ADVANCED_FACE")[0];
    let plane = args_ref(&source, cap, 2);
    let axis = args_ref(&source, plane, 1);
    let normal = args_ref(&source, axis, 2);
    let flipped = edit(&source, normal, |e| {
        let Value::List(items) = &mut e.args[1] else {
            panic!()
        };
        for item in items {
            let Value::Number(n) = item else { panic!() };
            *n = -*n;
        }
    });
    let flipped = edit(&flipped, cap, |e| e.args[3] = Value::Enum("F"));
    assert_eq!(read_rounded_cylinder(&flipped, 1e-7).unwrap().0, expected);
}
#[test]
fn displaced_planes_rims_axes_and_meridian_centers_are_rejected() {
    let source = specimen();
    let u = [0.8, 0.6, 0.];
    let normal = [0.48, -0.64, 0.6];
    let cap = ids(&source, "ADVANCED_FACE")[0];
    let plane = args_ref(&source, cap, 2);
    let axis = args_ref(&source, plane, 1);
    let origin = args_ref(&source, axis, 1);
    let point = coordinates(&source, origin);
    let (extra, new_point) = clone_entity(&source, origin, |e| {
        e.args[1] = Value::List(
            point
                .into_iter()
                .zip(normal)
                .map(|(p, n)| Value::Number(p + 0.1 * n))
                .collect(),
        )
    });
    let (extra, new_axis) = clone_entity(&extra, axis, |e| e.args[1] = Value::Reference(new_point));
    refused(&edit(&extra, plane, |e| {
        e.args[1] = Value::Reference(new_axis)
    }));
    let cylinder = ids(&source, "CYLINDRICAL_SURFACE")[0];
    let axis = args_ref(&source, cylinder, 1);
    let origin = args_ref(&source, axis, 1);
    let point = coordinates(&source, origin);
    let (extra, new_point) = clone_entity(&source, origin, |e| {
        e.args[1] = Value::List(
            point
                .into_iter()
                .zip(u)
                .map(|(p, n)| Value::Number(p + 0.1 * n))
                .collect(),
        )
    });
    let (extra, new_axis) = clone_entity(&extra, axis, |e| e.args[1] = Value::Reference(new_point));
    refused(&edit(&extra, cylinder, |e| {
        e.args[1] = Value::Reference(new_axis)
    }));
    let (_, _, center, point) = rim_data(&source, 3);
    let shifted = shift_point(&source, center, u.map(|v| v * 0.1));
    refused(&shift_point(&shifted, point, u.map(|v| v * 0.1)));
    let (circle, _, _, point) = rim_data(&source, 3);
    let altered = edit(&source, circle, |e| e.args[2] = Value::Number(4.9));
    refused(&shift_point(&altered, point, u.map(|v| -v * 0.1)));
    let (_, _, _, point) = rim_data(&source, 0);
    refused(&shift_point(&source, point, u.map(|v| v * 0.1)));
    let (_, _, center, point) = rim_data(&source, 2);
    let flat = shift_point(&source, center, normal.map(|v| -v * 12.));
    refused(&shift_point(&flat, point, normal.map(|v| -v * 12.)));
    let meridian = ids(&source, "CIRCLE")[4];
    let axis = args_ref(&source, meridian, 1);
    let center = args_ref(&source, axis, 1);
    refused(&shift_point(&source, center, u.map(|v| v * 0.1)));
    let wall_circle = ids(&source, "CIRCLE")[1];
    refused(&edit(&source, wall_circle, |e| e.name = "LINE"));
    let shell = ids(&source, "CLOSED_SHELL")[0];
    refused(&edit(&source, shell, |e| {
        let Value::List(items) = &mut e.args[1] else {
            panic!()
        };
        items.remove(3);
    }));
    // A complete periodic circle cannot be replaced by two turns in a cap loop.
    let (_, loop_id, uses) = wire(&source, cap);
    let (extra, new_use) = clone_entity(&source, uses[0], |_| {});
    refused(&edit(&extra, loop_id, |e| {
        e.args[1] = Value::List(vec![Value::Reference(uses[0]), Value::Reference(new_use)])
    }));
    let g = RoundedCylinder::new(5., 12., [Some(1.), None], 1e-6, 1e7).unwrap();
    let distant =
        crate::step::write_rounded_cylinder(&g, [1e10, 0., 0.], [1., 0., 0.], [0., 1., 0.], 1e-3)
            .unwrap();
    assert_eq!(
        read_rounded_cylinder(&distant, 1e-7).unwrap_err(),
        ReadError::Unsupported
    );
    assert!(read_rounded_cylinder(&distant, 1e-3).is_ok());
}

#[test]
fn a_patch_cannot_be_reused_at_both_ends_or_hidden_outside_the_capped_material() {
    let geometry = RoundedCylinder::new(5., 12., [Some(1.), None], 1e-6, 1e7).unwrap();
    let source =
        crate::step::write_rounded_cylinder(&geometry, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
            .unwrap();
    let faces = ids(&source, "ADVANCED_FACE");
    let (_, low_edge, low_center, _) = rim_data(&source, 1);
    let end_plane = args_ref(&source, faces[1], 2);
    let axis = args_ref(&source, end_plane, 1);
    let altered = edit(&source, axis, |e| e.args[1] = Value::Reference(low_center));
    let (_, _, uses) = wire(&source, faces[1]);
    let altered = edit(&altered, uses[0], |e| {
        e.args[3] = Value::Reference(low_edge)
    });
    refused(&altered);
    // An extra toroidal face follows the cylinder wire but does not bound the
    // material. Its entirely owned topology must not become a hidden face.
    let (wall_bound, wall_loop, wall_uses) = wire(&source, faces[2]);
    let mut extra = source.clone();
    let mut new_uses = Vec::new();
    for usage in wall_uses {
        let (updated, id) = clone_entity(&extra, usage, |_| {});
        extra = updated;
        new_uses.push(Value::Reference(id));
    }
    let (updated, new_loop) =
        clone_entity(&extra, wall_loop, |e| e.args[1] = Value::List(new_uses));
    extra = updated;
    let (updated, new_bound) = clone_entity(&extra, wall_bound, |e| {
        e.args[1] = Value::Reference(new_loop)
    });
    extra = updated;
    let support = args_ref(&source, faces[3], 2);
    let (updated, new_face) = clone_entity(&extra, faces[2], |e| {
        e.args[1] = Value::List(vec![Value::Reference(new_bound)]);
        e.args[2] = Value::Reference(support);
    });
    extra = updated;
    let shell = ids(&source, "CLOSED_SHELL")[0];
    refused(&edit(&extra, shell, |e| {
        let Value::List(items) = &mut e.args[1] else {
            panic!()
        };
        items.push(Value::Reference(new_face));
    }));
}

fn decoded_faces(shell: &Shell<'_, '_>, source: &str) -> Vec<topology::FaceUse> {
    let mut seen = BTreeSet::new();
    ids(source, "ADVANCED_FACE")
        .into_iter()
        .map(|id| topology::face(shell, id, &mut seen).unwrap())
        .collect()
}
#[test]
fn closed_rim_and_periodic_decoders_independently_reject_open_or_mismatched_trims() {
    let source = specimen();
    let (circle, edge, center, _) = rim_data(&source, 0);
    let parsed = parse_entities(&source, 1e-7).unwrap();
    let axis = reference(&parsed[&circle][0].args[1]).unwrap();
    let shell = Shell::new(&parsed, 1e-7, true);
    let plane = shell.placement(axis).unwrap();
    let radius = match parsed[&circle][0].args[2] {
        Value::Number(r) => r,
        _ => panic!(),
    };
    let (extra, point) = clone_entity(&source, center, |e| {
        e.args[1] = Value::List(
            std::array::from_fn::<_, 3, _>(|i| {
                Value::Number(plane.origin[i] + radius * plane.v[i])
            })
            .into_iter()
            .collect(),
        )
    });
    let vertex = args_ref(&source, edge, 1);
    let (extra, new_vertex) = clone_entity(&extra, vertex, |e| e.args[1] = Value::Reference(point));
    let extra = edit(&extra, edge, |e| e.args[2] = Value::Reference(new_vertex));
    let entities = parse_entities(&extra, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    assert!(matches!(
        topology::rim(&mut shell, edge),
        Err(ReadError::Invalid)
    ));
    let entities = parse_entities(&source, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    let faces = decoded_faces(&shell, &source);
    let (_, low, _, _) = rim_data(&source, 1);
    let (_, high, _, _) = rim_data(&source, 3);
    assert_eq!(
        topology::periodic(&mut shell, &faces[0], low, high, None).unwrap_err(),
        ReadError::Invalid
    );
    assert_eq!(
        topology::periodic(&mut shell, &faces[2], low, low, None).unwrap_err(),
        ReadError::Invalid
    );
    let seam = faces[2].wire[1].edge;
    let wrong = edit(&source, seam, |e| {
        e.args[1] = Value::Reference(args_ref(&source, high, 1))
    });
    let entities = parse_entities(&wrong, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    assert_eq!(
        topology::periodic(&mut shell, &faces[2], low, high, None).unwrap_err(),
        ReadError::Invalid
    );
}
#[test]
fn incidence_rejects_missing_duplicate_same_sense_and_unowned_uses() {
    let source = specimen();
    let entities = parse_entities(&source, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    for defect in 0..4 {
        let mut faces = decoded_faces(&shell, &source);
        let rims = (0..4)
            .map(|i| rim_data(&source, i).1)
            .collect::<BTreeSet<_>>();
        let seams = [2, 3, 4]
            .into_iter()
            .map(|i| (faces[i].id, faces[i].wire[1].edge))
            .collect::<BTreeMap<_, _>>();
        let mut declared = rims.clone();
        let mut seams = seams;
        match defect {
            0 => {
                declared.remove(&rim_data(&source, 0).1);
            }
            1 => {
                let edge = faces[0].wire[0].edge;
                faces[0].wire.push(topology::WireUse {
                    edge,
                    forward: false,
                });
            }
            2 => {
                faces[0].wire[0].forward = true;
            }
            _ => {
                let first = faces[2].id;
                seams.insert(first, faces[3].wire[1].edge);
            }
        }
        assert_eq!(
            topology::incidence(&faces, &declared, &seams).unwrap_err(),
            ReadError::Invalid
        );
    }
}

#[test]
fn repeated_oriented_usage_and_mixed_member_publications_fail_atomically() {
    let source = specimen();
    let id = ids(&source, "EDGE_LOOP")[0];
    refused(&edit(&source, id, |e| {
        let Value::List(items) = &mut e.args[1] else {
            panic!()
        };
        let first = reference(&items[0]).unwrap();
        items.push(Value::Reference(first));
    }));
    let g = RoundedCylinder::new(5., 12., [Some(1.), None], 1e-6, 1e7).unwrap();
    let mut writer = crate::step::Writer::new();
    writer
        .append_rounded_cylinder(&g, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    writer
        .append_sphere(
            crate::sphere::Sphere::new(2.).unwrap(),
            [20., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
        )
        .unwrap();
    assert_eq!(
        read_rounded_cylinder_compound(&writer.finish().unwrap(), 1e-7).unwrap_err(),
        ReadError::Unsupported
    );
}
