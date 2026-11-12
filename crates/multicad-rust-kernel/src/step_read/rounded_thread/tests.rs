use super::*;
use crate::step::write_rounded_thread;
fn value(v: &Value<'_>) -> String {
    match v {
        Value::String(s) => format!("'{s}'"),
        Value::Enum(s) => format!(".{s}."),
        Value::Reference(id) => format!("#{id}"),
        Value::Number(n) => format!("{n:.16E}"),
        Value::List(v) => format!("({})", v.iter().map(value).collect::<Vec<_>>().join(",")),
        Value::Omitted => "$".into(),
        Value::Derived => "*".into(),
        Value::Typed(e) => entity(e),
    }
}
fn entity(e: &Entity<'_>) -> String {
    format!(
        "{}({})",
        e.name,
        e.args.iter().map(value).collect::<Vec<_>>().join(",")
    )
}
fn rewrite<'a>(file: &'a str, edit: impl FnOnce(&mut BTreeMap<u32, Vec<Entity<'a>>>)) -> String {
    let mut entities = part21::parse(file).unwrap();
    edit(&mut entities);
    let data = file.find("DATA;").unwrap() + 5;
    let end = file.rfind("ENDSEC;").unwrap();
    let body = entities
        .iter()
        .map(|(id, parts)| {
            let body = if parts.len() == 1 {
                entity(&parts[0])
            } else {
                format!(
                    "({})",
                    parts.iter().map(entity).collect::<Vec<_>>().join(" ")
                )
            };
            format!("#{id}={body};")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n{body}\n{}", &file[..data], &file[end..])
}
fn nth<'m, 's>(
    map: &'m mut BTreeMap<u32, Vec<Entity<'s>>>,
    name: &str,
    index: usize,
) -> &'m mut Entity<'s> {
    // The caller's document owns all borrowed token spellings; only static
    // schema names and numeric/reference values are introduced below.
    map.values_mut()
        .filter_map(|v| v.first_mut())
        .filter(|e| e.name == name)
        .nth(index)
        .unwrap()
}
#[test]
fn malformed_finite_face_graph_and_support_controls_refuse_without_publishing() {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    let t = Thread::new(crate::rounded_thread::Spec {
        major_radius_mm: 3.175,
        pitch_mm: p,
        length_mm: 3.7 * p,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    })
    .unwrap();
    let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = write_rounded_thread(&t, frame[0], frame[1], frame[2], 1e-7).unwrap();
    for case in 0..12 {
        let bad = rewrite(&file, |map| match case {
            0 => nth(map, "B_SPLINE_SURFACE_WITH_KNOTS", 0).args[3] = Value::Number(0.),
            1 => {
                nth(map, "B_SPLINE_SURFACE_WITH_KNOTS", 0).args.pop();
            }
            2 => nth(map, "PLANE", 0).args.push(Value::Number(0.)),
            3 => {
                let f = nth(map, "ADVANCED_FACE", 0);
                let Value::List(v) = &mut f.args[1] else {
                    unreachable!()
                };
                let id = reference(&v[0]).unwrap();
                v.push(Value::Reference(id));
            }
            4 => {
                let f = nth(map, "ADVANCED_FACE", 5);
                let Value::List(v) = &mut f.args[1] else {
                    unreachable!()
                };
                let id = reference(&v[0]).unwrap();
                v.push(Value::Reference(id));
            }
            5 => {
                let e = nth(map, "EDGE_LOOP", 0);
                let Value::List(v) = &mut e.args[1] else {
                    unreachable!()
                };
                let id = reference(&v[0]).unwrap();
                v.push(Value::Reference(id));
            }
            6 => nth(map, "B_SPLINE_CURVE_WITH_KNOTS", 5).args[1] = Value::Number(10.),
            7 => {
                let axis = reference(&nth(map, "PLANE", 0).args[1]).unwrap();
                let id = map.keys().last().copied().unwrap() + 1;
                map.insert(
                    id,
                    vec![Entity {
                        name: "DIRECTION",
                        args: vec![
                            Value::String(""),
                            Value::List([0., 1., 0.].map(Value::Number).into()),
                        ],
                    }],
                );
                map.get_mut(&axis).unwrap()[0].args[3] = Value::Reference(id);
            }
            8 => {
                let axes = [
                    reference(&nth(map, "PLANE", 0).args[1]).unwrap(),
                    reference(&nth(map, "PLANE", 1).args[1]).unwrap(),
                ];
                for axis in axes {
                    let id = map.keys().last().copied().unwrap() + 1;
                    map.insert(
                        id,
                        vec![Entity {
                            name: "CARTESIAN_POINT",
                            args: vec![
                                Value::String(""),
                                Value::List([0., 0., 1.].map(Value::Number).into()),
                            ],
                        }],
                    );
                    map.get_mut(&axis).unwrap()[0].args[1] = Value::Reference(id);
                }
            }
            9 => {
                let e = nth(map, "B_SPLINE_SURFACE_WITH_KNOTS", 0);
                let Value::List(rows) = &mut e.args[3] else {
                    unreachable!()
                };
                let Value::List(columns) = &mut rows[0] else {
                    unreachable!()
                };
                columns.pop();
            }
            10 => {
                let e = nth(map, "B_SPLINE_SURFACE_WITH_KNOTS", 0);
                let Value::List(rows) = &mut e.args[3] else {
                    unreachable!()
                };
                rows.pop();
            }
            11 => {
                let axis = reference(&nth(map, "PLANE", 1).args[1]).unwrap();
                let old = reference(&map[&axis][0].args[1]).unwrap();
                let Value::List(coords) = &map[&old][0].args[1] else {
                    unreachable!()
                };
                let mut point = coords
                    .iter()
                    .map(|v| {
                        let Value::Number(n) = v else { unreachable!() };
                        *n
                    })
                    .collect::<Vec<_>>();
                point[0] += 0.1;
                let id = map.keys().last().copied().unwrap() + 1;
                map.insert(
                    id,
                    vec![Entity {
                        name: "CARTESIAN_POINT",
                        args: vec![
                            Value::String(""),
                            Value::List(point.into_iter().map(Value::Number).collect()),
                        ],
                    }],
                );
                map.get_mut(&axis).unwrap()[0].args[1] = Value::Reference(id);
            }
            _ => unreachable!(),
        });
        assert!(
            verify_rounded_thread(&bad, &t, frame[0], frame[1], frame[2], 1e-7).is_err(),
            "case{case}"
        );
        assert!(read_rounded_thread(&bad, 1e-7).is_err(), "case{case}");
    }
    read_rounded_thread(&file, 1e-7).unwrap();
}

#[test]
fn untrusted_tensor_size_and_unproved_reparameterizations_fail_with_typed_limits() {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    let t = Thread::new(crate::rounded_thread::Spec {
        major_radius_mm: 3.175,
        pitch_mm: p,
        length_mm: 3.7 * p,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    })
    .unwrap();
    let file = write_rounded_thread(&t, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7).unwrap();
    for (rows, columns) in [(11276, 12), (56, 11001)] {
        let bad = rewrite(&file, |map| {
            let surface = nth(map, "B_SPLINE_SURFACE_WITH_KNOTS", 0);
            let Value::List(existing) = &surface.args[3] else {
                unreachable!()
            };
            let ids = refs(&existing[0]).unwrap();
            surface.args[3] = Value::List(
                (0..rows)
                    .map(|_| {
                        Value::List(
                            (0..columns)
                                .map(|i| Value::Reference(ids[i % ids.len()]))
                                .collect(),
                        )
                    })
                    .collect(),
            );
        });
        assert_eq!(
            read_rounded_thread(&bad, 1e-7).unwrap_err(),
            ReadError::Limit,
            "{rows}/{columns}"
        );
    }
    let excessive_turns = rewrite(&file, |map| {
        for index in 0..5 {
            let s = nth(map, "B_SPLINE_SURFACE_WITH_KNOTS", index);
            let Value::List(knots) = &mut s.args[11] else {
                unreachable!()
            };
            for knot in knots {
                let Value::Number(k) = knot else {
                    unreachable!()
                };
                *k *= 100.;
            }
        }
    });
    assert_eq!(
        read_rounded_thread(&excessive_turns, 1e-7).unwrap_err(),
        ReadError::Limit
    );
    let invalid_profile = rewrite(&file, |map| {
        let s = nth(map, "B_SPLINE_SURFACE_WITH_KNOTS", 0);
        let Value::List(knots) = &mut s.args[10] else {
            unreachable!()
        };
        for knot in knots {
            let Value::Number(k) = knot else {
                unreachable!()
            };
            *k *= 0.1;
        }
    });
    assert_eq!(
        read_rounded_thread(&invalid_profile, 1e-7).unwrap_err(),
        ReadError::Invalid
    );
    let open_shell = rewrite(&file, |map| {
        let e = nth(map, "CLOSED_SHELL", 0);
        let Value::List(faces) = &mut e.args[1] else {
            unreachable!()
        };
        faces.pop();
    });
    assert_eq!(
        read_rounded_thread(&open_shell, 1e-7).unwrap_err(),
        ReadError::Unsupported
    );
    read_rounded_thread(&file, 1e-7).unwrap();
}
