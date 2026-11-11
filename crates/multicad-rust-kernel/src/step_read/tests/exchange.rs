use crate::{
    Circle, Rational,
    annulus::Annulus,
    cylinder::AxialCylinder,
    orthogonal::{Box3, Operation, boolean},
    orthogonal_boundary,
    polygon::PolygonPrism,
    step::{
        StepError, write_axial_cylinder, write_box, write_orthogonal, write_placed_annulus,
        write_polygon_prism,
    },
    step_read::{
        ImportProfile, ImportSolid, ReadError, cross, dot, read_analytic_solid, read_import_solid,
        read_planar_solid, sub,
    },
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

fn cylinder_step() -> String {
    let diagonal = (0.5_f64).sqrt();
    let circle = Circle::new([n(3), n(3)], n(1)).unwrap();
    let cylinder = AxialCylinder::new(1, circle, n(-5), n(2)).unwrap();
    write_axial_cylinder(&cylinder, [diagonal, diagonal, 0.]).unwrap()
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "{actual} differs from {expected}"
    );
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
    assert_eq!(read_analytic_solid(&source, TOLERANCE).unwrap(), read);
}

#[test]
fn import_box_frame_ignores_record_and_shell_order() {
    let source = box_step();
    let (readback, recognized) = read_import_solid(&source, TOLERANCE).unwrap();
    assert_eq!(readback, read_planar_solid(&source, TOLERANCE).unwrap());
    assert_eq!(
        recognized,
        ImportSolid {
            origin_mm: [10., 20., 30.],
            u: X,
            v: Y,
            height_mm: 4.,
            profile: ImportProfile::Polygon {
                rings: vec![vec![[0., 0.], [12., 0.], [12., 8.], [0., 8.]]],
                circular_holes: Vec::new(),
            },
        }
    );
    let source = replace(&source, "CLOSED_SHELL", 0, |line| {
        let face_ids = (0..6)
            .rev()
            .map(|i| id(entity(&source, "ADVANCED_FACE", i)))
            .collect::<Vec<_>>();
        format!("{}=CLOSED_SHELL('',({}));", id(line), face_ids.join(","))
    });
    let (header, data) = source.split_once("\nDATA;\n").unwrap();
    let (records, trailer) = data.split_once("\nENDSEC;").unwrap();
    let records = records
        .split_inclusive('\n')
        .rev()
        .map(str::trim_end)
        .collect::<Vec<_>>();
    let reversed = format!("{header}\nDATA;\n{}\nENDSEC;{trailer}", records.join("\n"));
    assert_eq!(
        read_import_solid(&reversed, TOLERANCE).unwrap().1,
        recognized
    );
}

#[test]
fn import_recognizes_oblique_shell_and_a_blind_pocket() {
    let cube = Box3 {
        min: [0.; 3],
        max: [3.; 3],
    };
    let solid = boolean(&[cube], &[], Operation::Union, TOLERANCE)
        .unwrap()
        .unwrap();
    let boundary = orthogonal_boundary::build(solid.quads()).unwrap();
    let oblique = write_orthogonal(&boundary, |[x, y, z]| [x + z, y + z, z]).unwrap();
    close(
        read_analytic_solid(&oblique, TOLERANCE).unwrap().volume_mm3,
        27.,
    );
    let (_, recognized) = read_import_solid(&oblique, TOLERANCE).unwrap();
    assert_eq!(
        recognized.profile,
        ImportProfile::LinearSweep {
            base: vec![vec![[0., 0.], [3., 0.], [3., 3.], [0., 3.]]],
            top: vec![vec![[3., 3.], [6., 3.], [6., 6.], [3., 6.]]],
        }
    );

    let tool = Box3 {
        min: [1.; 3],
        max: [2., 2., 4.],
    };
    let solid = boolean(&[cube], &[tool], Operation::Subtract, TOLERANCE)
        .unwrap()
        .unwrap();
    let boundary = orthogonal_boundary::build(solid.quads()).unwrap();
    let blind_pocket = write_orthogonal(&boundary, |p| p).unwrap();
    close(
        read_analytic_solid(&blind_pocket, TOLERANCE)
            .unwrap()
            .volume_mm3,
        25.,
    );
    let (_, recognized) = read_import_solid(&blind_pocket, TOLERANCE).unwrap();
    let ImportProfile::Orthogonal { solid: imported } = recognized.profile else {
        panic!("expected a blind pocket with an orthogonal boundary");
    };
    assert_eq!(solid.cells(), imported.cells());
    assert_eq!(solid.quads(), imported.quads());
}

