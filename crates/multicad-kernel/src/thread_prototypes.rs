//! Application-owned derived native geometry; never an authored project payload.
//! A mismatching or damaged registration is an ordinary construction-cache miss.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, HelicalThread, Solid, ffi};
use multicad_core::{document::EntityId, project_file::ContentHash};

mod registered {
    include!("../assets/thread-prototypes/default-m3.rs");
    pub(super) fn matches(data: &super::ffi::ThreadData, policy: super::GeometryPolicy) -> bool {
        super::input_bits(data) == INPUT_BITS
            && data.starts == STARTS
            && data.left == LEFT
            && super::policy_bits(policy) == POLICY_BITS
            && super::source_fingerprint() == SOURCE_FINGERPRINT
    }
    pub(super) fn receipt() -> super::Receipt<'static> {
        super::Receipt {
            sha: BREP_SHA,
            bounds: BOUNDS,
            volume: VOLUME,
            seeds: SEEDS,
            faces: FACE_METRICS,
        }
    }
}
const BREP: &str = include_str!("../assets/thread-prototypes/default-m3.brep");
fn input_bits(data: &ffi::ThreadData) -> [u64; 11] {
    [
        data.radius,
        data.pitch,
        data.height,
        data.depth,
        data.crest_width,
        data.rounding,
        data.lower_slope,
        data.upper_slope,
        data.taper,
        data.overlap,
        data.tolerance,
    ]
    .map(f64::to_bits)
}
fn policy_bits(policy: GeometryPolicy) -> [u64; 4] {
    [
        policy.linear_tolerance_mm,
        policy.boolean_fuzzy_mm,
        policy.minimum_feature_mm,
        policy.maximum_coordinate_mm,
    ]
    .map(f64::to_bits)
}
/// Identity of native algorithms, policy/naming code, and pinned OCCT sources.
/// Generated asset bytes/receipts are excluded to avoid a generation hash cycle.
pub fn source_fingerprint() -> String {
    let mut bytes = b"multicad.thread-prototype/native-brep-seeds/1".to_vec();
    for source in [
        crate::source_identity(),
        include_str!("threads.rs"),
        include_str!("threads.cpp"),
        include_str!("bridge.cpp"),
        include_str!("geometry.rs"),
        include_str!("provenance.rs"),
        include_str!("lib.rs"),
        include_str!("thread_prototypes.rs"),
        include_str!("../include/bridge.hpp"),
        include_str!("../include/threads.hpp"),
        include_str!("../include/measure.hpp"),
        include_str!("../build.rs"),
    ] {
        bytes.extend_from_slice(&(source.len() as u64).to_le_bytes());
        bytes.extend_from_slice(source.as_bytes());
    }
    ContentHash::of(&bytes).to_string()
}
struct Receipt<'a> {
    sha: &'a str,
    bounds: [[f64; 3]; 2],
    volume: f64,
    seeds: &'a [(u32, u32, u8)],
    faces: &'a [[f64; 4]],
}
fn failure(message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "thread prototype",
        kind: GeometryErrorKind::InvalidResult,
        message: message.into(),
    }
}
fn restore(
    brep: &str,
    receipt: &Receipt<'_>,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    if brep.is_empty() || ContentHash::of(brep.as_bytes()).to_string() != receipt.sha {
        return Err(failure("Derived native prototype digest mismatch"));
    }
    let seeds: Vec<_> = receipt
        .seeds
        .iter()
        .map(|&(face, source_index, role)| ffi::Seed {
            face,
            source_index,
            role,
        })
        .collect();
    let handle =
        ffi::shape_from_thread_prototype(brep, &seeds).map_err(|e| failure(e.to_string()))?;
    // Keep ordinary validity, solid count, policy and guarded volume adoption.
    let solid = Solid::adopt(handle, policy, "thread prototype")?;
    if solid.solid_count()? != 1 {
        return Err(failure("Prototype is not one connected solid"));
    }
    let bounds = solid.bounds()?;
    let tolerance = policy.linear_tolerance_mm * 16.;
    for (actual, expected) in [bounds.min, bounds.max].into_iter().zip(receipt.bounds) {
        for (a, b) in actual.into_iter().zip(expected) {
            if !b.is_finite() || (a - b).abs() > tolerance {
                return Err(failure("Prototype bounds differ from native receipt"));
            }
        }
    }
    let faces = solid.faces()?;
    if faces.len() != receipt.faces.len() || faces.is_empty() {
        return Err(failure("Prototype face count differs from role receipt"));
    }
    if faces.iter().any(|face| {
        !face.area_mm2.is_finite()
            || face.area_mm2 <= 0.
            || !face.center_mm.into_iter().all(f64::is_finite)
    }) {
        return Err(failure("Prototype has nonfinite or empty face geometry"));
    }
    let area: f64 = faces.iter().map(|face| face.area_mm2).sum();
    if !area.is_finite() || area <= 0. {
        return Err(failure("Prototype has no finite positive surface area"));
    }
    let volume = solid.volume_mm3()?;
    let volume_budget =
        1e-7 * (volume.abs() + receipt.volume.abs()) + 2. * policy.linear_tolerance_mm * area;
    if !receipt.volume.is_finite()
        || receipt.volume <= 0.
        || (volume - receipt.volume).abs() > volume_budget
    {
        return Err(failure(
            "Prototype volume differs from guarded native receipt",
        ));
    }
    for face in &faces {
        let expected = receipt
            .faces
            .get(face.index as usize)
            .ok_or_else(|| failure("Prototype face ordinal is outside receipt"))?;
        let area_budget = 1e-7 * (face.area_mm2.abs() + expected[3].abs())
            + tolerance * (face.area_mm2.abs().sqrt() + expected[3].abs().sqrt());
        if !expected.iter().all(|value| value.is_finite())
            || expected[3] <= 0.
            || face
                .center_mm
                .into_iter()
                .zip(expected[..3].iter())
                .any(|(a, b)| (a - b).abs() > tolerance)
            || (face.area_mm2 - expected[3]).abs() > area_budget
        {
            return Err(failure(
                "Prototype face geometry differs from neutral ancestry receipt",
            ));
        }
    }
    Ok(solid)
}
/// Only this compiled registration can supply a trusted construction shortcut.
/// All misses, damaged assets and read/receipt failures reconstruct normally.
pub(crate) fn load(
    data: &ffi::ThreadData,
    policy: GeometryPolicy,
) -> Result<Option<cxx::UniquePtr<ffi::Shape>>, GeometryError> {
    if !registered::matches(data, policy) {
        return Ok(None);
    }
    Ok(restore(BREP, &registered::receipt(), policy)
        .ok()
        .map(|solid| solid.native.0))
}
/// Developer output from a forced fresh native construction, not a project format.
pub struct GeneratedPrototype {
    pub brep: String,
    pub registration: String,
    pub fresh_seconds: f64,
    pub load_seconds: f64,
    pub volume_mm3: f64,
    pub face_count: usize,
}
/// Generate and independently roundtrip one canonical caller-supplied recipe.
/// The fresh constructor bypasses both the factory cache and compiled assets.
pub fn generate(
    spec: HelicalThread,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<GeneratedPrototype, GeometryError> {
    let data = spec.checked(policy)?;
    let started = std::time::Instant::now();
    let fresh = Solid::helical_thread_named_fresh(spec, feature, policy)?;
    let fresh_seconds = started.elapsed().as_secs_f64();
    let brep = fresh.to_brep()?;
    let sha = ContentHash::of(brep.as_bytes()).to_string();
    let bounds = fresh.bounds()?;
    let volume = fresh.volume_mm3()?;
    let native_faces = fresh.faces()?;
    let mut faces = vec![[0.; 4]; native_faces.len()];
    for face in &native_faces {
        let row = faces
            .get_mut(face.index as usize)
            .ok_or_else(|| failure("Noncontiguous face index"))?;
        *row = [
            face.center_mm[0],
            face.center_mm[1],
            face.center_mm[2],
            face.area_mm2,
        ];
    }
    let seeds: Vec<_> = ffi::seeds(&fresh.native.0)
        .map_err(|e| failure(e.to_string()))?
        .into_iter()
        .map(|s| (s.face, s.source_index, s.role))
        .collect();
    let receipt = Receipt {
        sha: &sha,
        bounds: [bounds.min, bounds.max],
        volume,
        seeds: &seeds,
        faces: &faces,
    };
    let started = std::time::Instant::now();
    let mut loaded = restore(&brep, &receipt, policy)?;
    loaded.name_thread(feature, spec.starts)?;
    // Compare complete current native topology and every owner-bound face role.
    let fresh_topology = fresh.topology_snapshot(feature, 0)?;
    let loaded_topology = loaded.topology_snapshot(feature, 0)?;
    if fresh_topology.elements.len() != loaded_topology.elements.len()
        || fresh_topology
            .elements
            .iter()
            .zip(&loaded_topology.elements)
            .any(|(a, b)| a.key != b.key || a.origins != b.origins || a.boundary != b.boundary)
    {
        return Err(failure(
            "Prototype roundtrip changes native topology or persistent face intents",
        ));
    }
    let load_seconds = started.elapsed().as_secs_f64();
    let registration = format!(
        "// Generated by generate_thread_prototypes; derived native cache only.\nconst INPUT_BITS: [u64;11] = {:?};\nconst STARTS: u32 = {};\nconst LEFT: bool = {};\nconst POLICY_BITS: [u64;4] = {:?};\nconst SOURCE_FINGERPRINT: &str = {:?};\nconst BREP_SHA: &str = {:?};\nconst BOUNDS: [[f64;3];2] = {:?};\nconst VOLUME: f64 = {:?};\nconst SEEDS: &[(u32,u32,u8)] = &{:?};\nconst FACE_METRICS: &[[f64;4]] = &{:?};\n",
        input_bits(&data),
        data.starts,
        data.left,
        policy_bits(policy),
        source_fingerprint(),
        sha,
        [bounds.min, bounds.max],
        volume,
        seeds,
        faces
    );
    Ok(GeneratedPrototype {
        brep,
        registration,
        fresh_seconds,
        load_seconds,
        volume_mm3: volume,
        face_count: faces.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn default_spec() -> HelicalThread {
        let multicad_core::model::MechanicalPartParameters::MachineScrew { thread, .. } =
            multicad_core::model::default_machine_screw()
        else {
            unreachable!()
        };
        let document = multicad_core::document::Document::new("Canonical prototype test").unwrap();
        let values = multicad_core::parameters::evaluate(&document).unwrap();
        let resolved = thread
            .resolve(&values, multicad_core::model::ThreadMode::NewExternal)
            .unwrap();
        HelicalThread {
            major_radius_mm: resolved.major_radius_mm,
            pitch_mm: resolved.pitch_mm,
            length_mm: resolved.length_mm,
            depth_mm: resolved.profile.depth_mm,
            crest_width_mm: resolved.profile.crest_width_mm,
            lower_flank_deg: resolved.profile.lower_flank_deg,
            upper_flank_deg: resolved.profile.upper_flank_deg,
            radial_taper: resolved.profile.radial_taper,
            starts: resolved.starts,
            left_hand: false,
        }
    }
    #[test]
    fn compiled_default_m3_matches_only_exact_native_input_policy_and_current_sources() {
        let policy = GeometryPolicy::default();
        let data = default_spec().checked(policy).unwrap();
        assert!(
            !BREP.is_empty(),
            "Generate the reviewed default M3 native asset first"
        );
        assert!(
            registered::matches(&data, policy),
            "Regenerate stale native asset after kernel changes"
        );
        assert!(load(&data, policy).unwrap().is_some());
        let mut changed = data;
        changed.height = f64::from_bits(changed.height.to_bits() + 1);
        assert!(load(&changed, policy).unwrap().is_none());
        let mut changed_policy = policy;
        changed_policy.maximum_coordinate_mm =
            f64::from_bits(changed_policy.maximum_coordinate_mm.to_bits() + 1);
        assert!(load(&data, changed_policy).unwrap().is_none());
        let mut changed = data;
        changed.left = !changed.left;
        assert!(load(&changed, policy).unwrap().is_none());
    }
    #[test]
    fn corrupted_geometry_or_role_receipts_refuse_without_trusting_project_payloads() {
        let policy = GeometryPolicy::default();
        assert!(
            !BREP.is_empty(),
            "Generate the reviewed default M3 native asset first"
        );
        let receipt = registered::receipt();
        let corrupted = format!("{BREP}\ncorruption");
        assert!(restore(&corrupted, &receipt, policy).is_err());
        let wrong_seeds = [(u32::MAX, 0, 0)];
        let wrong_receipt = Receipt {
            seeds: &wrong_seeds,
            ..receipt
        };
        assert!(restore(BREP, &wrong_receipt, policy).is_err());
        let receipt = registered::receipt();
        for seeds in [
            Vec::new(),
            receipt.seeds[..receipt.seeds.len() - 1].to_vec(),
            {
                let mut duplicate = receipt.seeds.to_vec();
                duplicate.push(duplicate[0]);
                duplicate
            },
            {
                let mut invalid = receipt.seeds.to_vec();
                invalid[0].2 = u8::MAX;
                invalid
            },
        ] {
            let wrong_receipt = Receipt {
                seeds: &seeds,
                ..registered::receipt()
            };
            assert!(restore(BREP, &wrong_receipt, policy).is_err());
        }
        let receipt = registered::receipt();
        let wrong_receipt = Receipt {
            volume: receipt.volume * 2.,
            ..receipt
        };
        assert!(restore(BREP, &wrong_receipt, policy).is_err());
    }
    #[test]
    fn loaded_prototype_and_fresh_construction_preserve_native_face_intents_for_new_owners() {
        let spec = default_spec();
        let policy = GeometryPolicy::default();
        let owner = EntityId::new();
        ffi::reset_thread_factory_cache().unwrap();
        let retained = Solid::helical_thread_named(spec, EntityId::new(), policy).unwrap();
        let source_brep = retained.to_brep().unwrap();
        let source_volume = retained.volume_mm3().unwrap();
        let source_bounds = retained.bounds().unwrap();
        let before = crate::thread_factory_stats().unwrap();
        let fresh = Solid::helical_thread_named_fresh(spec, owner, policy).unwrap();
        let after = crate::thread_factory_stats().unwrap();
        assert_eq!(after.constructions, before.constructions + 1);
        assert_eq!(after.hits, before.hits);
        assert_eq!(after.entries, before.entries);
        assert_eq!(retained.to_brep().unwrap(), source_brep);
        assert_eq!(retained.volume_mm3().unwrap(), source_volume);
        assert_eq!(retained.bounds().unwrap(), source_bounds);
        let handle = load(&spec.checked(policy).unwrap(), policy)
            .unwrap()
            .expect("Current compiled prototype");
        let mut loaded = Solid::adopt(handle, policy, "prototype comparison").unwrap();
        loaded.name_thread(owner, spec.starts).unwrap();
        let fresh_topology = fresh.topology_snapshot(owner, 0).unwrap();
        let loaded_topology = loaded.topology_snapshot(owner, 0).unwrap();
        assert_eq!(
            fresh_topology.elements.len(),
            loaded_topology.elements.len()
        );
        for (a, b) in fresh_topology
            .elements
            .iter()
            .zip(&loaded_topology.elements)
        {
            assert_eq!(a.key, b.key);
            assert_eq!(a.origins, b.origins);
            assert_eq!(a.boundary, b.boundary);
        }
        let directory = tempfile::tempdir().unwrap();
        let mut steps = Vec::new();
        for (name, solid) in [("fresh", &fresh), ("loaded", &loaded)] {
            let path = directory.path().join(format!("{name}.step"));
            solid.write_step(&path).unwrap();
            let read = Solid::read_step(&path, policy).unwrap();
            let area: f64 = solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum();
            let expected = solid.volume_mm3().unwrap();
            let actual = read.volume_mm3().unwrap();
            let budget =
                1e-7 * (expected.abs() + actual.abs()) + 2. * policy.linear_tolerance_mm * area;
            assert!((actual - expected).abs() <= budget);
            let bounds = read.bounds().unwrap();
            for (actual, expected) in [bounds.min, bounds.max]
                .into_iter()
                .zip([source_bounds.min, source_bounds.max])
            {
                for (a, b) in actual.into_iter().zip(expected) {
                    assert!((a - b).abs() <= 16. * policy.linear_tolerance_mm);
                }
            }
            steps.push(actual);
        }
        assert!((steps[0] - steps[1]).abs() <= 2e-7 * (steps[0].abs() + steps[1].abs()));
        assert_eq!(retained.to_brep().unwrap(), source_brep);
        let other = EntityId::new();
        let handle = load(&spec.checked(policy).unwrap(), policy)
            .unwrap()
            .unwrap();
        let mut other_solid = Solid::adopt(handle, policy, "second owner").unwrap();
        other_solid.name_thread(other, spec.starts).unwrap();
        assert!(
            other_solid
                .topology_snapshot(other, 0)
                .unwrap()
                .elements
                .iter()
                .flat_map(|element| element.origins.iter())
                .all(|origin| origin.source == other)
        );
    }
}
