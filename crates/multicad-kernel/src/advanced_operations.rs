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
        let mut sum = 0.;
        for index in 0..count {
            checkpoint()?;
            let handle = ffi::solid_component_at(&self.native.0, index)
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
            &self.native.0,
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
            &self.native.0,
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
        policy.validate()?;
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
        let mut job =
            ffi::compound_job().map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        for solid in solids {
            ffi::compound_add(job.pin_mut(), &solid.native.0)
                .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        }
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
        Ok(result)
    }
}