#[test]
fn import_orthogonal_through_and_blind_cuts_in_a_rotated_far_placement() {
    let stock = Box3 {
        min: [0.; 3],
        max: [12., 10., 6.],
    };
    let tools = [
        Box3 {
            min: [1., 1., 4.],
            max: [3., 3., 7.],
        },
        Box3 {
            min: [8., 6., -1.],
            max: [10., 9., 3.],
        },
        Box3 {
            min: [5., 4., -1.],
            max: [7., 6., 7.],
        },
        Box3 {
            min: [4., 8., 4.],
            max: [6., 9., 7.],
        },
    ];
    let solid = boolean(&[stock], &tools, Operation::Subtract, TOLERANCE)
        .unwrap()
        .unwrap();
    let boundary = orthogonal_boundary::build(solid.quads()).unwrap();
    let placed = |[x, y, z]: [f64; 3]| {
        [
            1_000_000. + 0.6 * x - 0.48 * y + 0.64 * z,
            -700_000. + 0.8 * x + 0.36 * y - 0.48 * z,
            200_000. + 0.8 * y + 0.6 * z,
        ]
    };
    let source = write_orthogonal(&boundary, placed).unwrap();
    let (readback, imported) = read_import_solid(&source, TOLERANCE).unwrap();
    close(readback.volume_mm3, 666.);
    let ImportProfile::Orthogonal { solid: rebuilt } = imported.profile else {
        panic!("expected a rotated orthogonal boundary");
    };
    close(rebuilt.volume_mm3(), 666.);
    assert_eq!(rebuilt.solid_count(), 1);
    let material = |point| {
        let delta = sub(placed(point), imported.origin_mm);
        let point =
            [imported.u, imported.v, cross(imported.u, imported.v)].map(|axis| dot(delta, axis));
        rebuilt.cells().iter().any(|cell| {
            (0..3).all(|i| cell.min[i] + 1e-7 < point[i] && point[i] < cell.max[i] - 1e-7)
        })
    };
    assert!(!material([1.5, 1.5, 5.]));
    assert!(!material([9., 7., 2.]));
    assert!(!material([6., 5., 2.]));
    assert!(!material([5.5, 8.5, 5.]));
    assert!(material([1.5, 1.5, 2.]));
    assert!(material([9., 7., 5.]));
    assert!(material([5.5, 8.5, 2.]));

    for shear in [0.01, 1e-6] {
        let skewed = write_orthogonal(&boundary, |[x, y, z]| [x + z * shear, y, z]).unwrap();
        assert!(read_planar_solid(&skewed, TOLERANCE).is_ok());
        assert_eq!(
            read_import_solid(&skewed, TOLERANCE),
            Err(ReadError::Unsupported)
        );
    }
}

#[test]
fn import_orthogonal_refuses_distinct_cardinal_planes_inside_tolerance() {
    for (bottom, floor, gap) in [(0., 4., TOLERANCE / 2.), (-1e7, 0., 5e-10)] {
        let stock = Box3 {
            min: [0., 0., bottom],
            max: [12., 10., 6.],
        };
        let tools = [
            Box3 {
                min: [1., 1., floor],
                max: [3., 3., 7.],
            },
            Box3 {
                min: [7., 6., floor + gap],
                max: [9., 8., 7.],
            },
        ];
        let solid = boolean(&[stock], &tools, Operation::Subtract, 0.)
            .unwrap()
            .unwrap();
        let boundary = orthogonal_boundary::build(solid.quads()).unwrap();
        let source = write_orthogonal(&boundary, |p| p).unwrap();
        assert!(read_planar_solid(&source, TOLERANCE).is_ok());
        assert_eq!(
            read_import_solid(&source, TOLERANCE),
            Err(ReadError::Unsupported)
        );
    }
}

