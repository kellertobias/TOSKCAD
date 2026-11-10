use super::*;

const PLANE_AND_RIM: &str = "
#1=PLANE('',#2);
#2=AXIS2_PLACEMENT_3D('',#3,#4,#5);
#3=CARTESIAN_POINT('',(0.,0.,0.));
#4=DIRECTION('',(0.,0.,1.));
#5=DIRECTION('',(1.,0.,0.));
#6=CIRCLE('',#2,2.);
";

const CIRCLE: &str = "
#7=CIRCLE('',#8,2.);
#8=AXIS2_PLACEMENT_2D('',#9,#10);
#9=CARTESIAN_POINT('',(0.,0.));
#10=DIRECTION('',(1.,0.));
";

const RATIONAL_CIRCLE: &str = "
#7=(BOUNDED_CURVE() B_SPLINE_CURVE(2,(#11,#12,#13,#14,#15,#16,#17),.UNSPECIFIED.,.T.,.F.)
B_SPLINE_CURVE_WITH_KNOTS((1,2,2,2,2,1),(-1.,0.,2.0943951023931953,4.1887902047863905,6.283185307179586,7.),.UNSPECIFIED.)
CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE((1.,0.5,1.,0.5,1.,0.5,1.)) REPRESENTATION_ITEM(''));
#11=CARTESIAN_POINT('',(2.,0.));
#12=CARTESIAN_POINT('',(2.,3.4641016151377544));
#13=CARTESIAN_POINT('',(-1.,1.7320508075688772));
#14=CARTESIAN_POINT('',(-4.,0.));
#15=CARTESIAN_POINT('',(-1.,-1.7320508075688772));
#16=CARTESIAN_POINT('',(2.,-3.4641016151377544));
#17=CARTESIAN_POINT('',(2.,0.));
";

fn verify(records: &str) -> Result<()> {
    let step = format!(
        "ISO-10303-21;HEADER;FILE_DESCRIPTION();FILE_NAME();FILE_SCHEMA();ENDSEC;DATA;
         {records}
         ENDSEC;END-ISO-10303-21;"
    );
    let entities = parse_entities(&step, 1e-7)?;
    Shell::new(&entities, 1e-7, true).verify_planar_circle_master(6, 1, 7)
}

#[test]
fn planar_circle_master_proves_axis_radius_and_complete_traversal() {
    let records = format!("{PLANE_AND_RIM}{CIRCLE}");
    assert_eq!(verify(&records), Ok(()));
    assert_eq!(verify(&records.replace("#9,#10", "#9,$")), Ok(()));
    for (from, to) in [
        ("PLANE('',#2)", "PLANE('',#2,$)"),
        ("CIRCLE('',#2,2.)", "CIRCLE('',#2)"),
        ("CIRCLE('',#2,2.)", "CIRCLE('',#2,0.)"),
        ("CIRCLE('',#8,2.)", "CIRCLE('',#8)"),
        ("CIRCLE('',#8,2.)", "CIRCLE('',#8,-2.)"),
        ("AXIS2_PLACEMENT_2D('',#9,#10)", "AXIS2_PLACEMENT_2D('',#9)"),
        ("(0.,0.));", "(0.,0.000001));"),
        ("(1.,0.));", "(-1.,0.));"),
    ] {
        assert_eq!(
            verify(&records.replace(from, to)),
            Err(ReadError::Invalid),
            "{to}"
        );
    }
}

#[test]
fn rational_circle_master_proves_all_quadratic_spans_and_weights() {
    let records = format!("{PLANE_AND_RIM}{RATIONAL_CIRCLE}");
    assert_eq!(verify(&records), Ok(()));
    for (from, to, expected) in [
        ("RATIONAL_B_SPLINE_CURVE", "UNKNOWN", ReadError::Unsupported),
        (
            "B_SPLINE_CURVE(2,",
            "B_SPLINE_CURVE(3,",
            ReadError::Unsupported,
        ),
        (".UNSPECIFIED.)", ".UNKNOWN.)", ReadError::Unsupported),
        (
            "RATIONAL_B_SPLINE_CURVE((1.",
            "RATIONAL_B_SPLINE_CURVE('',(1.",
            ReadError::Unsupported,
        ),
        ("(1,2,2,2,2,1)", "(2,2,2,2,2,1)", ReadError::Unsupported),
        ("(1,2,2,2,2,1)", "(1,2,1,2,2,1)", ReadError::Unsupported),
        ("(-1.,0.,", "(-1.,$,", ReadError::Invalid),
        ("(-1.,0.,", "(1.,0.,", ReadError::Invalid),
        (
            "(-1.,0.,2.0943951023931953",
            "(-1.,0.,$",
            ReadError::Invalid,
        ),
        (
            "(-1.,0.,2.0943951023931953",
            "(-1.,0.,0.",
            ReadError::Invalid,
        ),
        (
            "((1.,0.5,1.,0.5",
            "((0.9,0.5,1.,0.5",
            ReadError::Unsupported,
        ),
        ("((1.,0.5,1.,0.5", "((1.,0.,1.,0.5", ReadError::Invalid),
        ("((1.,0.5,1.,0.5", "((1.,0.6,1.,0.5", ReadError::Invalid),
        ("3.4641016151377544", "3.46", ReadError::Invalid),
    ] {
        assert_eq!(verify(&records.replace(from, to)), Err(expected), "{to}");
    }
}
