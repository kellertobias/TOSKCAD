use super::*;
use crate::{prism::Location, step::write_helical_tube_cut};

const TOL: f64 = 1e-7;
const IDENTITY: [Point; 3] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];

fn specimen(start: f64, phase: f64, [origin, u, v]: [Point; 3]) -> (Cut, String) {
    let cut = Cut::new(
        Spec {
            stock_radius_mm: 8.,
            stock_height_mm: 30.,
            tube_radius_mm: 0.6,
            pitch_mm: 2.,
            path_height_mm: 15.,
            start_height_mm: start,
            start_angle_rad: phase,
        },
        TOL,
    )
    .unwrap();
    let text = write_helical_tube_cut(cut, origin, u, v, TOL).unwrap();
    (cut, text)
}

#[test]
fn reads_complete_helical_shell_in_distinct_frames_and_periodic_lifts() {
    for (start, phase, frame) in [
        (16.2, 0., IDENTITY),
        (16., -0.1, IDENTITY),
        (16.6, 0.1, IDENTITY),
        (16.2, TAU, IDENTITY),
        (16.2, -TAU, IDENTITY),
        (16.2, FRAC_PI_2, [[4., 5., 6.], [1., 0., 0.], [0., 0., 1.]]),
        (16.2, 0., [[17., 4., -12.], [0.8, 0., -0.6], [0., 1., 0.]]),
    ] {
        let (cut, source) = specimen(start, phase, frame);
        let (proof, body) = read_import_solid(&source, TOL).unwrap();
        assert_eq!(read_analytic_solid(&source, TOL).unwrap(), proof);
        assert_eq!(proof.solid_count, 1);
        assert_eq!(proof.face_count, 5);
        assert!((proof.volume_mm3 - cut.volume_mm3()).abs() < 1e-8);
        assert_eq!([body.origin_mm, body.u, body.v], frame);
        let n = cross(frame[1], frame[2]);
        for i in 0..3 {
            let direction = [frame[1][i], frame[2][i], n[i]];
            let min = cut.support_upper_mm(direction.map(|x| -x)).unwrap();
            let max = cut.support_upper_mm(direction).unwrap();
            assert!((proof.bounds_mm[0][i] - (frame[0][i] - min)).abs() < TOL);
            assert!((proof.bounds_mm[1][i] - (frame[0][i] + max)).abs() < TOL);
        }
        let ImportProfile::HelicalTubeCut {
            stock_radius_mm,
            radius_mm,
            pitch_mm,
            path_height_mm,
            start_height_mm,
            start_angle_rad,
        } = body.profile
        else {
            panic!("expected editable helical cut");
        };
        let imported = Cut::new(
            Spec {
                stock_radius_mm,
                stock_height_mm: body.height_mm,
                tube_radius_mm: radius_mm,
                pitch_mm,
                path_height_mm,
                start_height_mm,
                start_angle_rad,
            },
            TOL,
        )
        .unwrap();
        assert!(path_height_mm < cut.spec().path_height_mm);
        for theta in [TAU - 0.01, TAU + 0.01] {
            let [sine, cosine] = [(theta + phase).sin(), (theta + phase).cos()];
            for (r, expected) in [
                (7.7, Location::Exterior),
                (7.4, Location::Boundary),
                (7.2, Location::Interior),
            ] {
                let point = [r * cosine, r * sine, start + 2. * theta / TAU];
                assert_eq!(imported.classify(point), Ok(expected));
            }
        }
    }
}

fn record(text: &str, id: u32) -> &str {
    let prefix = format!("#{id}=");
    text.lines().find(|line| line.starts_with(&prefix)).unwrap()
}

fn changed(text: &str, id: u32, change: impl FnOnce(&str) -> String) -> String {
    let original = record(text, id);
    let edited = change(original);
    assert_ne!(original, edited);
    text.replacen(original, &edited, 1)
}

fn corrupt_point(text: &str, id: u32) -> String {
    changed(text, id, |original| {
        let prefix = original.find("('',(").unwrap() + 5;
        let comma = original[prefix..].find(',').unwrap() + prefix;
        format!("{}9.123E5{}", &original[..prefix], &original[comma..])
    })
}