#[test]
fn surface_curve_keeps_validated_3d_geometry_as_master() {
    let source = box_step();
    let line = id(entity(&source, "LINE", 0));
    let plane = id(entity(&source, "PLANE", 0));
    let point = id(entity(&source, "CARTESIAN_POINT", 0));
    let wrapped = replace(&source, "EDGE_CURVE", 0, |edge| {
        edge.replace(&format!(",{line},"), ",#999001,")
    });
    let record = |args| before_end(&wrapped, &format!("#999001=SURFACE_CURVE({args});"));
    let correct = record(format!("'',{line},({plane}),.CURVE_3D."));
    assert_eq!(
        read_import_solid(&correct, TOLERANCE),
        read_import_solid(&source, TOLERANCE)
    );
    for (args, error) in [
        (format!("'',{line},({plane})"), ReadError::Invalid),
        (format!("'',{line},(),.CURVE_3D."), ReadError::Invalid),
        (
            format!("'',{line},({plane}),.PCURVE_S1."),
            ReadError::Unsupported,
        ),
        (
            format!("'',{line},({point}),.CURVE_3D."),
            ReadError::Unsupported,
        ),
    ] {
        assert_eq!(read_import_solid(&record(args), TOLERANCE), Err(error));
    }
}

#[test]
fn selected_planar_pcurve_must_match_the_trimmed_3d_line() {
    let source = write_box([n(100_000), n(8), n(4)], [10., 20., 30.], X, Y).unwrap();
    let line = id(entity(&source, "LINE", 0));
    let plane = id(entity(&source, "PLANE", 0));
    let wrapped = replace(&source, "EDGE_CURVE", 0, |edge| {
        edge.replace(&format!(",{line},"), ",#999001,")
    });
    let wrapped = before_end(
        &wrapped,
        &format!(
            "#999001=SURFACE_CURVE('',{line},(#999002,#999002),.PCURVE_S2.);\n\
         #999002=PCURVE('',{plane},#999003);\n\
         #999003=DEFINITIONAL_REPRESENTATION('',(#999004),#999008);\n\
         #999004=LINE('',#999005,#999006);\n\
         #999005=CARTESIAN_POINT('',(100000.,-8.));\n\
         #999006=VECTOR('',#999007,1.);\n\
         #999007=DIRECTION('',(-1.,0.));\n\
         #999008=(GEOMETRIC_REPRESENTATION_CONTEXT(2) PARAMETRIC_REPRESENTATION_CONTEXT() REPRESENTATION_CONTEXT('2D SPACE',''));"
        ),
    );
    let expected = read_import_solid(&source, TOLERANCE).unwrap();
    assert_eq!(read_import_solid(&wrapped, TOLERANCE).unwrap(), expected);
    let first = wrapped.replace(".PCURVE_S2.", ".PCURVE_S1.");
    assert_eq!(read_import_solid(&first, TOLERANCE).unwrap(), expected);

    let clamped = wrapped
        .replace(
            "#999004=LINE('',#999005,#999006);",
            "#999004=B_SPLINE_CURVE_WITH_KNOTS('',1,(#999005,#999010),.UNSPECIFIED.,.F.,.F.,(2,2),(-10.,100000.),.PIECEWISE_BEZIER_KNOTS.);\n\
             #999010=CARTESIAN_POINT('',(0.,-8.));",
        )
        .replace("(100000.,-8.)", "(100010.,-8.)");
    assert_eq!(read_import_solid(&clamped, TOLERANCE).unwrap(), expected);
    for (invalid, error) in [
        (
            clamped.replace("'',1,(#999005", "'',2,(#999005"),
            ReadError::Unsupported,
        ),
        (clamped.replace("(2,2)", "(1,2)"), ReadError::Unsupported),
        (
            clamped.replace("(-10.,100000.)", "(100000.,100000.)"),
            ReadError::Invalid,
        ),
        (
            clamped.replace("(-10.,100000.)", "(0.,5.E-324)"),
            ReadError::Invalid,
        ),
        (
            clamped.replace("(100010.,-8.)", "(100010.,-7.)"),
            ReadError::Invalid,
        ),
        (
            clamped.replace("#999005,#999010", "#999010,#999005"),
            ReadError::Invalid,
        ),
        (
            clamped.replace(&format!("PCURVE('',{plane},"), "PCURVE('',#999006,"),
            ReadError::Unsupported,
        ),
        (
            clamped
                .replace("(-10.,100000.)", "(10.,100000.)")
                .replace("(100010.,-8.)", "(99990.,-8.)"),
            ReadError::Invalid,
        ),
        (
            clamped
                .replace("(-10.,100000.)", "(-10.,99990.)")
                .replace("(0.,-8.)", "(10.,-8.)"),
            ReadError::Invalid,
        ),
    ] {
        assert_eq!(read_import_solid(&invalid, TOLERANCE), Err(error));
    }

    for (from, to, error) in [
        (".PCURVE_S2.", ".UNKNOWN.", ReadError::Unsupported),
        ("#999002,#999002", "#999002", ReadError::Invalid),
        ("#999003);", "#999003,$);", ReadError::Invalid),
        (
            "#999004),#999008);",
            "#999004),#999008,$);",
            ReadError::Invalid,
        ),
        (
            "(#999004),#999008);",
            "(#999004,#999004),#999008);",
            ReadError::Unsupported,
        ),
        ("CONTEXT(2)", "CONTEXT(3)", ReadError::Unsupported),
        (
            "#999005=CARTESIAN_POINT('',(100000.,-8.));",
            "#999005=CARTESIAN_POINT((100000.,-8.));",
            ReadError::Invalid,
        ),
        ("(100000.,-8.)", "(100000.,-8.,0.)", ReadError::Invalid),
        ("(100000.,-8.)", "(100000.,-7.)", ReadError::Invalid),
        ("(100000.,-8.)", "(1.E13,-8.)", ReadError::Limit),
        ("(-1.,0.)", "(1.,0.)", ReadError::Invalid),
        ("(-1.,0.)", "(-1.,1.E-11)", ReadError::Invalid),
        ("#999007,1.)", "#999007,2.)", ReadError::Invalid),
    ] {
        assert!(wrapped.contains(from));
        let invalid = wrapped.replace(from, to);
        assert_eq!(
            read_import_solid(&invalid, TOLERANCE),
            Err(error),
            "{from} → {to}"
        );
    }
    let unused_plane = entity(&source, "PLANE", 0)
        .replacen(plane, "#999009", 1)
        .replace("PLANE('',", "PLANE('',*,");
    let invalid = before_end(
        &wrapped.replace(&format!("PCURVE('',{plane},"), "PCURVE('',#999009,"),
        &unused_plane,
    );
    assert_eq!(
        read_import_solid(&invalid, TOLERANCE),
        Err(ReadError::Invalid)
    );
}

