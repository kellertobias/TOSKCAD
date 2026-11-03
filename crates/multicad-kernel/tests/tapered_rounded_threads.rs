use multicad_core::document::EntityId;
use multicad_kernel::{BooleanOperation, RigidTransform, RoundedHelicalThread, Solid};

fn expected_volume(s: RoundedHelicalThread) -> f64 {
    let a = 27.5_f64.to_radians();
    let k = s.radial_taper;
    let m = 1. / a.tan();
    let q = (1. + k * k).sqrt();
    let r = s.rounding_radius_mm;
    let h = s.depth_mm;
    let p = s.pitch_mm;
    let fundamental = p * (m * m - k * k) / (2. * m);
    let left = -fundamental / (m - k);
    let right = fundamental / (m + k);
    let half = r * a.cos();
    let crest_center = -r * q;
    let first = left + k * r / q;
    let samples = 20000;
    // Integrate one independent analytic axial period. Removing k*z makes
    // this radius offset periodic; its first two moments give the full cone
    // volume after exact axial integration, without using a native mesh.
    let mut first_moment = 0.;
    let mut second_moment = 0.;
    for i in 0..=samples {
        let z = first + p * i as f64 / samples as f64;
        let radius = if z < left + half {
            -h + k * left + r * q - (r * r - (z - left).powi(2)).sqrt()
        } else if z < -half {
            crest_center + r * a.sin() + m * (z + half)
        } else if z <= half {
            crest_center + (r * r - z * z).sqrt()
        } else if z < right - half {
            crest_center + r * a.sin() - m * (z - half)
        } else {
            -h + k * right + r * q - (r * r - (z - right).powi(2)).sqrt()
        };
        let f = radius - k * z;
        let weight = if i == 0 || i == samples {
            1.
        } else if i % 2 == 0 {
            2.
        } else {
            4.
        };
        first_moment += weight * f;
        second_moment += weight * f * f;
    }
    let mean = first_moment / (3. * samples as f64);
    let square = second_moment / (3. * samples as f64);
    let length = s.length_mm;
    let radius = s.major_radius_mm;
    std::f64::consts::PI
        * (length * radius * radius
            + radius * k * length * length
            + k * k * length.powi(3) / 3.
            + 2. * mean * (radius * length + k * length * length / 2.)
            + length * square)
}

#[test]
fn sourced_pipe_recipe_extents_keep_all_native_circular_bands() {
    use multicad_core::{commands::Session, document::Document, model::*};
    let session = Session::new(Document::new("Sourced pipe extents").unwrap()).unwrap();
    for size in [PipeThreadSize::OneQuarter, PipeThreadSize::OneEighth] {
        for form in [PipeThreadForm::R, PipeThreadForm::Rc] {
            let intent = PipeThreadIntent {
                form,
                size,
                gauge: PipeGaugeConvention::Iso7Nominal1994,
            };
            let mut p = intent.parameters();
            p.radial_allowance = length(0.005);
            if form == PipeThreadForm::R {
                p.length = length(intent.default_length_mm() + 0.005 * 32.);
            }
            let resolved = p
                .resolve(
                    session.parameters(),
                    if form == PipeThreadForm::R {
                        ThreadMode::NewExternal
                    } else {
                        ThreadMode::CutInternal
                    },
                )
                .unwrap();
            let spec = RoundedHelicalThread {
                major_radius_mm: resolved.major_radius_mm,
                pitch_mm: resolved.pitch_mm,
                length_mm: resolved.length_mm,
                depth_mm: resolved.profile.depth_mm,
                rounding_radius_mm: resolved.profile.rounding_radius_mm.unwrap(),
                flank_angle_deg: 27.5,
                radial_taper: resolved.profile.radial_taper,
                starts: 1,
                left_hand: false,
            };
            let solid =
                Solid::rounded_helical_thread_named(spec, EntityId::new(), Default::default())
                    .unwrap_or_else(|error| panic!("{size:?}/{form:?}: {spec:?}: {error}"));
            let bounds = solid.bounds().unwrap();
            assert!(bounds.min[2].abs() < 16e-7);
            assert!((bounds.max[2] - spec.length_mm).abs() < 16e-7);
            let volume = solid.volume_mm3().unwrap();
            let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            let expected = expected_volume(spec);
            assert!(
                (volume - expected).abs() < 1e-7 * area + 1e-7 * volume,
                "{size:?}/{form:?} volume {volume} vs independent {expected}"
            );
        }
    }
}

