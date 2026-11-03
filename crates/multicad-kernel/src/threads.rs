//! Bounded native helices with flat or circular crest/root axial profiles.
//! These are geometric inputs; standard families and fit tables belong to core.
pub use crate::ffi::ThreadFactoryStats;
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::document::EntityId;

/// Diagnostic work counts for the current native owner thread. No native
/// handles or cache mutation cross the thread boundary through these values.
pub fn thread_factory_stats() -> Result<ThreadFactoryStats, GeometryError> {
    ffi::thread_factory_stats().map_err(|error| GeometryError {
        operation: "thread factory metrics",
        kind: GeometryErrorKind::Native,
        message: error.to_string(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HelicalThread {
    /// Crest radius at the base plane z=0.
    pub major_radius_mm: f64,
    /// Axial spacing between adjacent threads, independent of start count.
    pub pitch_mm: f64,
    pub length_mm: f64,
    pub depth_mm: f64,
    pub crest_width_mm: f64,
    /// Flank angles from the radial direction in the axial section.
    pub lower_flank_deg: f64,
    pub upper_flank_deg: f64,
    /// Change of radius per unit axial length, rather than diameter taper.
    pub radial_taper: f64,
    pub starts: u32,
    pub left_hand: bool,
}
/// Symmetric circular crest/root profile in an axial meridian. The first
/// bounded implementation accepts the exact single-start, parallel 55° form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedHelicalThread {
    pub major_radius_mm: f64,
    pub pitch_mm: f64,
    pub length_mm: f64,
    pub depth_mm: f64,
    pub rounding_radius_mm: f64,
    /// Half of the included angle, measured from the radial direction.
    pub flank_angle_deg: f64,
    pub starts: u32,
    pub left_hand: bool,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "thread",
        kind,
        message: message.into(),
    }
}
impl HelicalThread {
    pub fn lead_mm(self) -> f64 {
        self.pitch_mm * f64::from(self.starts)
    }
    fn checked(self, policy: GeometryPolicy) -> Result<ffi::ThreadData, GeometryError> {
        policy.validate()?;
        for length in [
            self.major_radius_mm,
            self.pitch_mm,
            self.length_mm,
            self.depth_mm,
            self.crest_width_mm,
        ] {
            policy.length(length)?;
        }
        if !(1..=8).contains(&self.starts)
            || ![self.lower_flank_deg, self.upper_flank_deg]
                .into_iter()
                .all(|angle| angle.is_finite() && (1. ..=80.).contains(&angle))
            || !self.radial_taper.is_finite()
            || self.radial_taper.abs() > 0.1
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Thread needs 1–8 starts, finite 1–80 degree flanks and radius taper within ±0.1",
            ));
        }
        if self.pitch_mm < 0.001
            || self.length_mm < self.pitch_mm
            || self.length_mm / self.pitch_mm + 2. * f64::from(self.starts) > 128.
        {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Thread needs pitch ≥0.001 mm, at least one pitch of length, and at most 128 total extended turns",
            ));
        }
        let lower_slope = self.lower_flank_deg.to_radians().tan();
        let upper_slope = self.upper_flank_deg.to_radians().tan();
        let overlap = (self.pitch_mm * 0.02).max(policy.linear_tolerance_mm * 64.);
        let root = self.major_radius_mm - self.depth_mm;
        let width = self.crest_width_mm + (self.depth_mm + overlap) * (lower_slope + upper_slope);
        if self.crest_width_mm < policy.linear_tolerance_mm * 100.
            || root - overlap < policy.minimum_feature_mm
            || width >= self.pitch_mm - policy.minimum_feature_mm
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Thread profile needs a positive root, resolvable crest and separated adjacent starts",
            ));
        }
        for z in [-2. * self.lead_mm(), self.length_mm + 2. * self.lead_mm()] {
            let radius = root + self.radial_taper * z - overlap;
            policy.length(radius)?;
            policy.point([self.major_radius_mm + self.radial_taper * z, 0., z])?;
        }
        Ok(ffi::ThreadData {
            radius: self.major_radius_mm,
            pitch: self.pitch_mm,
            height: self.length_mm,
            depth: self.depth_mm,
            crest_width: self.crest_width_mm,
            rounding: 0.,
            lower_slope,
            upper_slope,
            taper: self.radial_taper,
            overlap,
            tolerance: policy.linear_tolerance_mm,
            starts: self.starts,
            left: self.left_hand,
        })
    }
}
impl RoundedHelicalThread {
    fn checked(self, policy: GeometryPolicy) -> Result<ffi::ThreadData, GeometryError> {
        let angle = 27.5_f64.to_radians();
        let depth = self.pitch_mm / (3. * angle.tan());
        let rounding = self.pitch_mm * angle.cos() / (12. * (1. - angle.sin()));
        let matches = |actual: f64, expected: f64| {
            actual.is_finite() && (actual - expected).abs() <= expected.abs() * 1e-12
        };
        if self.starts != 1
            || !self.flank_angle_deg.is_finite()
            || (self.flank_angle_deg - 27.5).abs() > 1e-10
            || !matches(self.depth_mm, depth)
            || !matches(self.rounding_radius_mm, rounding)
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Rounded thread needs the exact symmetric 55-degree single-start parallel profile",
            ));
        }
        policy.length(self.rounding_radius_mm)?;
        let mut data = HelicalThread {
            major_radius_mm: self.major_radius_mm,
            pitch_mm: self.pitch_mm,
            length_mm: self.length_mm,
            depth_mm: self.depth_mm,
            crest_width_mm: 2. * self.rounding_radius_mm * angle.cos(),
            lower_flank_deg: self.flank_angle_deg,
            upper_flank_deg: self.flank_angle_deg,
            radial_taper: 0.,
            starts: self.starts,
            left_hand: self.left_hand,
        }
        .checked(policy)?;
        data.rounding = self.rounding_radius_mm;
        Ok(data)
    }
}
impl Solid {
    pub fn rounded_helical_thread_named(
        spec: RoundedHelicalThread,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Thread feature identity must not be nil",
            ));
        }
        let native = ffi::helical_thread(&spec.checked(policy)?)
            .map_err(|error| failure(GeometryErrorKind::Native, error.to_string()))?;
        let mut solid = Self::adopt(native, policy, "rounded thread")?;
        if solid.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Thread must remain one connected solid",
            ));
        }
        solid.name_thread(feature, spec.starts)?;
        // The optional factory only remembers geometry after every ordinary
        // adoption, policy and owner-specific naming guard succeeded. Cache
        // allocation/serialization failure must not invalidate a valid solid.
        let _ = ffi::remember_validated_thread(&solid.native.0);
        Ok(solid)
    }
    /// Build a real helical B-rep around +Z with flat cut ends. Native face
    /// ancestry names each start's flanks/crest, the root and both end caps.
    pub fn helical_thread_named(
        spec: HelicalThread,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Thread feature identity must not be nil",
            ));
        }
        let data = spec.checked(policy)?;
        let native = ffi::helical_thread(&data)
            .map_err(|error| failure(GeometryErrorKind::Native, error.to_string()))?;
        let mut solid = Self::adopt(native, policy, "thread")?;
        if solid.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Thread must remain one connected solid",
            ));
        }
        solid.name_thread(feature, spec.starts)?;
        let _ = ffi::remember_validated_thread(&solid.native.0);
        Ok(solid)
    }
}

