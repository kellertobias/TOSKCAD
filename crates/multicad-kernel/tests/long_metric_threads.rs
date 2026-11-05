use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, GeometryPolicy, HelicalThread, RigidTransform, Solid,
};
use std::f64::consts::{PI, TAU};

fn m8(length_mm: f64) -> HelicalThread {
    HelicalThread {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm,
        depth_mm: 5. * 3_f64.sqrt() / 16. * 1.25,
        crest_width_mm: 1.25 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    }
}

fn analytic_volume(spec: HelicalThread) -> f64 {
    // Integrate the axial crest/flank width against circular annuli. Every
    // transverse section is a rotation of the same section, including at a
    // fractional final turn, so its area times the exact authored length is V.
    let major = spec.major_radius_mm;
    let root = major - spec.depth_mm;
    let flank_sum = 2. * 30_f64.to_radians().tan();
    let ridge = spec.crest_width_mm * (major * major - root * root) / 2.
        + flank_sum
            * (major * (major * major - root * root) / 2. - (major.powi(3) - root.powi(3)) / 3.);
    spec.length_mm * (PI * root * root + TAU / spec.pitch_mm * ridge)
}

fn volume_and_uncertainty(solid: &Solid) -> (f64, f64) {
    let volume = solid.volume_mm3().unwrap();
    let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    // Established native metrology budget: actual policy's linear tolerance
    // times measured area, plus the unchanged adaptive GK relative-error gate.
    // Independent profile/material probes below also constrain the geometry.
    (
        volume,
        GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * volume.abs(),
    )
}

fn material_fraction(solid: &Solid, center: [f64; 3]) -> f64 {
    let edge = 0.002;
    let probe = Solid::box_solid([edge; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|x| x - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&probe, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |intersection| intersection.volume_mm3().unwrap())
        / edge.powi(3)
}

fn assert_material(solid: &Solid, point: [f64; 3], inside: bool) {
    let fraction = material_fraction(solid, point);
    assert!(
        if inside {
            fraction > 0.999
        } else {
            fraction < 1e-5
        },
        "point {point:?}, expected material={inside}, measured fraction={fraction}"
    );
}

fn assert_extent_and_profile(solid: &Solid, spec: HelicalThread) {
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    for axis in 0..2 {
        assert!((bounds.min[axis] + spec.major_radius_mm).abs() < 1e-6);
        assert!((bounds.max[axis] - spec.major_radius_mm).abs() < 1e-6);
    }
    assert!(bounds.min[2].abs() < 1e-6);
    assert!((bounds.max[2] - spec.length_mm).abs() < 1e-6);

    let crest_probe_radius = spec.major_radius_mm - spec.depth_mm * 0.05;
    // Basic 60-degree profile: at this pitch radius, tooth axial width is P/2.
    let pitch_radius = spec.major_radius_mm - 3. * 3_f64.sqrt() / 16. * spec.pitch_mm;
    for z in [
        0.75 * spec.pitch_mm,
        0.47 * spec.length_mm,
        spec.length_mm - 0.75 * spec.pitch_mm,
    ] {
        let phase = TAU * z / spec.pitch_mm;
        let (sin, cos) = phase.sin_cos();
        assert_material(
            solid,
            [crest_probe_radius * cos, crest_probe_radius * sin, z],
            true,
        );
        assert_material(
            solid,
            [-crest_probe_radius * cos, -crest_probe_radius * sin, z],
            false,
        );
        for (offset, inside) in [(0.20, true), (0.30, false)] {
            assert_material(
                solid,
                [
                    pitch_radius * cos,
                    pitch_radius * sin,
                    z + offset * spec.pitch_mm,
                ],
                inside,
            );
        }
    }
}

#[test]
fn fresh_m8_long_and_fractional_threads_preserve_profile_volume_and_step() {
    for length in [12.5, 50., 49.8] {
        let spec = m8(length);
        let feature = EntityId::new();
        // Bypass both trusted assets and completed-thread caches. This catches
        // the original long continuous PipeShell failure on each cold build.
        let solid = Solid::helical_thread_named_fresh(spec, feature, Default::default())
            .unwrap_or_else(|error| panic!("fresh M8x1.25 L{length}: {error}"));
        assert_extent_and_profile(&solid, spec);
        let expected = analytic_volume(spec);
        let (native_volume, native_budget) = volume_and_uncertainty(&solid);
        assert!(
            (native_volume - expected).abs() <= native_budget,
            "L{length}: native={native_volume}, analytic={expected}, budget={native_budget}"
        );
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        let faces = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .collect::<Vec<_>>();
        assert!(faces.iter().all(|face| !face.origins.is_empty()
            && face.origins.iter().all(|origin| origin.source == feature)));
        for role in [
            "cap.start",
            "cap.end",
            "thread.root",
            "thread/start/0/lower",
            "thread/start/0/crest",
            "thread/start/0/upper",
        ] {
            assert!(
                faces
                    .iter()
                    .any(|face| face.origins.iter().any(|origin| origin.role == role)),
                "L{length}: missing native face role {role}"
            );
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("long-metric.step");
        solid.write_step(&path).unwrap();
        // STEP import adopts and natively validates a fresh B-rep, independently
        // of the original wrapper's completed measurements and face metadata.
        let reopened = Solid::read_step(&path, Default::default()).unwrap();
        assert_extent_and_profile(&reopened, spec);
        let (readback_volume, readback_budget) = volume_and_uncertainty(&reopened);
        assert!(
            (readback_volume - expected).abs() <= readback_budget,
            "L{length}: STEP={readback_volume}, analytic={expected}, budget={readback_budget}"
        );
        assert!(
            (readback_volume - native_volume).abs() <= native_budget + readback_budget,
            "L{length}: native={native_volume}, STEP={readback_volume}, combined budget={}",
            native_budget + readback_budget
        );
    }
}

#[test]
fn long_thread_recovery_preserves_input_length_and_extended_turn_limits() {
    for length in [0.5, 160.] {
        let error =
            Solid::helical_thread_named_fresh(m8(length), EntityId::new(), Default::default())
                .err()
                .expect("invalid input must not be rescued by a native representation retry");
        assert_eq!(error.kind, GeometryErrorKind::Limit);
        assert!(error.message.contains("128 total extended turns"));
    }
}