#[test]
fn read_cylinder_extrema_away_from_the_seam_and_measure_volume() {
    let source = cylinder_step();
    assert!(read_planar_solid(&source, TOLERANCE).is_err());
    let read = read_analytic_solid(&source, TOLERANCE).unwrap();
    assert_eq!(read.bounds_mm, [[2., 2., -5.], [4., 4., -3.]]);
    close(read.volume_mm3, 2. * std::f64::consts::PI);
    assert_eq!(read.face_count, 3);
    assert!(read.edges_mm.is_empty());
    assert_eq!(read.circles.len(), 2);
    for (circle, z) in read.circles.iter().zip([-5., -3.]) {
        assert_eq!(circle.center_mm, [3., 3., z]);
        assert_eq!(circle.normal, [0., 0., 1.]);
        assert_eq!(circle.radius_mm, 1.);
    }
    let (imported, solid) = read_import_solid(&source, TOLERANCE).unwrap();
    assert_eq!(imported, read);
    assert_eq!(solid.origin_mm, [3., 3., -5.]);
    assert_eq!(solid.height_mm, 2.);
    assert_eq!(solid.profile, ImportProfile::Cylinder { radius_mm: 1. });
}

#[test]
fn selected_planar_circle_pcurve_must_match_the_full_master_rim() {
    let source = cylinder_step();
    let circle = id(entity(&source, "CIRCLE", 1));
    let plane = id(entity(&source, "PLANE", 1));
    let wrapped = replace(&source, "EDGE_CURVE", 1, |edge| {
        edge.replace(&format!(",{circle},"), ",#999001,")
    });
    let step = before_end(
        &wrapped,
        &format!(
            "#999001=SURFACE_CURVE('',{circle},(#999002),.PCURVE_S1.);\n\
             #999002=PCURVE('',{plane},#999003);\n\
             #999003=DEFINITIONAL_REPRESENTATION('',(#999004),#999005);\n\
             #999004=CIRCLE('',#999006,1.);\n\
             #999005=(GEOMETRIC_REPRESENTATION_CONTEXT(2) PARAMETRIC_REPRESENTATION_CONTEXT() REPRESENTATION_CONTEXT('2D SPACE',''));\n\
             #999006=AXIS2_PLACEMENT_2D('',#999007,$);\n\
             #999007=CARTESIAN_POINT('',(0.,0.));"
        ),
    );
    assert_eq!(
        read_import_solid(&step, TOLERANCE),
        read_import_solid(&source, TOLERANCE)
    );
    assert_eq!(
        read_import_solid(&step.replace("#999006,1.", "#999006,1.1"), TOLERANCE),
        Err(ReadError::Invalid)
    );
}