fn fraction(solid: &Solid, point: [f64; 3], size: f64) -> f64 {
    let cube = Solid::box_solid([size; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / size.powi(3)
}

#[test]
fn tapered_profile_preserves_axial_circles_flanks_and_gauge_cones() {
    let a = 27.5_f64.to_radians();
    let pitch = 25.4 / 28.;
    let depth = pitch / (3. * a.tan());
    for k in [1. / 32., -1. / 32.] {
        let m = 1. / a.tan();
        let q = (1_f64 + k * k).sqrt();
        let fundamental = pitch * (m * m - k * k) / (2. * m);
        let r = (fundamental - depth) / (2. * (1. / a.sin() - q));
        assert!((fundamental / pitch - 0.960237).abs() < 0.0000005);
        assert!((r / pitch - 0.137278).abs() < 0.0000005);
        let gauge_z = if k > 0. { 4. } else { pitch / 2. };
        let spec = RoundedHelicalThread {
            major_radius_mm: 9.728 / 2. - k * gauge_z,
            pitch_mm: pitch,
            length_mm: 6.5,
            depth_mm: depth,
            rounding_radius_mm: r,
            flank_angle_deg: 27.5,
            radial_taper: k,
            starts: 1,
            left_hand: false,
        };
        let solid =
            Solid::rounded_helical_thread_named(spec, EntityId::new(), Default::default()).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        // Independently probe the sourced gauge cone and a second plane one
        // pitch inward. The point is the circle/cone tangency, not the crest
        // circle center; confusing those datums changes the measured radius.
        for z in [gauge_z, gauge_z + pitch] {
            for valley in [false, true] {
                let center_z = if valley {
                    z - k * r / q - fundamental / (m + k)
                } else {
                    z + k * r / q
                };
                let theta = std::f64::consts::TAU * center_z / pitch;
                let radius = 9.728 / 2. + k * (z - gauge_z) - if valley { depth } else { 0. };
                for (offset, expected) in [(-0.001, true), (0.001, false)] {
                    let measured = fraction(
                        &solid,
                        [
                            (radius + offset) * theta.cos(),
                            (radius + offset) * theta.sin(),
                            z,
                        ],
                        0.0002,
                    );
                    assert!(
                        if expected {
                            measured > 0.999
                        } else {
                            measured < 1e-5
                        },
                        "gauge/taper mismatch k={k} z={z} valley={valley}"
                    );
                }
            }
        }
        let half = r * a.cos();
        // Independent analytic meridian: centers translate along the taper,
        // but each arc remains a circle, with unsheared +/-cot(alpha) flanks.
        for turn in [2., 4.] {
            let z = turn * pitch;
            let crest_center = spec.major_radius_mm + k * z - r * q;
            let root_z = z + fundamental / (m + k);
            let root_center = spec.major_radius_mm - depth + k * root_z + r * q;
            for (axial, radius) in [
                (
                    z - 0.65 * half,
                    crest_center + (r * r - (0.65 * half).powi(2)).sqrt(),
                ),
                (
                    z + 0.65 * half,
                    crest_center + (r * r - (0.65 * half).powi(2)).sqrt(),
                ),
                (
                    root_z - 0.65 * half,
                    root_center - (r * r - (0.65 * half).powi(2)).sqrt(),
                ),
                (
                    root_z + 0.65 * half,
                    root_center - (r * r - (0.65 * half).powi(2)).sqrt(),
                ),
                (
                    z + 0.25 * pitch,
                    crest_center + r * a.sin() - m * (0.25 * pitch - half),
                ),
            ] {
                let epsilon = 0.001;
                assert!(
                    fraction(&solid, [radius - epsilon, 0., axial], epsilon / 5.) > 0.999,
                    "missing material k={k} z={axial} r={radius}"
                );
                assert!(
                    fraction(&solid, [radius + epsilon, 0., axial], epsilon / 5.) < 1e-5,
                    "excess material k={k} z={axial} r={radius}"
                );
            }
        }
    }
}
