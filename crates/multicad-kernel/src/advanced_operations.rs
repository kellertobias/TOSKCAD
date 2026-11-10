use crate::{
    GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid, TreatmentFace, ffi,
};
use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, TopologyKind},
};
use std::collections::BTreeSet;
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "advanced solid",
        kind,
        message: message.into(),
    }
}
impl Solid {
    /// Copy exact constituent solids without merging touching or overlapping
    /// members. Semantic identity comes from source provenance, never ordering.
    pub fn solid_components(&self, policy: GeometryPolicy) -> Result<Vec<Solid>, GeometryError> {
        self.solid_components_interruptible(policy, None)
    }
    pub fn solid_components_interruptible(
        &self,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Vec<Solid>, GeometryError> {
        policy.validate()?;
        let checkpoint = || {
            if interrupt.is_some_and(|f| f()) {
                Err(failure(
                    GeometryErrorKind::Cancelled,
                    "Solid component extraction superseded",
                ))
            } else {
                Ok(())
            }
        };
        checkpoint()?;
        let count = self.solid_count()?;
        if count == 0 || count > 1024 {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Solid component extraction needs between 1 and 1024 solids",
            ));
        }
        let mut components = Vec::with_capacity(count as usize);
        if self.uses_rust_geometry() {
            if self.geometry_policy() != policy {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Rust component extraction requires the source geometry policy",
                ));
            }
            if let Some(compound) = self.rust_compound() {
                for member in compound.members() {
                    checkpoint()?;
                    for mut child in member.solid_components_interruptible(policy, interrupt)? {
                        child.rust_origins.extend(self.rust_origins.iter().cloned());
                        components.push(child);
                    }
                }
                checkpoint()?;
                return Ok(components);
            }
            if count == 1 {
                return Ok(vec![self.share()?]);
            }
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust component extraction requires independent compound members",
            ));
        }
        let mut sum = 0.;
        for index in 0..count {
            checkpoint()?;
            let handle = ffi::solid_component_at(self.occt()?, index)
                .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
            let mut component = Self::adopt(handle, policy, "solid component")?;
            if component.solid_count()? != 1 {
                return Err(failure(
                    GeometryErrorKind::InvalidResult,
                    "Solid component copy did not retain exactly one solid",
                ));
            }
            component.inherit_provenance(&[self], None)?;
            sum += component.volume_mm3()?;
            components.push(component);
        }
        checkpoint()?;
        let bounds = self.bounds()?;
        let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| bounds.max[i] - bounds.min[i]);
        let original = self.volume_mm3()?;
        let tolerance =
            (original * 1e-8).max(policy.linear_tolerance_mm * 2. * (x * y + y * z + z * x));
        if !sum.is_finite() || (sum - original).abs() > tolerance {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Solid component copies do not conserve the source volume within tolerance",
            ));
        }
        Ok(components)
    }
    fn selected_faces(
        &self,
        faces: &[TreatmentFace],
        feature: EntityId,
        empty_allowed: bool,
    ) -> Result<Vec<u32>, GeometryError> {
        if feature.is_nil() || faces.len() > 128 || (!empty_allowed && faces.is_empty()) {
            return Err(failure(
                GeometryErrorKind::Input,
                "A valid feature identity and bounded explicit face selection are required",
            ));
        }
        if self.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::Input,
                "This operation requires one connected solid",
            ));
        }
        let snapshot = self.topology_snapshot(feature, 0)?;
        let available: BTreeSet<_> = snapshot.elements.iter().map(|e| e.key).collect();
        let mut keys = BTreeSet::new();
        let mut origins = BTreeSet::new();
        for face in faces {
            if face.key.kind != TopologyKind::Face
                || !available.contains(&face.key)
                || !keys.insert(face.key)
                || face.origin.source != feature
                || !origins.insert(face.origin.clone())
                || snapshot
                    .elements
                    .iter()
                    .any(|e| e.origins.contains(&face.origin))
            {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Selected faces must be present, distinct and carry distinct operation-owned source identities",
                ));
            }
            GeometryIntent {
                id: feature,
                kind: TopologyKind::Face,
                origins: BTreeSet::from([face.origin.clone()]),
                boundary: None,
                attachment: None,
            }
            .validate()
            .map_err(|e| failure(GeometryErrorKind::Input, e))?;
        }
        Ok(faces.iter().map(|f| f.key.ordinal).collect())
    }
    /// Positive thickness builds inward; negative builds outward. Closing a
    /// completely hollow part without openings is accepted only when native
    /// topology genuinely contains both closed shells in one solid.
    pub fn shell_named(
        &self,
        openings: &[TreatmentFace],
        thickness_mm: f64,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        policy.length(thickness_mm.abs())?;
        let indices = self.selected_faces(openings, feature, true)?;
        let handle = ffi::shell_solid(
            self.occt()?,
            &indices,
            thickness_mm,
            policy.linear_tolerance_mm,
        )
        .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let mut result = Self::adopt(handle, policy, "shell")?;
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Shell did not produce one connected solid",
            ));
        }
        // OCCT may return a valid-looking offset after the thickness has
        // crossed the opposite wall. Shell material must occupy the requested
        // side of the original solid; local B-rep validity alone cannot prove it.
        let tolerance = (self.volume_mm3()? * 1e-8).max(policy.minimum_feature_mm.powi(3));
        let wrong_side = result.boolean(
            self,
            if thickness_mm > 0. {
                crate::BooleanOperation::Subtract
            } else {
                crate::BooleanOperation::Intersect
            },
            GeometryPolicy {
                boolean_fuzzy_mm: 0.,
                ..policy
            },
        )?;
        let wrong_volume = wrong_side
            .as_ref()
            .map(Solid::volume_mm3)
            .transpose()?
            .unwrap_or(0.);
        if wrong_volume > tolerance
            || (thickness_mm > 0. && result.volume_mm3()? >= self.volume_mm3()? - tolerance)
        {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Shell thickness crosses the source or produces material on the wrong side; reduce thickness",
            ));
        }
        result.inherit_advanced_provenance(
            self,
            &openings
                .iter()
                .map(|f| f.origin.clone())
                .collect::<Vec<_>>(),
            feature,
        )?;
        Ok(result)
    }
    pub fn draft_faces_named(
        &self,
        faces: &[TreatmentFace],
        pull_direction: [f64; 3],
        neutral: PlaneFrame,
        angle_rad: f64,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        let length = pull_direction.iter().map(|v| v * v).sum::<f64>().sqrt();
        if pull_direction.iter().any(|v| !v.is_finite())
            || !length.is_finite()
            || length < 1e-12
            || !angle_rad.is_finite()
            || angle_rad.abs() < 1e-8
            || angle_rad.abs() >= std::f64::consts::FRAC_PI_2
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Draft needs a finite nonzero pull direction and a nonzero signed angle smaller than 90 degrees",
            ));
        }
        let direction = pull_direction.map(|v| v / length);
        let direction = ffi::Point3 {
            x: direction[0],
            y: direction[1],
            z: direction[2],
        };
        let indices = self.selected_faces(faces, feature, false)?;
        let handle = ffi::draft_solid(
            self.occt()?,
            &indices,
            &direction,
            &neutral.checked(policy)?,
            angle_rad,
        )
        .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let mut result = Self::adopt(handle, policy, "draft")?;
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Draft did not produce one connected solid",
            ));
        }
        result.inherit_advanced_provenance(
            self,
            &faces.iter().map(|f| f.origin.clone()).collect::<Vec<_>>(),
            feature,
        )?;
        Ok(result)
    }
    /// Keep exact members separate, even when their volumes overlap. This is
    /// compound construction; explicit Boolean union remains a separate action.
    pub fn compound_named(
        solids: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::compound_named_interruptible(solids, feature, policy, None)
    }
    pub fn compound_named_interruptible(
        solids: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        let checkpoint = || {
            if interrupt.is_some_and(|f| f()) {
                Err(failure(
                    GeometryErrorKind::Cancelled,
                    "Compound construction superseded",
                ))
            } else {
                Ok(())
            }
        };
        checkpoint()?;
        if feature.is_nil() || solids.is_empty() || solids.len() > 128 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Compound needs a feature identity and between 1 and 128 members",
            ));
        }
        let count = solids.iter().try_fold(0u64, |n, solid| {
            solid.solid_count().map(|count| n + u64::from(count))
        })?;
        if count > 1024 {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Compound solid budget exceeded",
            ));
        }
        if solids.iter().any(|solid| solid.uses_rust_geometry()) {
            return Self::make_rust_compound(solids, feature, policy, interrupt);
        }
        let mut job =
            ffi::compound_job().map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        for solid in solids {
            checkpoint()?;
            ffi::compound_add(job.pin_mut(), solid.occt()?)
                .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        }
        checkpoint()?;
        let handle = ffi::compound_finish(job.pin_mut())
            .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let mut result = Self::adopt(handle, policy, "compound")?;
        if u64::from(result.solid_count()?) != count {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Compound changed the number of distinct member solids",
            ));
        }
        result.inherit_provenance(solids, None)?;
        checkpoint()?;
        Ok(result)
    }
}

