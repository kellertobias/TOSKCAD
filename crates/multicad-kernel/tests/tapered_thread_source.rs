//! Fresh Native witnesses for the retained legacy Custom taper convention.
#![cfg(feature = "occt")]
use multicad_core::document::EntityId;
use multicad_kernel::rust_kernel::helical_thread::{Spec, Thread};
use multicad_kernel::{GeometryPolicy, HelicalThread, Solid};
use std::f64::consts::{PI, TAU};
fn transported_radius(s: HelicalThread, phase: f64, z: f64) -> f64 {
    let u = if phase <= s.pitch_mm / 2. {
        phase
    } else {
        phase - s.pitch_mm
    };
    let root = s.major_radius_mm - s.depth_mm;
    let lead = s.lead_mm();
    let rate = if s.left_hand { -lead / TAU } else { lead / TAU };
    let slope = if u < 0. {
        s.lower_flank_deg
    } else {
        s.upper_flank_deg
    }
    .to_radians()
    .tan();
    let foot = s.crest_width_mm / 2. + (s.depth_mm + 0.02 * s.pitch_mm) * slope;
    if u.abs() > foot + 0.001 {
        return root + s.radial_taper * z;
    }
    let mut delta = u;
    let mut radius = root + s.radial_taper * z;
    for _ in 0..8 {
        let center_z = z - delta;
        let center_radius = root + s.radial_taper * center_z;
        let psi = (s.radial_taper * rate / (root - s.radial_taper * lead)).atan()
            - (s.radial_taper * rate / center_radius).atan();
        let profile = s.major_radius_mm - (delta.abs() - s.crest_width_mm / 2.).max(0.) / slope;
        let dr = profile - root + s.radial_taper * delta;
        let (sin, cos) = psi.sin_cos();
        let re = center_radius + dr * cos;
        let im = dr * sin;
        radius = re.hypot(im);
        delta = u + rate * im.atan2(re);
    }
    radius.max(root + s.radial_taper * z)
}
fn transported_volume(s: HelicalThread) -> f64 {
    let nu = 8192;
    let nz = 128;
    let mut total = 0.;
    for iz in 0..=nz {
        let z = s.length_mm * iz as f64 / nz as f64;
        let mut section = 0.;
        for iu in 0..=nu {
            let u = s.pitch_mm * iu as f64 / nu as f64;
            let r = transported_radius(s, u, z);
            let w = if iu == 0 || iu == nu {
                1.
            } else if iu % 2 == 0 {
                2.
            } else {
                4.
            };
            section += w * r * r;
        }
        section *= PI / (3. * nu as f64);
        let w = if iz == 0 || iz == nz {
            1.
        } else if iz % 2 == 0 {
            2.
        } else {
            4.
        };
        total += w * section;
    }
    total * s.length_mm / (3. * nz as f64)
}
fn ideal_volume(s: HelicalThread) -> f64 {
    let root = s.major_radius_mm - s.depth_mm;
    let d = s.depth_mm;
    let slope = s.lower_flank_deg.to_radians().tan() + s.upper_flank_deg.to_radians().tan();
    let mean = root + (s.crest_width_mm * d + slope * d * d / 2.) / s.pitch_mm;
    let square = root * root
        + (s.crest_width_mm * (2. * root * d + d * d) + slope * (root * d * d + d.powi(3) / 3.))
            / s.pitch_mm;
    PI * (s.length_mm * square
        + s.radial_taper * mean * s.length_mm.powi(2)
        + s.radial_taper.powi(2) * s.length_mm.powi(3) / 3.)
}
#[test]
fn fresh_native_custom_transport_witness_source() {
    for k in [0., -0.1, -0.03125, 0.03125, 0.1] {
        for starts in [1, 3] {
            let native = HelicalThread {
                major_radius_mm: 4.,
                pitch_mm: 1.25,
                length_mm: 8.,
                depth_mm: 0.5,
                crest_width_mm: 0.25,
                lower_flank_deg: 15.,
                upper_flank_deg: 15.,
                radial_taper: k,
                starts,
                left_hand: false,
            };
            let _base = Thread::new(Spec {
                major_radius_mm: native.major_radius_mm,
                pitch_mm: native.pitch_mm,
                length_mm: native.length_mm,
                depth_mm: native.depth_mm,
                crest_width_mm: native.crest_width_mm,
                lower_flank_deg: native.lower_flank_deg,
                upper_flank_deg: native.upper_flank_deg,
                starts,
                left_hand: false,
                root_radius_mm: None,
            })
            .unwrap();
            let now = std::time::Instant::now();
            let shape = Solid::helical_thread_named_fresh(
                native,
                EntityId::new(),
                GeometryPolicy::default(),
            )
            .unwrap();
            let mut components = shape.solid_components(GeometryPolicy::default()).unwrap();
            assert_eq!(components.len(), 1);
            let shape = components.remove(0);
            assert_eq!(shape.solid_count().unwrap(), 1);
            let volume = shape.volume_mm3().unwrap();
            let expected = ideal_volume(native);
            eprintln!(
                "CASE k={k} starts={starts} seconds={:.3} native_volume={volume:.12} additive_volume={expected:.12} delta={:.12}",
                now.elapsed().as_secs_f64(),
                volume - expected
            );
            let transported = transported_volume(native);
            eprintln!(
                "TRANSPORT k={k} starts={starts} predicted_volume={transported:.12} native_delta={:.12}",
                volume - transported
            );
            if k == -0.1 && starts == 3 {
                // Midpoints between the rejected additive law and independently
                // classified Native boundaries. Both boxes are wholly on one
                // side, avoiding a mesh or an uncertain near-surface probe.
                for (phase, radius, expected_inside) in [
                    (0.21, 3.00243962095187, false),
                    (1.04, 3.00322311423228, true),
                ] {
                    let theta = (6.8 - phase) / (native.lead_mm() / TAU);
                    let edge: f64 = 0.00005;
                    let center = [radius * theta.cos(), radius * theta.sin(), 6.8];
                    let cube = Solid::box_solid([edge; 3], GeometryPolicy::default())
                        .unwrap()
                        .transformed(multicad_kernel::RigidTransform {
                            translation_mm: center.map(|v| v - edge / 2.),
                            ..Default::default()
                        })
                        .unwrap();
                    let volume = shape
                        .boolean(
                            &cube,
                            multicad_kernel::BooleanOperation::Intersect,
                            GeometryPolicy::default(),
                        )
                        .unwrap()
                        .map_or(0., |s| s.volume_mm3().unwrap());
                    assert!(if expected_inside {
                        volume / edge.powi(3) > 0.99
                    } else {
                        volume == 0.
                    });
                }
            }
            let path =
                std::path::PathBuf::from(format!("/tmp/multicad-taper-custom-{k}-{starts}.step"));
            shape.write_step(&path).unwrap();
            eprintln!("STEP k={k} starts={starts} path={}", path.display());
        }
    }
}