#[test]
fn rejects_changed_incidence_spline_controls_and_parameter_curves() {
    let (_, source) = specimen(16.2, 0., IDENTITY);
    let records = parse_entities(&source, TOL).unwrap();
    let shell = Shell::new(&records, TOL, true);
    let faces = shell
        .member_faces(shell.represented_solids(true).unwrap()[0])
        .unwrap();
    let named: Vec<_> = faces.iter().map(|&id| face(&shell, id).unwrap()).collect();
    let wires = std::array::from_fn(|i| ring(&shell, named[i].2[0], true).unwrap());
    let edges = incidence(&shell, &wires).unwrap();
    let [_, top, cylinder, cavity, _] = std::array::from_fn(|i| named[i].0);

    let (top_curve, [cavity_uv, planar_uv]) = decode::wrapper(
        &shell,
        edges[TOP_CAVITY],
        "SURFACE_CURVE",
        "CURVE_3D",
        [cavity, top],
    )
    .unwrap();
    let (rail, [rail_uv, _]) = decode::wrapper(
        &shell,
        edges[POSITIVE],
        "SURFACE_CURVE",
        "PCURVE_S1",
        [cylinder, cavity],
    )
    .unwrap();
    let (_, [floor_uv]) = decode::wrapper(
        &shell,
        edges[FLOOR_CYLINDER],
        "SURFACE_CURVE",
        "CURVE_3D",
        [cylinder],
    )
    .unwrap();
    let (_, [_, seam_left]) = decode::wrapper(
        &shell,
        edges[SEAM],
        "SEAM_CURVE",
        "PCURVE_S1",
        [cylinder; 2],
    )
    .unwrap();

    let control =
        |curve| refs(&shell.args(curve, "B_SPLINE_CURVE_WITH_KNOTS").unwrap()[2]).unwrap()[1];
    let Value::List(net) = &shell.args(cavity, "B_SPLINE_SURFACE_WITH_KNOTS").unwrap()[3] else {
        panic!()
    };
    let surface_control = refs(&net[1]).unwrap()[1];
    for point in [
        control(top_curve),
        control(cavity_uv),
        control(planar_uv),
        control(rail),
        control(rail_uv),
        control(floor_uv),
        control(seam_left),
        surface_control,
    ] {
        assert!(
            read_import_solid(&corrupt_point(&source, point), TOL).is_err(),
            "control #{point}"
        );
    }

    let wrong_face = changed(&source, faces[3], |record| record.replace(".F.", ".T."));
    assert_eq!(read_import_solid(&wrong_face, TOL), Err(ReadError::Invalid));
    let wrong_sense = changed(&source, wires[2][3].0, |record| {
        record.replace(".T.", ".F.")
    });
    assert_eq!(
        read_import_solid(&wrong_sense, TOL),
        Err(ReadError::Invalid)
    );
    let wrong_master = changed(&source, edges[SEAM].geometry, |record| {
        record.replace(".PCURVE_S1.", ".CURVE_3D.")
    });
    assert_eq!(
        read_import_solid(&wrong_master, TOL),
        Err(ReadError::Invalid)
    );
}

fn inspect(shell: &Shell<'_, '_>) -> ([u32; 5], [u32; 5], [Vec<EdgeUse>; 5], [Edge; 9]) {
    let faces: [u32; 5] = shell
        .member_faces(shell.represented_solids(true).unwrap()[0])
        .unwrap()
        .try_into()
        .unwrap();
    let named = faces.map(|id| face(shell, id).unwrap());
    let wires = std::array::from_fn(|i| ring(shell, named[i].2[0], true).unwrap());
    let edges = incidence(shell, &wires).unwrap();
    (faces, named.map(|value| value.0), wires, edges)
}

fn ref_arg(shell: &Shell<'_, '_>, id: u32, index: usize) -> u32 {
    reference(&shell.simple(id).unwrap().args[index]).unwrap()
}