#[test]
fn annulus_round_trip_preserves_analytic_bore_and_tilted_extrema() {
    let annulus = Annulus::new(2.5, 6., 1.75).unwrap();
    let diagonal = 0.5_f64.sqrt();
    let origin = [100., -20., 300.];
    let v = [-diagonal, 0., diagonal];
    assert_eq!(
        write_placed_annulus(&annulus, origin, Y, Y),
        Err(StepError::InvalidPlacement)
    );
    let source = write_placed_annulus(&annulus, origin, Y, v).unwrap();
    assert_eq!(source.matches("=CYLINDRICAL_SURFACE(").count(), 2);
    assert_eq!(source.matches("=FACE_BOUND(").count(), 2);
    assert!(read_planar_solid(&source, TOLERANCE).is_err());
    let readback = read_analytic_solid(&source, TOLERANCE).unwrap();
    let (verified, solid) = read_import_solid(&source, TOLERANCE).unwrap();
    assert_eq!(verified, readback);
    assert_eq!(readback.solid_count, 1);
    assert_eq!(readback.face_count, 4);
    assert_eq!(readback.circles.len(), 4);
    assert!(readback.edges_mm.is_empty());
    close(
        readback.volume_mm3,
        std::f64::consts::PI * (36. - 6.25) * 1.75,
    );
    let radial = 6. * diagonal;
    let axial = 1.75 * diagonal;
    for (actual, expected) in readback.bounds_mm.into_iter().flatten().zip([
        100. - radial,
        -26.,
        300. - radial,
        100. + radial + axial,
        -14.,
        300. + radial + axial,
    ]) {
        close(actual, expected);
    }
    assert_eq!(solid.origin_mm, origin);
    close(solid.height_mm, 1.75);
    assert_eq!(
        solid.profile,
        ImportProfile::Annulus {
            inner_radius_mm: 2.5,
            outer_radius_mm: 6.
        }
    );

    // Cap bounds may not turn the hole into a second material island.
    let reversed = replace(&source, "FACE_BOUND", 0, |line| line.replace(".T.", ".F."));
    assert_eq!(
        read_import_solid(&reversed, TOLERANCE),
        Err(ReadError::Invalid)
    );
    let outward = replace(&source, "ADVANCED_FACE", 3, |line| {
        line.replace(".F.", ".T.")
    });
    assert_eq!(
        read_import_solid(&outward, TOLERANCE),
        Err(ReadError::Invalid)
    );
}

