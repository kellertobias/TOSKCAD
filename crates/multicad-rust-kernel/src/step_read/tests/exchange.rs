use crate::{
    Rational,
    polygon::PolygonPrism,
    step::{write_box, write_polygon_prism},
    step_read::{ReadError, read_planar_solid},
};

const TOLERANCE: f64 = 1e-7;
const X: [f64; 3] = [1., 0., 0.];
const Y: [f64; 3] = [0., 1., 0.];

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn box_step() -> String {
    write_box([n(12), n(8), n(4)], [10., 20., 30.], X, Y).unwrap()
}

fn entity<'a>(step: &'a str, kind: &str, index: usize) -> &'a str {
    let marker = format!("={kind}(");
    step.lines()
        .filter(|line| line.contains(&marker))
        .nth(index)
        .unwrap()
}

fn replace(step: &str, kind: &str, index: usize, edit: impl FnOnce(&str) -> String) -> String {
    let line = entity(step, kind, index);
    step.replacen(line, &edit(line), 1)
}

fn arguments(step: &str, kind: &str, args: &str) -> String {
    replace(step, kind, 0, |line| {
        let (id, _) = line.split_once('=').unwrap();
        format!("{id}={kind}({args});")
    })
}

fn before_end(step: &str, record: &str) -> String {
    step.replacen(
        "ENDSEC;\nEND-ISO",
        &format!("{record}\nENDSEC;\nEND-ISO"),
        1,
    )
}

#[test]
fn readback_measures_actual_geometry() {
    let source = box_step()
        .replace("MultiCAD analytic solid", "misleading text")
        .replace("MultiCAD", "another implementation");
    let read = read_planar_solid(&source, TOLERANCE).unwrap();
    assert_eq!(read.bounds_mm, [[10., 20., 30.], [22., 28., 34.]]);
    assert_eq!(read.volume_mm3, 384.);
    assert_eq!(read.face_count, 6);
    assert_eq!(read.solid_count, 1);
    assert_eq!(read.edges_mm.len(), 12);
    assert!(
        read.edges_mm
            .iter()
            .any(|edge| { edge.contains(&[22., 20., 34.]) && edge.contains(&[22., 28., 34.]) })
    );
}

#[test]
fn reverse_placed_concave_sweep_subtracts_multiple_hole_areas() {
    let contours = [
        &[[0, 0], [8, 0], [8, 3], [4, 3], [4, 7], [0, 7]][..],
        &[[1, 1], [1, 2], [2, 2], [2, 1]][..],
        &[[1, 4], [1, 6], [2, 6], [2, 4]][..],
    ];
    let prism = PolygonPrism::new(
        contours
            .into_iter()
            .map(|points| points.iter().map(|point| point.map(n)).collect())
            .collect(),
        n(3),
    )
    .unwrap();
    let diagonal = (0.5_f64).sqrt();
    let step = write_polygon_prism(
        &prism,
        [100., -20., 300.],
        [diagonal, diagonal, 0.],
        [0., 0., 1.],
        true,
    )
    .unwrap();
    let read = read_planar_solid(&step, TOLERANCE).unwrap();
    assert!((read.volume_mm3 - 111.).abs() < TOLERANCE);
    assert_eq!(read.face_count, 16);
    assert_eq!(read.edges_mm.len(), 42);
    for (actual, expected) in read.bounds_mm.into_iter().flatten().zip([
        100. - 3. * diagonal,
        -20.,
        300.,
        100. + 8. * diagonal,
        -20. + 11. * diagonal,
        307.,
    ]) {
        assert!((actual - expected).abs() < TOLERANCE);
    }
}

#[test]
fn unsupported_geometry_or_context_is_rejected() {
    let step = box_step();
    for source in [
        step.replace(".MILLI.", ".CENTI."),
        step.replace("LENGTH_UNIT()", "SOLID_ANGLE_UNIT()"),
        step.replace(
            "GEOMETRIC_REPRESENTATION_CONTEXT(3)",
            "GEOMETRIC_REPRESENTATION_CONTEXT(2)",
        ),
        step.replace("GLOBAL_UNIT_ASSIGNED_CONTEXT", "OTHER_CONTEXT"),
        step.replace("ADVANCED_BREP_SHAPE_REPRESENTATION", "SHAPE_REPRESENTATION"),
        step.replace("MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS"),
        step.replace("CLOSED_SHELL", "OPEN_SHELL"),
        step.replace("=LINE(", "=CIRCLE("),
        step.replace("=PLANE(", "=CYLINDRICAL_SURFACE("),
        step.replace("=FACE_OUTER_BOUND(", "=OTHER_BOUND("),
        before_end(
            &step,
            &format!(
                "#999001={}",
                entity(&step, "ADVANCED_BREP_SHAPE_REPRESENTATION", 0)
                    .split_once('=')
                    .unwrap()
                    .1
            ),
        ),
    ] {
        assert_eq!(
            read_planar_solid(&source, TOLERANCE),
            Err(ReadError::Unsupported),
            "{source}"
        );
    }
}