#[test]
fn owned_transport_matches_fresh_native_signed_taper_asymmetric_hands() {
    use multicad_kernel::rust_kernel::tapered_thread::Thread as Tapered;
    use multicad_kernel::{BooleanOperation, RigidTransform};
    let policy = GeometryPolicy::default();
    for (k, left, starts) in [
        (0.1, false, 1),
        (-0.1, true, 1),
        (-0.03125, false, 3),
        (0.03125, true, 3),
    ] {
        let source = Spec {
            major_radius_mm: 4.,
            pitch_mm: 1.25,
            length_mm: 8.,
            depth_mm: 0.5,
            crest_width_mm: 0.25,
            lower_flank_deg: 12.,
            upper_flank_deg: 25.,
            starts,
            left_hand: left,
            root_radius_mm: None,
        };
        let owned = Tapered::new(
            source,
            k,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .unwrap();
        let parameters = HelicalThread {
            major_radius_mm: source.major_radius_mm,
            pitch_mm: source.pitch_mm,
            length_mm: source.length_mm,
            depth_mm: source.depth_mm,
            crest_width_mm: source.crest_width_mm,
            lower_flank_deg: source.lower_flank_deg,
            upper_flank_deg: source.upper_flank_deg,
            radial_taper: k,
            starts,
            left_hand: left,
        };
        let native =
            Solid::helical_thread_named_fresh(parameters, EntityId::new(), policy).unwrap();
        let mut parts = native.solid_components(policy).unwrap();
        assert_eq!(parts.len(), 1);
        let native = parts.remove(0);
        let [lo, hi] = owned.volume_bounds(0.01).unwrap();
        let volume = native.volume_mm3().unwrap();
        assert!(
            volume >= lo - 1e-4 && volume <= hi + 1e-4,
            "{k} {left} {starts} {lo} {volume} {hi}"
        );
        for (z, phase) in [(2., 0.21), (6., 1.04)] {
            let [lo, hi] = owned.radial_bounds(phase, z, 1e-10).unwrap();
            let boundary = (lo + hi) / 2.;
            let rate = if left { -1. } else { 1. } * parameters.lead_mm() / TAU;
            let theta = (z - phase) / rate;
            for (offset, inside) in [(-0.0003, true), (0.0003, false)] {
                let radius = boundary + offset;
                let edge: f64 = 0.00005;
                let center = [radius * theta.cos(), radius * theta.sin(), z];
                let cube = Solid::box_solid([edge; 3], policy)
                    .unwrap()
                    .transformed(RigidTransform {
                        translation_mm: center.map(|v| v - edge / 2.),
                        ..Default::default()
                    })
                    .unwrap();
                let overlap = native
                    .boolean(&cube, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .map_or(0., |s| s.volume_mm3().unwrap());
                assert!(
                    if inside {
                        overlap / edge.powi(3) > 0.99
                    } else {
                        overlap == 0.
                    },
                    "{k} {left} {starts} {phase} offset{offset} overlap{overlap}"
                );
            }
        }
    }
}
