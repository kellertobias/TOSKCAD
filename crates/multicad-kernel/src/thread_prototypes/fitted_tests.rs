use super::*;
use crate::{BooleanOperation, CurvedEdge2d, CurvedProfile, PlaneFrame, RigidTransform};
use multicad_core::model::{self, MetricFitEngagement, ThreadFit, ThreadMode};
fn fitted_source(fit: ThreadFit, length: f64, mode: ThreadMode) -> model::ResolvedThread {
    let doc = multicad_core::document::Document::new("Canonical fitted prototype source").unwrap();
    let values = multicad_core::parameters::evaluate(&doc).unwrap();
    model::ThreadParameters {
        major_diameter: model::length(3.),
        pitch: model::length(0.5),
        length: model::length(length),
        fit,
        metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
        ..Default::default()
    }
    .resolve(&values, mode)
    .unwrap()
}
fn profile(r: &model::ResolvedThread) -> HelicalThread {
    HelicalThread {
        major_radius_mm: r.major_radius_mm,
        pitch_mm: r.pitch_mm,
        length_mm: r.length_mm,
        depth_mm: r.profile.depth_mm,
        crest_width_mm: r.profile.crest_width_mm,
        lower_flank_deg: r.profile.lower_flank_deg,
        upper_flank_deg: r.profile.upper_flank_deg,
        radial_taper: r.profile.radial_taper,
        starts: r.starts,
        left_hand: false,
    }
}
fn root_spec() -> RootRoundedHelicalThread {
    root_spec_for_length(8.)
}
fn root_spec_for_length(length: f64) -> RootRoundedHelicalThread {
    let r = fitted_source(ThreadFit::Metric6gExternal, length, ThreadMode::NewExternal);
    RootRoundedHelicalThread {
        profile: profile(&r),
        root_radius_mm: r.metric_fit.unwrap().external_root_radius_mm.unwrap(),
    }
}
fn fake_entry(data: &ffi::ThreadData, policy: GeometryPolicy) -> RootRegistration {
    RootRegistration {
        kind: RootKind::SymmetricRounded,
        inputs: root_inputs(data),
        starts: data.starts,
        left: data.left,
        axial_flat: data.axial_flat,
        policy: policy_bits(policy),
        source: "test-source",
        brep: "",
        receipt: Receipt {
            sha: "",
            bounds: [[0.; 3]; 2],
            volume: 1.,
            seeds: &[],
            faces: &[],
        },
    }
}
#[test]
fn fitted_root_key_covers_every_native_bit_kind_policy_and_source() {
    let policy = GeometryPolicy::default();
    let spec = root_spec();
    let data = spec.checked(policy).unwrap();
    let entry = fake_entry(&data, policy);
    assert!(root_matches(&entry, &data, policy, "test-source"));
    assert!(!root_matches(&entry, &data, policy, "stale-source"));
    assert_ne!(
        root_rounded_key(spec, policy).unwrap(),
        key(spec.profile, policy).unwrap()
    );
    macro_rules! changed {
        ($field:ident) => {{
            let mut d = spec.checked(policy).unwrap();
            d.$field = f64::from_bits(d.$field.to_bits() + 1);
            assert_ne!(root_inputs(&d), root_inputs(&data), stringify!($field));
            assert!(
                !root_matches(&entry, &d, policy, "test-source"),
                stringify!($field)
            );
        }};
    }
    changed!(radius);
    changed!(pitch);
    changed!(height);
    changed!(depth);
    changed!(crest_width);
    changed!(rounding);
    changed!(lower_slope);
    changed!(upper_slope);
    changed!(taper);
    changed!(overlap);
    changed!(tolerance);
    changed!(crest_limit);
    changed!(root_rounding);
    changed!(knuckle_root_radius);
    let mut d = spec.checked(policy).unwrap();
    d.starts = 2;
    assert!(!root_matches(&entry, &d, policy, "test-source"));
    let mut d = spec.checked(policy).unwrap();
    d.left = true;
    assert!(!root_matches(&entry, &d, policy, "test-source"));
    let mut d = spec.checked(policy).unwrap();
    d.axial_flat = true;
    assert!(!root_matches(&entry, &d, policy, "test-source"));
    for i in 0..4 {
        let mut p = policy;
        let target = match i {
            0 => &mut p.linear_tolerance_mm,
            1 => &mut p.boolean_fuzzy_mm,
            2 => &mut p.minimum_feature_mm,
            _ => &mut p.maximum_coordinate_mm,
        };
        *target = f64::from_bits(target.to_bits() + 1);
        assert!(!root_matches(&entry, &data, p, "test-source"));
    }
    let mut invalid = spec;
    invalid.root_radius_mm *= 1.1;
    assert!(root_rounded_key(invalid, policy).is_err());
    assert!(Solid::root_rounded_helical_thread_named(spec, EntityId::nil(), policy).is_err());
    assert!(Solid::root_rounded_helical_thread_named_fresh(spec, EntityId::nil(), policy).is_err());
    let mut flat = spec.checked(policy).unwrap();
    flat.root_rounding = 0.;
    assert!(!symmetric_root(&flat));
    let mut asymmetric = spec.checked(policy).unwrap();
    asymmetric.lower_slope = 3_f64.to_radians().tan();
    assert!(!symmetric_root(&asymmetric));
}
#[test]
fn fitted_root_forced_fresh_generation_preserves_roundtrip_roles_without_retention() {
    ffi::reset_thread_factory_cache().unwrap();
    let result =
        generate_root_rounded(root_spec(), EntityId::new(), GeometryPolicy::default()).unwrap();
    let stats = crate::thread_factory_stats().unwrap();
    assert_eq!(stats.constructions, 1);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.entries, 0);
    assert!(
        result
            .registration
            .starts_with("RootRegistration { kind: RootKind::SymmetricRounded")
    );
    assert!(
        result
            .registration
            .contains("include_str!(\"__BREP_FILE__\")")
    );
    assert!(result.volume_mm3 > 0. && result.face_count > 0);
}
fn material(s: &Solid, point: [f64; 3]) -> f64 {
    let edge: f64 = 0.00002;
    let cube = Solid::box_solid([edge; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|v| v - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    s.boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / edge.powi(3)
}
fn root_probes(s: &Solid, spec: RootRoundedHelicalThread) {
    let cr = spec.profile.major_radius_mm - spec.profile.depth_mm + spec.root_radius_mm;
    for angle in [-0.4_f64, 0., 0.4] {
        let z = 2.37 * spec.profile.pitch_mm;
        let relative = spec.profile.pitch_mm / 2. + spec.root_radius_mm * angle.sin();
        let boundary = cr - spec.root_radius_mm * angle.cos();
        let phi = std::f64::consts::TAU * (z - relative) / spec.profile.pitch_mm;
        for (offset, inside) in [(-0.001, true), (0.001, false)] {
            let radial = boundary + offset;
            let value = material(s, [radial * phi.cos(), radial * phi.sin(), z]);
            assert!(
                if inside { value > 0.999 } else { value < 1e-5 },
                "root{angle}/{offset}/{value}"
            );
        }
    }
}
fn analytic_volume(profile: HelicalThread, round: Option<f64>) -> f64 {
    let r = profile.major_radius_mm;
    let root = r - profile.depth_mm;
    let integral = if let Some(radius) = round {
        let center = root + radius;
        let x = radius * (std::f64::consts::PI / 3.).sin();
        let primitive = |x: f64| {
            (center * center + radius * radius) * x
                - x.powi(3) / 3.
                - center
                    * (x * (radius * radius - x * x).sqrt() + radius * radius * (x / radius).asin())
        };
        let tangent = center - radius / 2.;
        profile.crest_width_mm * r * r
            + 2. / 3_f64.sqrt() * (r.powi(3) - tangent.powi(3)) / 3.
            + primitive(x)
            - primitive(-x)
    } else {
        let slopes =
            profile.lower_flank_deg.to_radians().tan() + profile.upper_flank_deg.to_radians().tan();
        let gap = profile.pitch_mm - profile.crest_width_mm - profile.depth_mm * slopes;
        profile.crest_width_mm * r * r
            + gap * root * root
            + slopes * (r.powi(3) - root.powi(3)) / 3.
    };
    std::f64::consts::PI * profile.length_mm / profile.pitch_mm * integral
}
fn volume_matches(shape: &Solid, expected: f64, policy: GeometryPolicy) {
    let actual = shape.volume_mm3().unwrap();
    let area = shape
        .faces()
        .unwrap()
        .iter()
        .map(|f| f.area_mm2)
        .sum::<f64>();
    assert!(
        (actual - expected).abs() <= policy.linear_tolerance_mm * area + 1e-7 * actual,
        "native/analytic {actual}/{expected}"
    );
}
fn flat_probes(shape: &Solid, spec: HelicalThread) {
    for (relative, boundary) in [
        (0., spec.major_radius_mm),
        (spec.pitch_mm / 2., spec.major_radius_mm - spec.depth_mm),
    ] {
        let z = 2.37 * spec.pitch_mm;
        let phi = std::f64::consts::TAU * (z - relative) / spec.pitch_mm;
        for (offset, inside) in [(-0.001, true), (0.001, false)] {
            let radius = boundary + offset;
            let value = material(shape, [radius * phi.cos(), radius * phi.sin(), z]);
            assert!(
                if inside { value > 0.999 } else { value < 1e-5 },
                "flat{relative}/{offset}/{value}"
            );
        }
    }
}
fn same_topology(a: &Solid, b: &Solid, owner: EntityId) {
    let a = a.topology_snapshot(owner, 0).unwrap();
    let b = b.topology_snapshot(owner, 0).unwrap();
    assert_eq!(a.elements.len(), b.elements.len());
    for (a, b) in a.elements.iter().zip(&b.elements) {
        assert_eq!(a.key, b.key);
        assert_eq!(a.origins, b.origins);
        assert_eq!(a.boundary, b.boundary);
    }
}
#[test]
fn fitted_registered_assets_preserve_native_mass_profiles_step_and_owner_lifetimes() {
    let policy = GeometryPolicy::default();
    let spec = root_spec();
    let owner = EntityId::new();
    ffi::reset_thread_factory_cache().unwrap();
    let first = Solid::root_rounded_helical_thread_named(spec, owner, policy).unwrap();
    assert_eq!(
        crate::thread_factory_stats().unwrap().constructions,
        0,
        "Freshly regenerate fitted assets after kernel source changes"
    );
    let original = first.to_brep().unwrap();
    let original_digest = first.exact_geometry_digest().unwrap();
    let entry =
        root_entry(&spec.checked(policy).unwrap(), policy).expect("Compiled current root receipt");
    let duplicate = {
        let mut seeds = entry.receipt.seeds.to_vec();
        seeds.push(seeds[0]);
        seeds
    };
    assert!(
        restore(
            entry.brep,
            &Receipt {
                seeds: &duplicate,
                ..entry.receipt
            },
            policy
        )
        .is_err()
    );
    assert!(restore(&format!("{}corrupt", entry.brep), &entry.receipt, policy).is_err());
    ffi::reset_thread_factory_cache().unwrap();
    let imported = Solid::from_brep(entry.brep, policy).unwrap();
    assert!(!ffi::remember_validated_thread(imported.occt().unwrap()).unwrap());
    assert_eq!(crate::thread_factory_stats().unwrap().entries, 0);
    // Re-establish an actual owner-thread hit after the separate import control.
    let _retained = Solid::root_rounded_helical_thread_named(spec, owner, policy).unwrap();

    let other = EntityId::new();
    let second = Solid::root_rounded_helical_thread_named(spec, other, policy).unwrap();
    assert_eq!(crate::thread_factory_stats().unwrap().hits, 1);
    for e in second.topology_snapshot(other, 0).unwrap().elements {
        assert!(e.origins.iter().all(|o| o.source == other));
    }
    let fresh = Solid::root_rounded_helical_thread_named_fresh(spec, owner, policy).unwrap();
    same_topology(&fresh, &first, owner);
    root_probes(&fresh, spec);
    // COMMON may mark an operand's root non-free. Exercise the exact loaded
    // payload on a separate native restore so the cached owner immutability
    // check below still compares every stored bit across head construction.
    let loaded_for_probes = restore(entry.brep, &entry.receipt, policy).unwrap();
    root_probes(&loaded_for_probes, spec);
    let expected = analytic_volume(spec.profile, Some(spec.root_radius_mm));
    volume_matches(&fresh, expected, policy);
    volume_matches(&first, expected, policy);

    let dir = tempfile::tempdir().unwrap();
    for (name, solid) in [("fresh", &fresh), ("loaded", &first)] {
        let path = dir.path().join(format!("{name}.step"));
        solid.write_step(&path).unwrap();
        let back = Solid::read_step(&path, policy).unwrap();
        let area = solid
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>();
        let v = solid.volume_mm3().unwrap();
        let actual = back.volume_mm3().unwrap();
        assert!(
            (v - actual).abs()
                <= 1e-7 * (v.abs() + actual.abs()) + 2. * policy.linear_tolerance_mm * area
        );
        assert_eq!(back.solid_count().unwrap(), 1);
        root_probes(&back, spec);
        volume_matches(&back, expected, policy);
        for (a, b) in back
            .bounds()
            .unwrap()
            .min
            .into_iter()
            .chain(back.bounds().unwrap().max)
            .zip(
                solid
                    .bounds()
                    .unwrap()
                    .min
                    .into_iter()
                    .chain(solid.bounds().unwrap().max),
            )
        {
            assert!((a - b).abs() <= 16. * policy.linear_tolerance_mm);
        }
    }
    let head_owner = EntityId::new();
    let head = Solid::extrude_curved_profile_named(
        &CurvedProfile {
            frame: PlaneFrame {
                origin: [0., 0., -3.],
                ..Default::default()
            },
            outer: vec![CurvedEdge2d::Circle {
                center: [0., 0.],
                radius_mm: 2.75,
            }],
            holes: vec![],
        },
        3.,
        head_owner,
        &[multicad_core::references::Origin {
            source: head_owner,
            role: "head.outer".into(),
        }],
        policy,
    )
    .unwrap();
    let second_digest = second.exact_geometry_digest().unwrap();
    let combined = second
        .boolean(&head, BooleanOperation::Union, policy)
        .unwrap()
        .unwrap();
    assert_eq!(combined.solid_count().unwrap(), 1);
    combined
        .write_step(&dir.path().join("headed-source-immutability.step"))
        .unwrap();
    assert_eq!(first.exact_geometry_digest().unwrap(), original_digest);
    assert_eq!(second.exact_geometry_digest().unwrap(), second_digest);
    assert_eq!(first.to_brep().unwrap(), original);
    assert_eq!(second.to_brep().unwrap(), original);
    for length in [2.4, 2.5] {
        ffi::reset_thread_factory_cache().unwrap();
        let r = fitted_source(ThreadFit::Metric6HInternal, length, ThreadMode::CutInternal);
        let flat = profile(&r);
        let loaded = Solid::helical_thread_named(flat, owner, policy).unwrap();
        assert_eq!(crate::thread_factory_stats().unwrap().constructions, 0);
        let fresh = Solid::helical_thread_named_fresh(flat, owner, policy).unwrap();
        same_topology(&loaded, &fresh, owner);
        assert!((loaded.volume_mm3().unwrap() - fresh.volume_mm3().unwrap()).abs() < 1e-7);
        let expected = analytic_volume(flat, None);
        for (suffix, shape) in [("fresh", &fresh), ("loaded", &loaded)] {
            volume_matches(shape, expected, policy);
            flat_probes(shape, flat);
            let path = dir.path().join(format!("internal-{length}-{suffix}.step"));
            shape.write_step(&path).unwrap();
            let back = Solid::read_step(&path, policy).unwrap();
            volume_matches(&back, expected, policy);
            flat_probes(&back, flat);
            assert_eq!(back.solid_count().unwrap(), 1);
        }
    }
    let workers =
        (0..2)
            .map(|_| {
                std::thread::spawn(move || {
                    ffi::reset_thread_factory_cache().unwrap();
                    let owner = EntityId::new();
                    let solid =
                        Solid::root_rounded_helical_thread_named(spec, owner, policy).unwrap();
                    let stats = crate::thread_factory_stats().unwrap();
                    assert_eq!(stats.constructions, 0);
                    assert_eq!(stats.hits, 0);
                    let topology = solid.topology_snapshot(owner, 0).unwrap();
                    assert!(
                        topology.elements.iter().all(|e| !e.origins.is_empty()
                            && e.origins.iter().all(|o| o.source == owner))
                    );
                    (
                        solid.volume_mm3().unwrap(),
                        ContentHash::of(solid.to_brep().unwrap().as_bytes()).to_string(),
                    )
                })
            })
            .collect::<Vec<_>>();
    let a = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(a[0], a[1]);
}

#[test]
fn fitted_short_and_long_registered_roots_match_fresh_geometry_and_step() {
    let policy = GeometryPolicy::default();
    let directory = tempfile::tempdir().unwrap();
    for length in [3., 12.] {
        ffi::reset_thread_factory_cache().unwrap();
        let owner = EntityId::new();
        let spec = root_spec_for_length(length);
        let restored = Solid::root_rounded_helical_thread_named(spec, owner, policy).unwrap();
        assert_eq!(crate::thread_factory_stats().unwrap().constructions, 0);
        let fresh = Solid::root_rounded_helical_thread_named_fresh(spec, owner, policy).unwrap();
        assert_eq!(crate::thread_factory_stats().unwrap().constructions, 1);
        same_topology(&restored, &fresh, owner);
        let expected = analytic_volume(spec.profile, Some(spec.root_radius_mm));
        let bounds = fresh.bounds().unwrap();
        assert!(bounds.min[2].abs() <= 16. * policy.linear_tolerance_mm);
        assert!((bounds.max[2] - length).abs() <= 16. * policy.linear_tolerance_mm);
        for (name, shape) in [("fresh", fresh), ("restored", restored)] {
            volume_matches(&shape, expected, policy);
            root_probes(&shape, spec);
            let path = directory.path().join(format!("{length}-{name}.step"));
            shape.write_step(&path).unwrap();
            let back = Solid::read_step(&path, policy).unwrap();
            assert_eq!(back.solid_count().unwrap(), 1);
            volume_matches(&back, expected, policy);
            root_probes(&back, spec);
            let other = back.bounds().unwrap();
            for (a, b) in bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip(other.min.into_iter().chain(other.max))
            {
                assert!((a - b).abs() <= 16. * policy.linear_tolerance_mm);
            }
        }
    }
}
