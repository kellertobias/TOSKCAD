use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::{GeometryPolicy, MeshOptions, RigidTransform, Solid, SphericalBevelGear};
use std::{collections::BTreeSet, f64::consts::PI};

/// The canonical 90° spherical-involute resolver with 0.05 mm backlash.
fn specification(module: f64, teeth: u32, mate: u32, width: f64, bore: f64) -> SphericalBevelGear {
    let (z, mate) = (f64::from(teeth), f64::from(mate));
    let alpha = 20_f64.to_radians();
    let delta = 1_f64.atan2(mate / z);
    let distance = module * z / (2. * delta.sin());
    let beta = (delta.sin() * alpha.cos()).asin();
    let inv = |g: f64| {
        let sigma = (g.cos() / beta.cos()).acos();
        sigma / beta.sin() - sigma.sin().atan2(beta.sin() * sigma.cos())
    };
    SphericalBevelGear {
        teeth,
        pitch_cone_angle_rad: delta,
        base_cone_angle_rad: beta,
        tip_cone_angle_rad: delta + (module / distance).atan(),
        root_cone_angle_rad: delta - (1.25 * module / distance).atan(),
        outer_cone_distance_mm: distance,
        face_width_mm: width,
        bore_radius_mm: bore,
        half_base_tooth_angle_rad: (PI * module / 2. - 0.05) / (module * z) + inv(delta),
        root_fillet_radius_mm: module * 0.2,
    }
}

fn area(solid: &Solid) -> f64 {
    solid.faces().unwrap().iter().map(|f| f.area_mm2).sum()
}

fn roles(solid: &Solid, feature: EntityId) -> BTreeSet<String> {
    solid
        .topology_snapshot(feature, 0)
        .unwrap()
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .flat_map(|e| e.origins.iter().map(|o| o.role.clone()))
        .collect()
}

#[test]
fn rust_bevels_match_native_solids_and_exchange_placed_step() {
    let transform = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
        translation_mm: [10., -20., 30.],
    };
    let policy = GeometryPolicy::default();
    for (module, teeth, mate, width, bore) in [
        (1., 24, 32, 2., 0.),
        (2., 32, 24, 6., 4.),
        (1., 24, 64, 3., 2.),
        (1.5, 64, 24, 5., 3.),
    ] {
        let spec = specification(module, teeth, mate, width, bore);
        let feature = EntityId::new();
        let rust = Solid::rust_spherical_bevel_gear_named(spec, feature, policy).unwrap();
        assert!(rust.uses_rust_geometry());
        let topology = rust.topology_snapshot(feature, 1).unwrap();
        topology.validate().unwrap();
        let mesh = rust
            .manufacturing_mesh(MeshOptions {
                linear_deflection_mm: 0.01,
                angular_deflection_rad: 0.3,
                maximum_triangles: 2_000_000,
            })
            .unwrap();
        assert!(!mesh.indices.is_empty());
        let placed = rust.transformed(transform).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bevel.step");
        placed.write_step(&path).unwrap();
        if !multicad_kernel::OCCT_LINKED {
            continue;
        }
        let native = Solid::spherical_bevel_gear_named(spec, feature, policy).unwrap();
        let volume = rust.volume_mm3().unwrap();
        let reference = native.volume_mm3().unwrap();
        assert!(
            (volume - reference).abs() / reference < 1e-5,
            "{volume} {reference}"
        );
        let [ra, na] = [area(&rust), area(&native)];
        assert!((ra - na).abs() / na < 1e-5, "{ra} {na}");
        let [rb, nb] = [rust.bounds().unwrap(), native.bounds().unwrap()];
        for i in 0..3 {
            // Native bounds add the shape's enlarged tolerance box.
            assert!((rb.min[i] - nb.min[i]).abs() < 2e-3 && (rb.max[i] - nb.max[i]).abs() < 2e-3);
        }
        assert_eq!(roles(&rust, feature), roles(&native, feature));
        let imported = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(imported.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
        let read = imported.volume().unwrap();
        assert!((read - volume).abs() / volume < 1e-5, "{read} {volume}");
        let read_area: f64 = Solid::read_step(&path, policy)
            .unwrap()
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum();
        assert!((read_area - ra).abs() / ra < 1e-5, "{read_area} {ra}");
        println!(
            "{{\"teeth\":[{teeth},{mate}],\"bore\":{bore},\"rust_volume\":{volume},\"native_volume\":{reference},\"occt_step_volume\":{read},\"rust_area\":{ra},\"native_area\":{na},\"occt_step_area\":{read_area}}}"
        );
        let [pb, qb] = [
            placed.bounds().unwrap(),
            native.transformed(transform).unwrap().bounds().unwrap(),
        ];
        for i in 0..3 {
            assert!((pb.min[i] - qb.min[i]).abs() < 2e-3 && (pb.max[i] - qb.max[i]).abs() < 2e-3);
        }
    }
}

