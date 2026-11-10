use super::*;

const START: &str = "ISO-10303-21;HEADER;FILE_DESCRIPTION();FILE_NAME();FILE_SCHEMA();ENDSEC;DATA;";
const END: &str = "ENDSEC;END-ISO-10303-21;";

fn shell_data() -> String {
    format!(
        "{START}{}{}",
        concat!(
            "#1=CARTESIAN_POINT('',(0.,0.,0.));",
            "#2=CARTESIAN_POINT('',(1.,1.,0.));",
            "#3=CARTESIAN_POINT('',(2.,1.,0.));",
            "#4=CARTESIAN_POINT('',(3.,0.,0.));",
            "#5=B_SPLINE_CURVE_WITH_KNOTS('',3,(#1,#2,#3,#4),.UNSPECIFIED.,.F.,.F.,(4,4),(0.,1.),.UNSPECIFIED.);",
            "#6=DIRECTION('',(0.,0.,1.));",
            "#7=VECTOR('',#6,1.);",
            "#8=SURFACE_OF_LINEAR_EXTRUSION('',#5,#7);",
        ),
        END
    )
}

#[test]
fn spline_extrema_integral_and_extrusion_follow_decoded_geometry() {
    let input = shell_data();
    let entities = parse_entities(&input, 1e-7).unwrap();
    let shell = Shell::new(&entities, 1e-7, true);
    let spline = shell.spline_curve(5).unwrap();
    assert_eq!(spline.poles_mm().len(), 4);
    assert_eq!(spline.knots(), [0., 1.]);
    assert_eq!(spline.multiplicities(), [4, 4]);
    assert_eq!(spline.bounds(), [[0., 0., 0.], [3., 0.75, 0.]]);
    assert!((spline.signed_area([0.; 3], [0., 0., 1.]) + 1.5).abs() < 1e-12);
    assert_eq!(shell.spline_extrusion(8).unwrap(), (5, [0., 0., 1.]));
    let shifted = SplineEdge {
        poles_mm: spline
            .poles_mm
            .iter()
            .map(|&[x, y, _]| [x, y, 3.])
            .collect(),
        ..spline.clone()
    };
    assert!(spline.translated(&shifted, [0., 0., 3.], 1e-7));
    assert!(!spline.translated(&shifted, [0., 0., 2.], 1e-7));
    assert!(!spline.translated(
        &SplineEdge {
            knots: vec![0., 2.],
            ..shifted
        },
        [0., 0., 3.],
        1e-7
    ));
}

#[test]
fn double_knot_join_uses_adjacent_span_lengths() {
    let spline = SplineEdge {
        poles_mm: [0., 1. / 3., 2. / 3., 5. / 3., 7. / 3., 3.]
            .map(|x| [x, 0., 0.])
            .to_vec(),
        knots: vec![0., 1., 3.],
        multiplicities: vec![4, 2, 4],
    };
    let spans: Vec<_> = spline.bezier_spans().collect();
    assert_eq!(spans[0][3], [1., 0., 0.]);
    assert_eq!(spans[0][3], spans[1][0]);
    assert_eq!(evaluate(spans[0], 0.5), [0.5, 0., 0.]);
    assert_eq!(evaluate(spans[1], 0.5), [2., 0., 0.]);
    assert_eq!(quadratic_roots([1., 1., 2.]), [None; 2]);
    assert_eq!(quadratic_roots([0., 0., 1.]), [Some(0.), None]);
    let roots = quadratic_roots([3. / 16., -5. / 16., 3. / 16.]);
    assert_eq!(roots, [Some(0.75), Some(0.25)]);
}

#[test]
fn decoder_refuses_unsupported_shapes_and_bad_parameters() {
    let original = shell_data();
    let too_many_poles = format!("({})", vec!["#1"; 257].join(","));
    for (from, to, error) in [
        (",3,(#1", ",2,(#1", ReadError::Unsupported),
        ("(4,4)", "(3,4)", ReadError::Unsupported),
        ("(4,4)", "(4,2,4)", ReadError::Invalid),
        ("(0.,1.)", "(0.,0.)", ReadError::Invalid),
        ("(0.,1.)", "(0.,1E13)", ReadError::Invalid),
        ("(#1,#2,#3,#4)", too_many_poles.as_str(), ReadError::Limit),
    ] {
        let source = original.replacen(from, to, 1);
        let entities = parse_entities(&source, 1e-7).unwrap();
        let shell = Shell::new(&entities, 1e-7, true);
        assert_eq!(shell.spline_curve(5), Err(error));
    }
    for (from, to) in [
        ("VECTOR('',#6,1.)", "VECTOR('',#6,0.)"),
        ("('',#5,#7)", "('',#7)"),
        ("VECTOR('',#6,1.)", "VECTOR('',#6)"),
    ] {
        let source = original.replacen(from, to, 1);
        let entities = parse_entities(&source, 1e-7).unwrap();
        let shell = Shell::new(&entities, 1e-7, true);
        assert_eq!(shell.spline_extrusion(8), Err(ReadError::Invalid));
    }
}
