use multicad_core::document::EntityId;
use multicad_kernel::{
    BooleanOperation, GeometryPolicy, HelicalThread, RigidTransform, RootRoundedHelicalThread,
    Solid,
};
fn material(solid: &Solid, point: [f64; 3]) -> f64 {
    // On a 3-degree flank a radial displacement has a small normal
    // component. Keep the probe wholly on its intended side of that plane.
    let edge = 0.00002;
    let cube = Solid::box_solid([edge; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|x| x - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / edge.powi(3)
}
fn spec(left_hand: bool, reverse: bool) -> RootRoundedHelicalThread {
    let pitch = 4.;
    let major = 10.;
    let radius = 0.12427 * pitch;
    let (lower, upper) = if reverse {
        (30_f64, 3_f64)
    } else {
        (3_f64, 30_f64)
    };
    let lo = lower.to_radians();
    let hi = upper.to_radians();
    let sum = lo.tan() + hi.tan();
    let width = (0.5 - 0.375 * sum) * pitch;
    let intersection = (pitch - width) / sum;
    let half = (lo + hi) / 2.;
    let tilt = (hi - lo) / 2.;
    let depth = intersection - radius * (tilt.cos() / half.sin() - 1.);
    RootRoundedHelicalThread {
        profile: HelicalThread {
            major_radius_mm: major,
            pitch_mm: pitch,
            length_mm: 2.37 * pitch,
            depth_mm: depth,
            crest_width_mm: width,
            lower_flank_deg: lower,
            upper_flank_deg: upper,
            radial_taper: 0.,
            starts: 1,
            left_hand,
        },
        root_radius_mm: radius,
    }
}
#[test]
fn asymmetric_root_is_a_true_tangent_circle_for_each_hand_and_axial_orientation() {
    let dir = tempfile::tempdir().unwrap();
    for left in [false, true] {
        for reverse in [false, true] {
            let spec = spec(left, reverse);
            let p = spec.profile;
            let r = spec.root_radius_mm;
            let lo = p.lower_flank_deg.to_radians();
            let hi = p.upper_flank_deg.to_radians();
            let intersection = (p.pitch_mm - p.crest_width_mm) / (lo.tan() + hi.tan());
            let half = (lo + hi) / 2.;
            let tilt = (hi - lo) / 2.;
            let cz = p.crest_width_mm / 2. + intersection * hi.tan() - r * tilt.sin() / half.sin();
            let cr = p.major_radius_mm - intersection + r * tilt.cos() / half.sin();
            let feature = EntityId::new();
            let solid = Solid::root_rounded_helical_thread_named(spec, feature, Default::default())
                .unwrap_or_else(|e| panic!("left={left},reverse={reverse}: {e}"));
            assert_eq!(solid.solid_count().unwrap(), 1);
            let bounds = solid.bounds().unwrap();
            assert!(bounds.min[2].abs() < 16e-7);
            assert!((bounds.max[2] - p.length_mm).abs() < 16e-7);
            let names = solid.topology_snapshot(feature, 0).unwrap();
            assert!(names.elements.iter().all(|e| !e.origins.is_empty()));
            for side in ["lower", "upper"] {
                let role = format!("thread/start/0/root/{side}");
                assert!(names.elements.iter().any(|e| {
                    e.origins
                        .iter()
                        .any(|o| o.source == feature && o.role == role)
                }));
            }
            let path = dir.path().join(format!("root-{left}-{reverse}.step"));
            solid.write_step(&path).unwrap();
            let back = Solid::read_step(&path, Default::default()).unwrap();
            let left_angle = -(std::f64::consts::FRAC_PI_2 - hi);
            let right_angle = std::f64::consts::FRAC_PI_2 - lo;
            for shape in [&solid, &back] {
                let z = 1.37 * p.pitch_mm;
                let hand = if left { -1. } else { 1. };
                for angle in [
                    left_angle * 0.8,
                    left_angle * 0.4,
                    0.,
                    right_angle * 0.4,
                    right_angle * 0.8,
                ] {
                    let relative = cz + r * angle.sin();
                    let boundary = cr - r * angle.cos();
                    let phi = hand * std::f64::consts::TAU * (z - relative) / p.pitch_mm;
                    for (offset, inside) in [(-0.001, true), (0.001, false)] {
                        let radius = boundary + offset;
                        let fraction = material(shape, [radius * phi.cos(), radius * phi.sin(), z]);
                        assert!(
                            if inside {
                                fraction > 0.999
                            } else {
                                fraction < 1e-5
                            },
                            "circle {left}/{reverse}/{angle}/{offset}: {fraction}"
                        );
                    }
                }
                for (direction, angle) in [(-1., lo), (1., hi)] {
                    for drop in [0.2 * p.pitch_mm, 0.5 * p.pitch_mm] {
                        let relative = direction * (p.crest_width_mm / 2. + drop * angle.tan());
                        let boundary = p.major_radius_mm - drop;
                        let phi = hand * std::f64::consts::TAU * (z - relative) / p.pitch_mm;
                        for (offset, inside) in [(-0.001, true), (0.001, false)] {
                            let radius = boundary + offset;
                            let fraction =
                                material(shape, [radius * phi.cos(), radius * phi.sin(), z]);
                            assert!(if inside {
                                fraction > 0.999
                            } else {
                                fraction < 1e-5
                            });
                        }
                    }
                }
            }
            // Independent phase integral of the actual external radial boundary.
            let x0 = r * left_angle.sin();
            let x1 = r * right_angle.sin();
            let primitive = |x: f64| {
                (cr * cr + r * r) * x
                    - x.powi(3) / 3.
                    - cr * (x * (r * r - x * x).sqrt() + r * r * (x / r).asin())
            };
            let rb = cr - r * hi.sin();
            let rt = cr - r * lo.sin();
            let squared = p.crest_width_mm * p.major_radius_mm.powi(2)
                + hi.tan() * (p.major_radius_mm.powi(3) - rb.powi(3)) / 3.
                + primitive(x1)
                - primitive(x0)
                + lo.tan() * (p.major_radius_mm.powi(3) - rt.powi(3)) / 3.;
            let expected = std::f64::consts::PI * p.length_mm * squared / p.pitch_mm;
            let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            let budget = GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * expected;
            for shape in [&solid, &back] {
                assert!(
                    (shape.volume_mm3().unwrap() - expected).abs() < budget,
                    "native {}, independent {expected}, budget {budget}",
                    shape.volume_mm3().unwrap()
                );
            }
        }
    }
}
#[test]
fn invalid_root_forms_do_not_enter_the_native_constructor() {
    let good = spec(false, false);
    for bad in [
        RootRoundedHelicalThread {
            root_radius_mm: 0.,
            ..good
        },
        RootRoundedHelicalThread {
            profile: HelicalThread {
                starts: 2,
                ..good.profile
            },
            ..good
        },
        RootRoundedHelicalThread {
            profile: HelicalThread {
                radial_taper: 1. / 32.,
                ..good.profile
            },
            ..good
        },
        RootRoundedHelicalThread {
            profile: HelicalThread {
                depth_mm: good.profile.depth_mm + 0.0001,
                ..good.profile
            },
            ..good
        },
    ] {
        assert!(
            Solid::root_rounded_helical_thread_named(bad, EntityId::new(), Default::default())
                .is_err()
        );
    }
}