#[test]
fn native_annulus_proves_cylindrical_parameter_masters_and_implicit_cap_winding() {
    let source = include_str!("fixtures/occt-annulus.step");
    let (readback, solid) = read_import_solid(source, TOLERANCE).unwrap();
    assert_eq!(readback.face_count, 4);
    assert_eq!(readback.circles.len(), 4);
    close(
        readback.volume_mm3,
        std::f64::consts::PI * (6.125_f64.powi(2) - 2.75_f64.powi(2)) * 1.25,
    );
    assert_eq!(
        solid.profile,
        ImportProfile::Annulus {
            inner_radius_mm: 2.75,
            outer_radius_mm: 6.125
        }
    );
    close(solid.height_mm, 1.25);

    for (from, to, expected) in [
        ("(#31,#43)", "(#31)", ReadError::Invalid),
        (
            "#43 = PCURVE('',#32,",
            "#43 = PCURVE('',#120,",
            ReadError::Invalid,
        ),
        (
            "#41 = DIRECTION('',(-0.,-1.));",
            "#41 = DIRECTION('',(1.,0.));",
            ReadError::Invalid,
        ),
        (
            "#48 = DIRECTION('',(-0.,-1.));",
            "#48 = DIRECTION('',(1.,0.));",
            ReadError::Invalid,
        ),
        ("(-6.28318530718,0.)", "(-6.1,0.)", ReadError::Invalid),
        (
            "#63 = DIRECTION('',(-1.,0.));",
            "#63 = DIRECTION('',(-1.,1.));",
            ReadError::Unsupported,
        ),
        (
            "#53 = CIRCLE('',#54,6.125);",
            "#53 = CIRCLE('',#54,6.125,$);",
            ReadError::Invalid,
        ),
        (
            "#62 = VECTOR('',#63,1.);",
            "#62 = VECTOR('',#63,1.001);",
            ReadError::Invalid,
        ),
        (
            "#58 = PCURVE('',#32,",
            "#58 = PCURVE('',#27,",
            ReadError::Unsupported,
        ),
        (
            "#184 = FACE_BOUND('',#185,.F.);",
            "#184 = FACE_BOUND('',#185,.T.);",
            ReadError::Invalid,
        ),
    ] {
        assert!(source.contains(from), "{from}");
        assert_eq!(
            read_import_solid(&source.replacen(from, to, 1), TOLERANCE),
            Err(expected),
            "{from}"
        );
    }
    let wrong_cylinder = source.replacen("#31 = PCURVE('',#32,", "#31 = PCURVE('',#120,", 1);
    for change in ["'',#121,2.75,$", "'',#121,1.E-8"] {
        let invalid = wrong_cylinder.replacen("'',#121,2.75", change, 1);
        assert_eq!(
            read_import_solid(&invalid, TOLERANCE),
            Err(ReadError::Invalid),
            "{change}"
        );
    }
}