#[test]
fn sliver_meridian_specs_build_like_native_without_meridian_faces() {
    let policy = GeometryPolicy::default();
    let base = specification(1., 24, 64, 2., 0.);
    let distance = base.outer_cone_distance_mm;
    let root = base.root_cone_angle_rad;
    let beta = base.base_cone_angle_rad;
    let length = |fillet: f64| {
        let rho = (fillet / distance).asin();
        beta - ((root + rho).cos() / rho.cos()).acos()
    };
    let fillet_for = |target: f64| {
        let (mut lo, mut hi) = (base.root_fillet_radius_mm, distance * 0.5);
        for _ in 0..200 {
            let middle = (lo + hi) / 2.;
            if length(middle) > target * policy.minimum_feature_mm / distance {
                lo = middle;
            } else {
                hi = middle;
            }
        }
        lo
    };
    let lo = fillet_for(0.5);
    let spec = SphericalBevelGear {
        root_fillet_radius_mm: lo,
        ..base
    };
    let feature = EntityId::new();
    let rust = Solid::rust_spherical_bevel_gear_named(spec, feature, policy).unwrap();
    let rust_roles = roles(&rust, feature);
    assert!(rust_roles.iter().all(|role| !role.contains("meridian")));
    if !multicad_kernel::OCCT_LINKED {
        return;
    }
    // The native builder drops the sliver face and then cannot close the
    // cap wire across the remaining gap; the Rust solid stays exact.
    match Solid::spherical_bevel_gear_named(spec, feature, policy) {
        Ok(native) => {
            assert_eq!(rust_roles, roles(&native, feature));
            let [r, n] = [&rust, &native].map(|s| s.volume_mm3().unwrap());
            assert!((r - n).abs() / n < 1e-7, "{r} {n}");
        }
        Err(error) => println!(
            "{{\"sliver_fillet_mm\":{lo},\"native\":{:?}}}",
            error.message
        ),
    }
    // A neighboring fillet keeps a meridian twenty minimum features long;
    // both kernels build it and the absorbed sliver stays beside it.
    let neighbor = SphericalBevelGear {
        root_fillet_radius_mm: fillet_for(20.),
        ..base
    };
    let near = Solid::rust_spherical_bevel_gear_named(neighbor, EntityId::new(), policy).unwrap();
    // The neighbor's fillet is about 4e-4 mm smaller than the absorbed one.
    let [r, n] = [&rust, &near].map(|s| s.volume_mm3().unwrap());
    assert!((r - n).abs() / n < 2e-5, "{r} {n}");
    let native = Solid::spherical_bevel_gear_named(neighbor, EntityId::new(), policy).unwrap();
    let reference = native.volume_mm3().unwrap();
    assert!((n - reference).abs() / reference < 1e-7, "{n} {reference}");
    println!(
        "{{\"sliver_volume_mm3\":{r},\"neighbor_rust_mm3\":{n},\"neighbor_native_mm3\":{reference}}}"
    );
}
