use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    model::{Direction, ExtrudeParameters, RevolveParameters, angle},
    parameters,
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn rad(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "rad".into(),
    }
}
#[test]
fn perpendicular_extrusion_is_exact_in_degrees_and_radians_without_source_rewrite() {
    let parameters = parameters::evaluate(&Document::new("Cardinal controls").unwrap()).unwrap();
    for elevation in [angle(90.), rad(FRAC_PI_2)] {
        for azimuth in [angle(0.), angle(90.), angle(180.), rad(3. * FRAC_PI_2)] {
            for reverse in [false, true] {
                let source = ExtrudeParameters {
                    direction: Direction {
                        elevation: elevation.clone(),
                        azimuth: azimuth.clone(),
                        reverse,
                    },
                    ..Default::default()
                };
                let original = serde_json::to_vec(&source).unwrap();
                assert_eq!(
                    source.resolve_controls(&parameters).unwrap().direction,
                    [0., 0., if reverse { -1. } else { 1. }]
                );
                assert_eq!(serde_json::to_vec(&source).unwrap(), original);
            }
        }
    }
}
#[test]
fn exact_bearings_include_reduced_turns_but_near_cardinals_are_not_snapped() {
    let parameters = parameters::evaluate(&Document::new("Axis controls").unwrap()).unwrap();
    for (degrees, expected) in [
        (0., [1., 0.]),
        (90., [0., 1.]),
        (180., [-1., 0.]),
        (270., [0., -1.]),
        (360., [1., 0.]),
        (-90., [0., -1.]),
    ] {
        let source = RevolveParameters {
            axis_angle: angle(degrees),
            ..Default::default()
        };
        assert_eq!(
            source.resolve(&parameters).unwrap().axis_direction,
            expected
        );
    }
    for (bearing, expected) in [
        (0., [1., 0.]),
        (FRAC_PI_2, [0., 1.]),
        (PI, [-1., 0.]),
        (3. * FRAC_PI_2, [0., -1.]),
        (TAU, [1., 0.]),
        (-FRAC_PI_2, [0., -1.]),
        (TAU + FRAC_PI_2, [0., 1.]),
    ] {
        let source = RevolveParameters {
            axis_angle: rad(bearing),
            ..Default::default()
        };
        assert_eq!(
            source.resolve(&parameters).unwrap().axis_direction,
            expected
        );
        let sweep = ExtrudeParameters {
            direction: Direction {
                elevation: angle(45.),
                azimuth: rad(bearing),
                reverse: false,
            },
            ..Default::default()
        };
        let resolved = sweep.resolve_controls(&parameters).unwrap().direction;
        assert_eq!(resolved[0], (PI / 4.).cos() * expected[0]);
        assert_eq!(resolved[1], (PI / 4.).cos() * expected[1]);
        assert_eq!(resolved[2], (PI / 4.).sin());
    }
    for bearing in [
        89.9999_f64.to_radians(),
        f64::from_bits(FRAC_PI_2.to_bits() - 1),
        f64::from_bits(FRAC_PI_2.to_bits() + 1),
    ] {
        let source = RevolveParameters {
            axis_angle: rad(bearing),
            ..Default::default()
        };
        assert_eq!(
            source.resolve(&parameters).unwrap().axis_direction,
            [bearing.cos(), bearing.sin()]
        );
        assert_ne!(source.resolve(&parameters).unwrap().axis_direction[0], 0.);
        let source = ExtrudeParameters {
            direction: Direction {
                elevation: rad(bearing),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            source.resolve_controls(&parameters).unwrap().direction,
            [bearing.cos(), 0., bearing.sin()]
        );
    }
}