#[cfg(test)]
mod factory_tests {
    use super::*;
    use crate::{BooleanOperation, MeshOptions, RigidTransform};
    use multicad_core::references::TopologyKind;

    fn reset() {
        ffi::reset_thread_factory_cache().unwrap();
    }
    fn small() -> HelicalThread {
        HelicalThread {
            major_radius_mm: 2.,
            pitch_mm: 1.,
            length_mm: 1.,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 1. / 8.,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            starts: 1,
            left_hand: false,
        }
    }
    fn build(spec: HelicalThread, owner: EntityId) -> Solid {
        Solid::helical_thread_named(spec, owner, Default::default()).unwrap()
    }
    fn owns(solid: &Solid, owner: EntityId, forbidden: EntityId) {
        let snapshot = solid.topology_snapshot(owner, 0).unwrap();
        let faces: Vec<_> = snapshot
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .collect();
        assert!(!faces.is_empty());
        assert!(
            faces
                .iter()
                .all(|face| face.origins.iter().any(|o| o.source == owner))
        );
        assert!(
            snapshot
                .elements
                .iter()
                .all(|e| e.origins.iter().all(|o| o.source != forbidden))
        );
    }
    fn material(solid: &Solid, point: [f64; 3]) -> f64 {
        let size = 0.001;
        let tool = Solid::box_solid([size; 3], Default::default())
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: point.map(|p| p - size / 2.),
                ..Default::default()
            })
            .unwrap();
        solid
            .boolean(&tool, BooleanOperation::Intersect, Default::default())
            .unwrap()
            .map_or(0., |s| s.volume_mm3().unwrap())
            / size.powi(3)
    }

    #[test]
    fn default_metric_factory_reuses_geometry_and_rebinds_each_new_owner() {
        reset();
        let spec = HelicalThread {
            major_radius_mm: 1.5,
            pitch_mm: 0.5,
            length_mm: 10.,
            depth_mm: 5. * 3_f64.sqrt() / 32.,
            crest_width_mm: 0.5 / 8.,
            ..small()
        };
        let first_id = EntityId::new();
        let second_id = EntityId::new();
        let first = build(spec, first_id);
        let before = thread_factory_stats().unwrap();
        assert_eq!(
            (
                before.requests,
                before.constructions,
                before.hits,
                before.entries
            ),
            (1, 1, 0, 1)
        );
        assert!(before.serialized_bytes > 0 && before.serialized_bytes <= 32 * 1024 * 1024);
        let started = std::time::Instant::now();
        let second = build(spec, second_id);
        let warm = started.elapsed();
        let after = thread_factory_stats().unwrap();
        assert_eq!((after.requests, after.constructions, after.hits), (2, 1, 1));
        assert_eq!(after.continuous_attempts, before.continuous_attempts);
        assert_eq!(after.bounded_attempts, before.bounded_attempts);
        owns(&first, first_id, second_id);
        owns(&second, second_id, first_id);
        let initial_bounds = first.bounds().unwrap();
        let initial_volume = first.volume_mm3().unwrap();
        assert_eq!(second.bounds().unwrap(), initial_bounds);
        assert_eq!(second.volume_mm3().unwrap(), initial_volume);
        assert!(
            !second
                .mesh(MeshOptions::default())
                .unwrap()
                .indices
                .is_empty()
        );
        let tool = Solid::box_solid([0.6, 0.6, 0.5], Default::default())
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: [1., -0.3, 4.5],
                ..Default::default()
            })
            .unwrap();
        let modified = second
            .boolean(&tool, BooleanOperation::Subtract, Default::default())
            .unwrap()
            .unwrap();
        assert!(modified.volume_mm3().unwrap() < initial_volume - 0.001);
        assert!(!ffi::remember_validated_thread(&modified.native.0).unwrap());
        let moved = second
            .transformed(RigidTransform {
                translation_mm: [3., 4., 5.],
                ..Default::default()
            })
            .unwrap();
        assert!(!ffi::remember_validated_thread(&moved.native.0).unwrap());
        let third_id = EntityId::new();
        let third = build(spec, third_id);
        assert_eq!(thread_factory_stats().unwrap().constructions, 1);
        owns(&third, third_id, second_id);
        assert_eq!(first.bounds().unwrap(), initial_bounds);
        assert_eq!(third.volume_mm3().unwrap(), initial_volume);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("factory-thread.step");
        third.write_step(&path).unwrap();
        let step = Solid::read_step(&path, Default::default()).unwrap();
        let fresh = Solid::from_brep(&first.to_brep().unwrap(), Default::default()).unwrap();
        for shape in [&first, &second, &third, &fresh, &step] {
            assert_eq!(shape.solid_count().unwrap(), 1);
            let area: f64 = shape.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            let volume = shape.volume_mm3().unwrap();
            assert!(
                (volume - initial_volume).abs()
                    <= GeometryPolicy::default().linear_tolerance_mm * area
                        + 1e-7 * (volume + initial_volume)
            );
            assert!(material(shape, [1.49, 0., 1.]) > 0.999);
            assert!(material(shape, [1.49, 0., 1.25]) < 0.001);
            assert!(material(shape, [1.51, 0., 1.]) < 0.001);
        }
        if std::env::var_os("MULTICAD_NATIVE_THREAD_PROFILE").is_some() {
            eprintln!(
                "default metric new-owner warm factory {warm:?}; constructions={}, hits={}",
                after.constructions, after.hits
            );
        }
    }

    #[test]
    fn every_native_input_bit_participates_in_factory_lookup() {
        reset();
        let spec = small();
        let _solid = build(spec, EntityId::new());
        let original = spec.checked(Default::default()).unwrap();
        assert!(ffi::thread_factory_contains(&original).unwrap());
        macro_rules! changed {
            ($field:ident) => {{
                let mut candidate = original;
                candidate.$field = f64::from_bits(candidate.$field.to_bits() + 1);
                assert!(
                    !ffi::thread_factory_contains(&candidate).unwrap(),
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
        let mut candidate = original;
        candidate.starts += 1;
        assert!(!ffi::thread_factory_contains(&candidate).unwrap());
        candidate = original;
        candidate.left = !candidate.left;
        assert!(!ffi::thread_factory_contains(&candidate).unwrap());
        let changed = build(
            HelicalThread {
                major_radius_mm: 2.01,
                ..spec
            },
            EntityId::new(),
        );
        assert_eq!(thread_factory_stats().unwrap().constructions, 2);
        assert!((changed.bounds().unwrap().max[0] - 2.01).abs() < 1e-6);
    }

    #[test]
    fn factory_retention_is_lru_and_bounded_by_serialized_weight() {
        ffi::thread_factory_test_limits(2, 32 * 1024 * 1024).unwrap();
        let spec = small();
        for radius in [2., 2.1, 2.2] {
            let _solid = build(
                HelicalThread {
                    major_radius_mm: radius,
                    ..spec
                },
                EntityId::new(),
            );
        }
        let stats = thread_factory_stats().unwrap();
        assert_eq!((stats.entries, stats.evictions), (2, 1));
        assert!(stats.serialized_bytes > 0 && stats.serialized_bytes <= 32 * 1024 * 1024);
        assert!(!ffi::thread_factory_contains(&spec.checked(Default::default()).unwrap()).unwrap());
        assert!(
            ffi::thread_factory_contains(
                &HelicalThread {
                    major_radius_mm: 2.1,
                    ..spec
                }
                .checked(Default::default())
                .unwrap()
            )
            .unwrap()
        );
        let _again = build(spec, EntityId::new());
        assert_eq!(thread_factory_stats().unwrap().constructions, 4);
        assert_eq!(thread_factory_stats().unwrap().entries, 2);
        ffi::thread_factory_test_limits(16, 1).unwrap();
        let _first = build(spec, EntityId::new());
        let _second = build(spec, EntityId::new());
        let stats = thread_factory_stats().unwrap();
        assert_eq!(
            (
                stats.entries,
                stats.serialized_bytes,
                stats.hits,
                stats.constructions
            ),
            (0, 0, 0, 2)
        );
        assert_eq!(stats.unretained, 2);
        assert!(ffi::thread_factory_test_limits(17, 32 * 1024 * 1024).is_err());
        assert!(ffi::thread_factory_test_limits(16, 32 * 1024 * 1024 + 1).is_err());
        reset();
    }

    #[test]
    fn failed_policy_naming_and_abandoned_native_work_are_not_retained() {
        reset();
        let spec = small();
        let data = spec.checked(Default::default()).unwrap();
        let abandoned = ffi::helical_thread(&data).unwrap();
        assert_eq!(thread_factory_stats().unwrap().entries, 0);
        drop(abandoned);
        let pending = ffi::helical_thread(&data).unwrap();
        let restricted = GeometryPolicy {
            maximum_coordinate_mm: 1.9,
            ..Default::default()
        };
        assert!(Solid::adopt(pending, restricted, "factory policy regression").is_err());
        assert_eq!(thread_factory_stats().unwrap().entries, 0);
        let pending = ffi::helical_thread(&data).unwrap();
        let mut unnamed =
            Solid::adopt(pending, Default::default(), "factory name regression").unwrap();
        assert!(unnamed.name_thread(EntityId::nil(), 1).is_err());
        assert_eq!(thread_factory_stats().unwrap().entries, 0);
        drop(unnamed);
        let mut invalid = data;
        invalid.radius = data.depth / 2.;
        for _ in 0..2 {
            assert!(ffi::helical_thread(&invalid).is_err());
            assert!(!ffi::thread_factory_contains(&invalid).unwrap());
            assert_eq!(thread_factory_stats().unwrap().entries, 0);
        }
        let solid = build(spec, EntityId::new());
        assert_eq!(solid.solid_count().unwrap(), 1);
        let stats = thread_factory_stats().unwrap();
        assert_eq!(
            (
                stats.requests,
                stats.constructions,
                stats.entries,
                stats.hits
            ),
            (6, 6, 1, 0)
        );
    }

    #[test]
    fn native_factory_handles_never_cross_owner_threads() {
        reset();
        let spec = small();
        let parent_id = EntityId::new();
        let parent = build(spec, parent_id);
        let baseline = thread_factory_stats().unwrap();
        let mut workers = vec![];
        for _ in 0..2 {
            workers.push(std::thread::spawn(move || {
                assert_eq!(thread_factory_stats().unwrap().entries, 0);
                let a = EntityId::new();
                let b = EntityId::new();
                let first = build(spec, a);
                let second = build(spec, b);
                owns(&first, a, b);
                owns(&second, b, a);
                assert_eq!(first.volume_mm3().unwrap(), second.volume_mm3().unwrap());
                thread_factory_stats().unwrap()
                // Solids and the native TLS cache die on this owning thread.
            }));
        }
        for worker in workers {
            let stats = worker.join().unwrap();
            assert_eq!((stats.constructions, stats.hits, stats.entries), (1, 1, 1));
        }
        assert_eq!(thread_factory_stats().unwrap(), baseline);
        owns(&parent, parent_id, EntityId::nil());
    }
}
