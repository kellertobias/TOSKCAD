use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::{
    GeneratedZaWormWheel, GeometryPolicy, MeshOptions, RigidTransform, Solid, ZaWorm,
};
use std::collections::{BTreeMap, BTreeSet};

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

/// Total area and area-weighted center per role.
fn by_role(solid: &Solid, feature: EntityId) -> BTreeMap<String, (f64, [f64; 3])> {
    let faces = solid.faces().unwrap();
    let topology = solid.topology_snapshot(feature, 0).unwrap();
    let mut out: BTreeMap<String, (f64, [f64; 3])> = BTreeMap::new();
    for element in topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        let face = &faces[element.key.ordinal as usize];
        for origin in &element.origins {
            let entry = out.entry(origin.role.clone()).or_insert((0., [0.; 3]));
            entry.0 += face.area_mm2;
            for i in 0..3 {
                entry.1[i] += face.center_mm[i] * face.area_mm2;
            }
        }
    }
    for (area, center) in out.values_mut() {
        *center = center.map(|c| c / *area);
    }
    out
}

fn compare(rust: &Solid, native: &Solid, feature: EntityId, volume_tolerance: f64, label: &str) {
    let volume = rust.volume_mm3().unwrap();
    let reference = native.volume_mm3().unwrap();
    assert!(
        (volume - reference).abs() / reference < volume_tolerance,
        "{label} volume {volume} {reference}"
    );
    let [rb, nb] = [rust.bounds().unwrap(), native.bounds().unwrap()];
    for i in 0..3 {
        assert!(
            (rb.min[i] - nb.min[i]).abs() < 5e-3 && (rb.max[i] - nb.max[i]).abs() < 5e-3,
            "{label} bounds {rb:?} {nb:?}"
        );
    }
    assert_eq!(roles(rust, feature), roles(native, feature), "{label}");
    let [ra, na] = [by_role(rust, feature), by_role(native, feature)];
    for (role, (area, center)) in &ra {
        let (native_area, native_center) = na[role];
        assert!(
            (area - native_area).abs() / native_area < 2e-3,
            "{label} {role} area {area} {native_area}"
        );
        let scale = native_area.sqrt();
        for i in 0..3 {
            assert!(
                (center[i] - native_center[i]).abs() < 0.02 * scale,
                "{label} {role} center {center:?} {native_center:?}"
            );
        }
    }
    let total = |m: &BTreeMap<String, (f64, [f64; 3])>| -> f64 { m.values().map(|(a, _)| a).sum() };
    println!(
        "{{\"case\":\"{label}\",\"rust_volume\":{volume},\"native_volume\":{reference},\"rust_area\":{},\"native_area\":{}}}",
        total(&ra),
        total(&na)
    );
}