fn mutate(
    source: &str,
    id: u32,
    edit: impl FnOnce(&mut Entity<'_>),
    check: impl FnOnce(&Shell<'_, '_>),
) {
    let mut entities = parse_entities(source, TOL).unwrap();
    edit(&mut entities.get_mut(&id).unwrap()[0]);
    check(&Shell::new(&entities, TOL, true));
}

#[test]
fn refuses_missing_bounds_and_each_wrong_oriented_wire_or_vertex_join() {
    let (_, source) = specimen(16.2, 0., IDENTITY);
    let entities = parse_entities(&source, TOL).unwrap();
    let shell = Shell::new(&entities, TOL, true);
    let (faces, surfaces, wires, _) = inspect(&shell);
    for (face, index) in [(0, 0), (1, 1), (4, 1), (2, 1), (2, 7), (3, 3)] {
        let mut broken = wires.clone();
        broken[face][index].1 = !broken[face][index].1;
        assert!(incidence(&shell, &broken).is_err());
    }
    let without_bounds = |record: &mut Entity<'_>| record.args[1] = Value::List(vec![]);
    mutate(&source, faces[3], without_bounds, |shell| {
        assert!(read_member(shell, &faces).is_err());
    });
    mutate(
        &source,
        surfaces[3],
        |record| record.name = "PLANE",
        |shell| {
            assert!(read_member(shell, &faces).unwrap().is_none());
        },
    );
    let p0 = ref_arg(&shell, wires[2][3].0, 1);
    let p1 = ref_arg(&shell, wires[2][3].0, 2);
    let n0 = ref_arg(&shell, wires[2][5].0, 1);
    let bottom = wires[0][0].0;
    for (edge, point) in [(wires[2][3].0, n0), (bottom, p1)] {
        mutate(
            &source,
            edge,
            |record| record.args[1] = Value::Reference(point),
            |shell| {
                assert!(incidence(shell, &wires).is_err());
            },
        );
    }
    mutate(
        &source,
        p0,
        |record| record.args.pop().map(drop).unwrap(),
        |shell| {
            assert!(read_member(shell, &faces).is_err());
        },
    );
}

#[test]
fn refuses_different_stock_and_floor_supports_or_chart_endpoints() {
    let (cut, source) = specimen(16.2, 0., IDENTITY);
    let entities = parse_entities(&source, TOL).unwrap();
    let shell = Shell::new(&entities, TOL, true);
    let (faces, [bottom, top, cylinder, cavity, _], _, edges) = inspect(&shell);
    let reject = |shell: &Shell<'_, '_>| assert!(read_member(shell, &faces).is_err());
    let placement = |surface| ref_arg(&shell, surface, 1);
    let origin = |surface| ref_arg(&shell, placement(surface), 1);
    let normal = |surface| ref_arg(&shell, placement(surface), 2);
    let vector = |record: &mut Entity<'_>, components: &[f64]| {
        record.args[1] = Value::List(components.iter().copied().map(Value::Number).collect());
    };
    mutate(
        &source,
        cylinder,
        |record| record.args.pop().map(drop).unwrap(),
        reject,
    );
    mutate(
        &source,
        origin(cylinder),
        |record| vector(record, &[1e8, 0., 0.]),
        |shell| {
            assert_eq!(read_member(shell, &faces), Err(ReadError::Limit));
        },
    );
    mutate(
        &source,
        origin(top),
        |record| vector(record, &[1., 0., 30.]),
        reject,
    );
    mutate(
        &source,
        normal(top),
        |record| vector(record, &[0., 1., 1.]),
        reject,
    );
    mutate(
        &source,
        normal(bottom),
        |record| vector(record, &[0., 1., -1.]),
        reject,
    );
    mutate(
        &source,
        edges[FLOOR_RIM].geometry,
        |record| record.args[3] = Value::Enum("PCURVE_S1"),
        reject,
    );

    let (circle, [_]) = decode::wrapper(
        &shell,
        edges[BOTTOM],
        "SURFACE_CURVE",
        "CURVE_3D",
        [cylinder],
    )
    .unwrap();
    mutate(
        &source,
        origin(circle),
        |record| vector(record, &[1., 0., 0.]),
        reject,
    );
    mutate(
        &source,
        normal(circle),
        |record| vector(record, &[0., 1., 1.]),
        reject,
    );
    let frame = Frame::from(shell.placement(placement(cylinder)).unwrap());
    let spec = cut.spec();
    let rise = spec.pitch_mm / TAU;
    let raw = spec.start_angle_rad + PI - spec.start_height_mm / rise;
    let seam_start = raw.rem_euclid(TAU);
    let seam = [seam_start, seam_start + spec.stock_height_mm / rise];
    let verifier = Verification {
        frame,
        cut,
        lift: seam_start - raw,
        geometry: TOL / 8.,
        angle: TOL / (8. * 31.),
        model: TOL * 0.75,
    };
    mutate(
        &source,
        origin(circle),
        |record| vector(record, &[1., 0., 0.]),
        |shell| {
            assert!(
                verifier
                    .stock_rims(
                        shell,
                        [edges[BOTTOM], edges[TOP_LO], edges[TOP_HI]],
                        cylinder,
                        seam,
                    )
                    .is_err()
            );
        },
    );
    let circle_u = ref_arg(&shell, placement(circle), 3);
    mutate(
        &source,
        circle_u,
        |record| vector(record, &[1., 0., 0.]),
        reject,
    );

    let (ellipse, [ellipse_uv]) = decode::wrapper(
        &shell,
        edges[FLOOR_CYLINDER],
        "SURFACE_CURVE",
        "CURVE_3D",
        [cylinder],
    )
    .unwrap();
    mutate(
        &source,
        ellipse,
        |record| record.args.pop().map(drop).unwrap(),
        reject,
    );
    mutate(
        &source,
        ellipse,
        |record| record.args[2] = Value::Number(8.),
        reject,
    );
    mutate(
        &source,
        origin(ellipse),
        |record| vector(record, &[0., 1., 16.2]),
        reject,
    );
    mutate(
        &source,
        normal(ellipse),
        |record| vector(record, &[1., 1., 0.]),
        reject,
    );
    let reversed = shell
        .direction(normal(ellipse))
        .unwrap()
        .map(|value| -value);
    mutate(
        &source,
        normal(ellipse),
        |record| vector(record, &reversed),
        |shell| {
            assert!(
                verifier
                    .floor_cylinder(shell, edges[FLOOR_CYLINDER], cylinder)
                    .is_err()
            )
        },
    );
    mutate(
        &source,
        ellipse_uv,
        |record| {
            let Value::List(knots) = &mut record.args[7] else {
                panic!()
            };
            knots[0] = Value::Number(-2.);
        },
        reject,
    );
    mutate(
        &source,
        ellipse_uv,
        |record| {
            let Value::List(multiplicities) = &mut record.args[6] else {
                panic!()
            };
            multiplicities[0] = Value::Number(11.);
        },
        reject,
    );

    let (_, [rail, wall]) = decode::wrapper(
        &shell,
        edges[NEGATIVE],
        "SURFACE_CURVE",
        "PCURVE_S1",
        [cylinder, cavity],
    )
    .unwrap();
    let control =
        |curve| refs(&shell.args(curve, "B_SPLINE_CURVE_WITH_KNOTS").unwrap()[2]).unwrap()[1];
    mutate(
        &source,
        control(rail),
        |record| {
            let Value::List(point) = &mut record.args[1] else {
                panic!()
            };
            point[1] = Value::Number(28.);
        },
        reject,
    );
    mutate(
        &source,
        control(wall),
        |record| {
            let Value::List(point) = &mut record.args[1] else {
                panic!()
            };
            point[1] = Value::Number(28.);
        },
        reject,
    );
    let (_, [seam, _]) = decode::wrapper(
        &shell,
        edges[SEAM],
        "SEAM_CURVE",
        "PCURVE_S1",
        [cylinder; 2],
    )
    .unwrap();
    mutate(
        &source,
        control(seam),
        |record| {
            let Value::List(point) = &mut record.args[1] else {
                panic!()
            };
            point[1] = Value::Number(28.);
        },
        reject,
    );
    let (_, [cap_uv, _]) = decode::wrapper(
        &shell,
        edges[TOP_CAVITY],
        "SURFACE_CURVE",
        "CURVE_3D",
        [cavity, top],
    )
    .unwrap();
    mutate(
        &source,
        cap_uv,
        |record| record.args[2] = Value::List(vec![]),
        reject,
    );
    assert_eq!(
        align_domain(&mut [], [0., 1.], TOL),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        cut_error(crate::helical_tube_cut::Error::InvalidInput),
        ReadError::Invalid
    );
}