#[cfg(all(test, feature = "occt"))]
mod compound_property_tests {
    use super::*;
    use crate::{HelicalThread, ffi};
    use multicad_core::references::TopologyKind;
    fn faces_count(s: &Solid) -> u64 {
        ffi::face_integration_count(s.occt().unwrap()).unwrap()
    }
    fn volumes_count(s: &Solid) -> u64 {
        ffi::volume_integration_count(s.occt().unwrap()).unwrap()
    }
    #[test]
    fn curved_compound_copies_keep_certificates_and_distinct_coincident_members() {
        let policy = GeometryPolicy::default();
        let owner = EntityId::new();
        let source = Solid::helical_thread_named(
            HelicalThread {
                major_radius_mm: 1.5,
                pitch_mm: 0.5,
                length_mm: 10.,
                depth_mm: 5. * 3_f64.sqrt() / 32.,
                crest_width_mm: 0.5 / 8.,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                radial_taper: 0.,
                starts: 1,
                left_hand: false,
            },
            owner,
            policy,
        )
        .unwrap();
        let original = source.to_brep().unwrap();
        let before = source.topology_snapshot(owner, 0).unwrap();
        let member = source
            .share()
            .unwrap()
            .with_occurrence_origin(EntityId::new(), EntityId::new())
            .unwrap();
        let singleton = Solid::compound_named(&[&member], EntityId::new(), policy).unwrap();
        assert_eq!(singleton.solid_count().unwrap(), 1);
        assert_eq!(faces_count(&singleton), 0);
        assert_eq!(volumes_count(&singleton), 0);
        assert_eq!(
            singleton.volume_mm3().unwrap(),
            source.volume_mm3().unwrap()
        );
        let nested = Solid::compound_named(&[&singleton], EntityId::new(), policy).unwrap();
        assert_eq!(faces_count(&nested), 0);
        assert_eq!(volumes_count(&nested), 0);
        assert_eq!(nested.volume_mm3().unwrap(), source.volume_mm3().unwrap());
        let second = source
            .share()
            .unwrap()
            .with_occurrence_origin(EntityId::new(), EntityId::new())
            .unwrap();
        let duplicate =
            Solid::compound_named(&[&member, &second], EntityId::new(), policy).unwrap();
        assert_eq!(duplicate.solid_count().unwrap(), 2);
        assert_eq!(faces_count(&duplicate), 0);
        assert_eq!(
            volumes_count(&duplicate),
            1,
            "Multi-member compounds must retain ordinary native GK integration"
        );
        assert_eq!(
            duplicate.faces().unwrap().len(),
            source.faces().unwrap().len() * 2
        );
        let topology = duplicate.topology_snapshot(owner, 0).unwrap();
        assert!(
            topology
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .all(|e| !e.origins.is_empty())
        );
        let directory = tempfile::tempdir().unwrap();
        for (index, solid) in [&singleton, &nested, &duplicate].into_iter().enumerate() {
            let fresh = Solid::from_brep(&solid.to_brep().unwrap(), policy).unwrap();
            assert_eq!(volumes_count(&fresh), 1);
            let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            for (a, b) in solid.faces().unwrap().iter().zip(fresh.faces().unwrap()) {
                assert_eq!(a.index, b.index);
                assert_eq!(a.plane, b.plane);
                assert!((a.area_mm2 - b.area_mm2).abs() < 1e-7 * (a.area_mm2 + b.area_mm2));
                assert!(
                    a.center_mm
                        .into_iter()
                        .zip(b.center_mm)
                        .all(|(x, y)| (x - y).abs() < policy.linear_tolerance_mm)
                );
            }
            let path = directory.path().join(format!("compound-{index}.step"));
            solid.write_step(&path).unwrap();
            let step = Solid::read_step(&path, policy).unwrap();
            assert_eq!(step.solid_count().unwrap(), solid.solid_count().unwrap());
            let budget = policy.linear_tolerance_mm
                * (area
                    + step
                        .faces()
                        .unwrap()
                        .iter()
                        .map(|f| f.area_mm2)
                        .sum::<f64>())
                + 1e-7 * (solid.volume_mm3().unwrap().abs() + step.volume_mm3().unwrap().abs());
            assert!((step.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < budget);
        }
        assert!(ffi::compound_property_fallback_probe(source.occt().unwrap()).unwrap());
        let raw = ffi::shape_from_brep(&original).unwrap();
        let mut job = ffi::compound_job().unwrap();
        ffi::compound_add(job.pin_mut(), &raw).unwrap();
        let handle = ffi::compound_finish(job.pin_mut()).unwrap();
        let cold = Solid::adopt(handle, policy, "cold compound").unwrap();
        assert_eq!(volumes_count(&cold), 1);
        assert_eq!(faces_count(&cold), 0);
        assert_eq!(cold.faces().unwrap().len(), source.faces().unwrap().len());
        assert_eq!(faces_count(&cold), source.faces().unwrap().len() as u64);
        assert!(
            Solid::compound_named_interruptible(
                &[&source],
                EntityId::new(),
                policy,
                Some(&|| true)
            )
            .is_err()
        );
        assert_eq!(source.to_brep().unwrap(), original);
        assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
    }
}