#[test]
fn read_tilted_prism_with_straight_and_round_open_holes_in_both_sweeps() {
    let outer = [[0, 0], [12, 0], [12, 8], [0, 8]];
    let square_hole = [[2, 2], [2, 4], [4, 4], [4, 2]];
    let slot_hole = [[5, 5], [5, 7], [6, 7], [6, 5]];
    let round_holes = [[7, 3], [9, 6]];
    let prism = PolygonPrism::with_circular_holes(
        vec![
            outer.map(|point| point.map(n)).to_vec(),
            slot_hole.map(|point| point.map(n)).to_vec(),
            square_hole.map(|point| point.map(n)).to_vec(),
        ],
        round_holes
            .iter()
            .map(|&center| Circle::new(center.map(n), n(1)).unwrap())
            .collect(),
        n(3),
    )
    .unwrap();
    let diagonal = (0.5_f64).sqrt();
    let origin = [100., -20., 300.];
    for reverse in [false, true] {
        let source = write_polygon_prism(
            &prism,
            origin,
            [diagonal, diagonal, 0.],
            [0., 0., 1.],
            reverse,
        )
        .unwrap();
        let read = read_analytic_solid(&source, TOLERANCE).unwrap();
        let (imported, solid) = read_import_solid(&source, TOLERANCE).unwrap();
        assert_eq!(imported, read);
        close(solid.height_mm, 3.);
        let ImportProfile::Polygon {
            rings,
            circular_holes,
        } = solid.profile
        else {
            panic!("expected polygon")
        };
        close(super::super::signed_area(&rings[0]), 96.);
        let mut hole_areas: Vec<_> = rings[1..]
            .iter()
            .map(|ring| super::super::signed_area(ring))
            .collect();
        hole_areas.sort_unstable_by(f64::total_cmp);
        assert_eq!(hole_areas.len(), 2);
        for (area, expected) in hole_areas.into_iter().zip([-4., -2.]) {
            close(area, expected);
        }
        assert_eq!(circular_holes.len(), 2);
        for (center, radius) in circular_holes {
            close(radius, 1.);
            let world: [f64; 3] = std::array::from_fn(|i| {
                solid.origin_mm[i] + center[0] * solid.u[i] + center[1] * solid.v[i]
            });
            assert!(
                read.circles
                    .iter()
                    .any(
                        |circle| super::super::length(super::super::sub(circle.center_mm, world))
                            < TOLERANCE
                    )
            );
        }
        close(read.volume_mm3, 3. * (90. - 2. * std::f64::consts::PI));
        assert_eq!(read.face_count, 16);
        assert_eq!(read.edges_mm.len(), 36);
        assert_eq!(read.circles.len(), 4);
        let sign = if reverse { -1. } else { 1. };
        let shift = 3. * sign * diagonal;
        for ([x, z], pair) in round_holes.into_iter().zip(read.circles.as_chunks::<2>().0) {
            for (circle, delta) in pair.iter().zip([0., shift]) {
                close(circle.center_mm[0], origin[0] + x as f64 * diagonal + delta);
                close(circle.center_mm[1], origin[1] + x as f64 * diagonal - delta);
                close(circle.center_mm[2], origin[2] + z as f64);
                close(circle.normal[0], diagonal * sign);
                close(circle.normal[1], -diagonal * sign);
                close(circle.normal[2], 0.);
                close(circle.radius_mm, 1.);
            }
        }
    }
}