#[test]
fn decoded_records_check_shape_degree_coordinates_and_control_budget() {
    let (_, source) = specimen(16.2, 0., IDENTITY);
    let entities = parse_entities(&source, TOL).unwrap();
    let shell = Shell::new(&entities, TOL, true);
    let (_, [bottom, _, cylinder, cavity, _], _, edges) = inspect(&shell);
    let frame = decode::plane(&shell, bottom).unwrap();
    let (circle, [uv]) = decode::wrapper(
        &shell,
        edges[BOTTOM],
        "SURFACE_CURVE",
        "CURVE_3D",
        [cylinder],
    )
    .unwrap();
    let control = refs(&shell.args(uv, "B_SPLINE_CURVE_WITH_KNOTS").unwrap()[2]).unwrap()[0];

    mutate(
        &source,
        edges[BOTTOM].geometry,
        |record| record.args.pop().map(drop).unwrap(),
        |shell| {
            assert!(
                decode::wrapper(
                    shell,
                    edges[BOTTOM],
                    "SURFACE_CURVE",
                    "CURVE_3D",
                    [cylinder]
                )
                .is_err()
            );
        },
    );
    mutate(
        &source,
        circle,
        |record| record.args.pop().map(drop).unwrap(),
        |shell| {
            assert!(decode::circle(shell, circle).is_err());
        },
    );
    mutate(
        &source,
        bottom,
        |record| record.args.pop().map(drop).unwrap(),
        |shell| {
            assert!(decode::plane(shell, bottom).is_err());
        },
    );
    let bad_curve = |shell: &Shell<'_, '_>| assert!(decode::curve::<2>(shell, uv).is_err());
    for (index, replacement) in [
        (1, Value::String("bad degree")),
        (1, Value::Number(2.5)),
        (4, Value::Enum("T")),
        (6, Value::Omitted),
        (
            7,
            Value::List(vec![Value::String("bad knot"), Value::Number(1.)]),
        ),
    ] {
        mutate(
            &source,
            uv,
            |record| record.args[index] = replacement,
            bad_curve,
        );
    }
    for coords in [
        Value::Omitted,
        Value::List(vec![Value::Omitted, Value::Number(0.)]),
        Value::List(vec![Value::Number(1e13), Value::Number(0.)]),
    ] {
        mutate(
            &source,
            control,
            |record| record.args[1] = coords,
            bad_curve,
        );
    }
    mutate(
        &source,
        control,
        |record| record.args.pop().map(drop).unwrap(),
        bad_curve,
    );

    for (index, replacement) in [(5, Value::Enum("T")), (3, Value::Omitted)] {
        mutate(
            &source,
            cavity,
            |record| record.args[index] = replacement,
            |shell| {
                assert!(decode::surface(shell, cavity, frame).is_err());
            },
        );
    }
    mutate(
        &source,
        cavity,
        |record| {
            for (i, j) in [(8, 10), (9, 11)] {
                record.args[i] = Value::List((0..1025).map(|_| Value::Number(11.)).collect());
                record.args[j] =
                    Value::List((0..1025).map(|k| Value::Number(f64::from(k))).collect());
            }
        },
        |shell| {
            assert_eq!(decode::surface(shell, cavity, frame), Err(ReadError::Limit));
        },
    );
}