#[test]
fn malformed_solid_reference_chain_is_rejected() {
    let step = box_step();
    for (kind, args) in [
        ("ADVANCED_BREP_SHAPE_REPRESENTATION", "''"),
        ("MANIFOLD_SOLID_BREP", "''"),
        ("CLOSED_SHELL", "'',#1"),
        ("CLOSED_SHELL", "'',(#1,#1,#1,#1)"),
        ("CLOSED_SHELL", "''"),
        ("SHAPE_DEFINITION_REPRESENTATION", "#999001,#999001"),
        ("AXIS2_PLACEMENT_3D", "''"),
        ("CARTESIAN_POINT", "'',($,$,$)"),
        ("CARTESIAN_POINT", "'',#1"),
        ("CARTESIAN_POINT", "''"),
        ("DIRECTION", "'',(0.,0.,0.)"),
        ("DIRECTION", "''"),
        ("ADVANCED_FACE", "'',(),#999001,.T."),
        ("ADVANCED_FACE", "''"),
        ("FACE_OUTER_BOUND", "''"),
        ("EDGE_LOOP", "'',()"),
        ("EDGE_LOOP", "''"),
        ("ORIENTED_EDGE", "'',$,$,#1,.T."),
        ("ORIENTED_EDGE", "'',*,*,#1,.U."),
        ("EDGE_CURVE", "''"),
        ("VERTEX_POINT", "''"),
        ("LINE", "''"),
        ("VECTOR", "''"),
        ("PLANE", "''"),
    ] {
        assert_eq!(
            read_planar_solid(&arguments(&step, kind, args), TOLERANCE),
            Err(ReadError::Invalid),
            "{kind} with {args}"
        );
    }
    let wrong_units = step.replace(
        "GLOBAL_UNIT_ASSIGNED_CONTEXT((",
        "GLOBAL_UNIT_ASSIGNED_CONTEXT($,(",
    );
    assert_eq!(
        read_planar_solid(&wrong_units, TOLERANCE),
        Err(ReadError::Invalid)
    );
    let literal_edge = replace(&step, "ORIENTED_EDGE", 0, |line| {
        let (id, _) = line.split_once('=').unwrap();
        format!("{id}=ORIENTED_EDGE('',*,*,42,.T.);")
    });
    assert_eq!(
        read_planar_solid(&literal_edge, TOLERANCE),
        Err(ReadError::Invalid)
    );
}

#[test]
fn winding_closure_incidence_and_geometry_are_independently_checked() {
    let step = box_step();
    for kind in [
        "ADVANCED_FACE",
        "ORIENTED_EDGE",
        "EDGE_CURVE",
        "FACE_OUTER_BOUND",
    ] {
        let changed = replace(&step, kind, 0, |line| {
            if line.contains(".T.") {
                line.replace(".T.", ".F.")
            } else {
                line.replace(".F.", ".T.")
            }
        });
        assert_eq!(
            read_planar_solid(&changed, TOLERANCE),
            Err(ReadError::Invalid),
            "{kind}"
        );
    }
    let short_edge = replace(&step, "EDGE_CURVE", 0, |line| {
        let mut args = line.split(',');
        let name = args.next().unwrap();
        let start = args.next().unwrap();
        args.next().unwrap();
        format!(
            "{name},{start},{start},{}",
            args.collect::<Vec<_>>().join(",")
        )
    });
    assert_eq!(
        read_planar_solid(&short_edge, TOLERANCE),
        Err(ReadError::Invalid)
    );

    let shell = replace(&step, "CLOSED_SHELL", 0, |line| {
        let (prefix, _) = line.rsplit_once(',').unwrap();
        format!("{prefix}));")
    });
    assert_eq!(
        read_planar_solid(&shell, TOLERANCE),
        Err(ReadError::Invalid)
    );
    for tolerance in [0., -1., f64::NAN, f64::INFINITY] {
        assert_eq!(read_planar_solid(&step, tolerance), Err(ReadError::Invalid));
    }
    assert_eq!(
        read_planar_solid(
            &arguments(&step, "CARTESIAN_POINT", "'',(1.E+13,0.,0.)"),
            TOLERANCE
        ),
        Err(ReadError::Limit)
    );
}