#[test]
fn readback_rejects_corrupted_round_geometry_and_trim() {
    let source = cylinder_step();
    let axis = id(entity(&source, "AXIS2_PLACEMENT_3D", 0));
    let top_vertex = id(entity(&source, "VERTEX_POINT", 1));
    let cap_plane = id(entity(&source, "PLANE", 1));
    let wall = id(entity(&source, "CYLINDRICAL_SURFACE", 0));
    let wall_bound = id(entity(&source, "FACE_OUTER_BOUND", 2));
    let wall_circle = id(entity(&source, "ORIENTED_EDGE", 2));
    let seam = id(entity(&source, "EDGE_CURVE", 2));
    let bottom_circle = id(entity(&source, "EDGE_CURVE", 0));
    let top_circle = id(entity(&source, "EDGE_CURVE", 1));
    let bad_arc = replace(&source, "EDGE_CURVE", 0, |line| {
        let mut args = line.split(',').map(str::to_owned).collect::<Vec<_>>();
        args[2] = top_vertex.into();
        args.join(",")
    });
    let shorter_wall = replace(&source, "EDGE_LOOP", 2, |line| {
        line.replace(&format!("({wall_circle},"), "(")
    });
    let flip_wall_circle = replace(&source, "ORIENTED_EDGE", 2, |line| {
        line.replace(".T.", ".F.")
    });
    let bend_wall = replace(&source, "CYLINDRICAL_SURFACE", 0, |line| {
        let (prefix, _) = line.rsplit_once(',').unwrap();
        format!("{prefix},1.5);")
    });
    let extra_bound = replace(&source, "ADVANCED_FACE", 2, |line| {
        line.replace(
            &format!("({wall_bound})"),
            &format!("({wall_bound},{wall_bound})"),
        )
    });
    let duplicate_seam = before_end(
        &source,
        &entity(&source, "EDGE_CURVE", 2).replacen(seam, "#999001", 1),
    );
    let different_seam = replace(&duplicate_seam, "ORIENTED_EDGE", 5, |line| {
        line.replace(seam, "#999001")
    });
    let repeated_bottom_circle = before_end(
        &source,
        &format!("#999001=ORIENTED_EDGE('',*,*,{bottom_circle},.F.);"),
    );
    let wrong_wall_order = replace(&repeated_bottom_circle, "EDGE_LOOP", 2, |line| {
        let wall_up = id(entity(&source, "ORIENTED_EDGE", 3));
        let wall_down = id(entity(&source, "ORIENTED_EDGE", 5));
        let (id, _) = line.split_once('=').unwrap();
        format!("{id}=EDGE_LOOP('',({wall_circle},{wall_up},{wall_down},#999001));")
    });
    // Move the wall to the front so its repeated circular trim is checked
    // before the incidence limit on the bottom cap can reject it.
    let wrong_wall_order = replace(&wrong_wall_order, "CLOSED_SHELL", 0, |line| {
        let face_ids = (0..3)
            .map(|i| id(entity(&source, "ADVANCED_FACE", i)))
            .collect::<Vec<_>>();
        let (id, _) = line.split_once('=').unwrap();
        format!(
            "{id}=CLOSED_SHELL('',({},{},{}));",
            face_ids[2], face_ids[0], face_ids[1]
        )
    });
    for (name, changed, expected) in [
        (
            "missing circle fields",
            arguments(&source, "CIRCLE", "''"),
            ReadError::Invalid,
        ),
        (
            "missing cylinder fields",
            arguments(&source, "CYLINDRICAL_SURFACE", "''"),
            ReadError::Invalid,
        ),
        (
            "wrong circle radius",
            arguments(&source, "CIRCLE", &format!("'',{axis},1.25")),
            ReadError::Invalid,
        ),
        (
            "zero circle radius",
            arguments(&source, "CIRCLE", &format!("'',{axis},0.")),
            ReadError::Invalid,
        ),
        ("open arc", bad_arc, ReadError::Invalid),
        (
            "cap plane offset",
            replace(&source, "ADVANCED_FACE", 0, |line| {
                line.replace(id(entity(&source, "PLANE", 0)), cap_plane)
            }),
            ReadError::Invalid,
        ),
        ("wall radius", bend_wall, ReadError::Invalid),
        (
            "wall repeats separate seam identities",
            different_seam,
            ReadError::Invalid,
        ),
        (
            "wall places seam where circular trim belongs",
            wrong_wall_order,
            ReadError::Invalid,
        ),
        (
            "wall lacks full boundary",
            shorter_wall,
            ReadError::Unsupported,
        ),
        (
            "opposite wall winding",
            flip_wall_circle,
            ReadError::Invalid,
        ),
        (
            "additional cylindrical trim",
            extra_bound,
            ReadError::Unsupported,
        ),
        (
            "same circle on both caps",
            replace(&source, "ORIENTED_EDGE", 1, |line| {
                line.replace(top_circle, bottom_circle)
            }),
            ReadError::Invalid,
        ),
        (
            "noncylinder wall",
            replace(&source, "ADVANCED_FACE", 2, |line| {
                line.replace(wall, cap_plane)
            }),
            ReadError::Invalid,
        ),
    ] {
        assert_eq!(
            read_analytic_solid(&changed, TOLERANCE),
            Err(expected),
            "{name}"
        );
    }
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
    assert_eq!(
        read_planar_solid(
            &step.replace("MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS"),
            TOLERANCE
        ),
        Err(ReadError::Invalid)
    );
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