#[test]
fn rust_worms_and_wheels_match_native_solids() {
    let policy = GeometryPolicy::default();
    let options = MeshOptions {
        linear_deflection_mm: 0.01,
        angular_deflection_rad: 0.3,
        maximum_triangles: 3_000_000,
    };
    let transform = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
        translation_mm: [10., -20., 30.],
    };
    for (m, length, bore, thinning) in
        [(1., 20., 0., 0.02), (2., 16., 3., 0.1), (0.5, 32., 0.8, 0.)]
    {
        let spec = ZaWorm {
            axial_module_mm: m,
            threaded_length_mm: length * m,
            bore_radius_mm: bore,
            axial_thinning_mm: thinning,
        };
        let feature = EntityId::new();
        let rust = Solid::rust_za_worm_named(spec, feature, policy).unwrap();
        assert!(rust.uses_rust_geometry());
        rust.topology_snapshot(feature, 1)
            .unwrap()
            .validate()
            .unwrap();
        assert!(!rust.manufacturing_mesh(options).unwrap().indices.is_empty());
        assert!(rust.display_edges(0.01).unwrap().len() >= 12);
        let placed = rust.transformed(transform).unwrap();
        assert_eq!(placed.volume_mm3().unwrap(), rust.volume_mm3().unwrap());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worm.step");
        placed.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let decoded = placed.verify_worm_step(&text).unwrap().unwrap();
        let volume = rust.volume_mm3().unwrap();
        assert!(
            (decoded - volume).abs() / volume < 1e-8,
            "{decoded} {volume}"
        );
        if !multicad_kernel::OCCT_LINKED {
            continue;
        }
        let native = Solid::za_worm_named(spec, feature, policy).unwrap();
        compare(
            &rust,
            &native,
            feature,
            1e-6,
            &format!("worm m{m} L{length} b{bore}"),
        );
        let imported = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(imported.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
        let read = imported.volume().unwrap();
        assert!((read - volume).abs() / volume < 1e-6, "{read} {volume}");
        println!(
            "{{\"case\":\"worm m{m} b{bore} STEP\",\"rust_decoded_volume\":{decoded},\"occt_step_volume\":{read}}}"
        );
        let [pb, qb] = [
            placed.bounds().unwrap(),
            native.transformed(transform).unwrap().bounds().unwrap(),
        ];
        for i in 0..3 {
            assert!((pb.min[i] - qb.min[i]).abs() < 5e-3 && (pb.max[i] - qb.max[i]).abs() < 5e-3);
        }
    }
    for (m, teeth, width, bore, thinning) in [
        (1., 32, 1., 0., 0.),
        (2., 40, 2., 6., 0.1),
        (0.5, 64, 0.5, 2., 0.02),
    ] {
        let spec = GeneratedZaWormWheel {
            teeth,
            axial_module_mm: m,
            face_width_mm: width * m,
            bore_radius_mm: bore,
            pitch_thinning_mm: thinning,
        };
        let feature = EntityId::new();
        let rust = Solid::rust_generated_za_worm_wheel_named(spec, feature, policy).unwrap();
        rust.topology_snapshot(feature, 1)
            .unwrap()
            .validate()
            .unwrap();
        assert!(!rust.manufacturing_mesh(options).unwrap().indices.is_empty());
        assert!(rust.display_edges(0.01).unwrap().len() >= 23 * teeth as usize);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wheel.step");
        let placed = rust.transformed(transform).unwrap();
        let started = std::time::Instant::now();
        placed.write_step(&path).unwrap();
        let written = started.elapsed();
        let text = std::fs::read_to_string(&path).unwrap();
        let decoded = placed.verify_worm_step(&text).unwrap().unwrap();
        let volume = rust.volume_mm3().unwrap();
        assert!(
            (decoded - volume).abs() / volume < 1e-8,
            "{decoded} {volume}"
        );
        // The proof belongs to this placement and wheel.
        assert!(rust.verify_worm_step(&text).unwrap().is_err());
        assert!(rust.verify_worm_step("").unwrap().is_err());
        if !multicad_kernel::OCCT_LINKED {
            continue;
        }
        let imported = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(imported.valid().unwrap());
        assert_eq!(Solid::step_solids(&path).unwrap().len(), 1);
        let read = imported.volume().unwrap();
        assert!((read - volume).abs() / volume < 1e-6, "{read} {volume}");
        println!(
            "{{\"case\":\"wheel m{m} z{teeth} b{bore} STEP\",\"bytes\":{},\"write_ms\":{},\"rust_decoded_volume\":{decoded},\"occt_step_volume\":{read},\"rust_volume\":{volume}}}",
            text.len(),
            written.as_secs_f64() * 1e3
        );
        let native = Solid::generated_za_worm_wheel_named(spec, feature, policy).unwrap();
        // The native wheel interpolates each generating patch within its
        // certified polynomial bound; the Rust wheel is exact.
        compare(
            &rust,
            &native,
            feature,
            1e-4,
            &format!("wheel m{m} z{teeth} b{bore}"),
        );
    }
}