fn id(line: &str) -> &str {
    line.split_once('=').unwrap().0
}

#[test]
fn valid_equivalent_senses_and_implicit_outer_bound_read_identically() {
    let step = box_step();
    let rewritten = replace(&step, "AXIS2_PLACEMENT_3D", 0, |line| {
        let normal = line.split(',').nth(2).unwrap();
        line.replace(normal, "#999001")
    });
    let rewritten = before_end(&rewritten, "#999001=DIRECTION('',(0.,0.,1.));");
    let rewritten = replace(&rewritten, "ADVANCED_FACE", 0, |line| {
        line.replace(".T.", ".F.")
    });
    assert_eq!(
        read_planar_solid(
            &rewritten.replace("FACE_OUTER_BOUND", "FACE_BOUND"),
            TOLERANCE
        ),
        read_planar_solid(&step, TOLERANCE)
    );
}

#[test]
fn degenerate_and_nonmanifold_references_are_rejected() {
    let step = box_step();
    let base_loop = id(entity(&step, "EDGE_LOOP", 0));
    let bound = id(entity(&step, "FACE_OUTER_BOUND", 0));
    let face = id(entity(&step, "ADVANCED_FACE", 0));
    let face_direction = id(entity(&step, "DIRECTION", 4));
    let face_plane = id(entity(&step, "PLANE", 0));
    let other_height = id(entity(&step, "CARTESIAN_POINT", 4));
    let vector_direction = id(entity(&step, "DIRECTION", 0));
    let unit_context = step
        .lines()
        .find(|line| line.contains("GEOMETRIC_REPRESENTATION_CONTEXT"))
        .map(id)
        .unwrap();
    let unsupported_point = arguments(&step, "VERTEX_POINT", &format!("'',{}", unit_context));
    assert_eq!(
        read_planar_solid(&unsupported_point, TOLERANCE),
        Err(ReadError::Unsupported)
    );
    let inward = step
        .lines()
        .map(|line| {
            if line.contains("=ADVANCED_FACE(") || line.contains("=FACE_OUTER_BOUND(") {
                line.replace(".T.", ".F.")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let overused = before_end(
        &step,
        &format!("#999001=ADVANCED_FACE('',({bound}),{face_plane},.T.);"),
    );
    let overused = replace(&overused, "CLOSED_SHELL", 0, |line| {
        line.replace("));", ",#999001));")
    });
    let open_loop = replace(&step, "EDGE_LOOP", 0, |line| {
        let (prefix, _) = line.rsplit_once(',').unwrap();
        format!("{prefix}));")
    });
    let oversized_loop = arguments(
        &step,
        "EDGE_LOOP",
        &format!("'',({})", vec![base_loop; 1025].join(",")),
    );
    assert_eq!(
        read_planar_solid(&oversized_loop, TOLERANCE),
        Err(ReadError::Limit)
    );
    for (name, source) in [
        ("inward", inward),
        ("open loop", open_loop),
        ("three faces on edge", overused),
        (
            "parallel axes",
            arguments(
                &step,
                "AXIS2_PLACEMENT_3D",
                &format!("'',#1,{face_direction},{face_direction}"),
            ),
        ),
        (
            "face off plane",
            arguments(
                &step,
                "AXIS2_PLACEMENT_3D",
                &format!("'',{other_height},{face_direction},{vector_direction}"),
            ),
        ),
        (
            "zero line vector",
            arguments(&step, "VECTOR", &format!("'',{vector_direction},0.")),
        ),
        (
            "two outer rings",
            arguments(
                &step,
                "ADVANCED_FACE",
                &format!("'',({bound},{bound}),{face_plane},.T."),
            ),
        ),
        (
            "duplicate face",
            replace(&step, "CLOSED_SHELL", 0, |line| {
                line.replace("));", &format!(",{face}));"))
            }),
        ),
    ] {
        assert_eq!(
            read_planar_solid(&source, TOLERANCE),
            Err(ReadError::Invalid),
            "{name}"
        );
    }
}
