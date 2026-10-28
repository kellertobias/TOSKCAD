use crate::{CurvedProfile, GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, TopologyKind},
};
use std::collections::BTreeSet;
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "advanced profile",
        kind,
        message: message.into(),
    }
}
fn origins_valid(feature: EntityId, origins: &[Origin]) -> Result<(), GeometryError> {
    if feature.is_nil()
        || origins.is_empty()
        || origins.len() > 1024
        || origins.iter().collect::<BTreeSet<_>>().len() != origins.len()
    {
        return Err(failure(
            GeometryErrorKind::Input,
            "Profile operation needs a feature identity and distinct authored curve origins",
        ));
    }
    for origin in origins {
        if origin.source == feature && ["cap.start", "cap.end"].contains(&origin.role.as_str()) {
            return Err(failure(
                GeometryErrorKind::Input,
                "Authored curve origin collides with a generated cap identity",
            ));
        }
        GeometryIntent {
            id: feature,
            kind: TopologyKind::Face,
            origins: BTreeSet::from([
                origin.clone(),
                Origin {
                    source: feature,
                    role: format!("side/{}/{}", origin.source, origin.role),
                },
            ]),
            boundary: None,
            attachment: None,
        }
        .validate()
        .map_err(|e| failure(GeometryErrorKind::Input, e))?;
    }
    Ok(())
}
impl Solid {
    pub fn sweep_polyline_named(
        profile: &CurvedProfile,
        path_mm: &[[f64; 3]],
        feature: EntityId,
        profile_origins: &[Origin],
        path_origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if !profile.holes.is_empty() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Sweep currently supports one closed outer profile without holes",
            ));
        }
        if !(2..=65).contains(&path_mm.len())
            || profile_origins.len() != profile.outer.len()
            || path_origins.len() + 1 != path_mm.len()
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Sweep needs 2 to 65 ordered path points and an authored origin for every profile/path edge",
            ));
        }
        let checked = profile.checked(policy)?;
        let origins: Vec<_> = profile_origins
            .iter()
            .chain(path_origins)
            .cloned()
            .collect();
        origins_valid(feature, &origins)?;
        let mut path = vec![];
        for point in path_mm {
            policy.point(*point)?;
            path.push(ffi::Point3 {
                x: point[0],
                y: point[1],
                z: point[2],
            });
        }
        for segment in path_mm.windows(2) {
            policy.length(
                (0..3)
                    .map(|i| (segment[1][i] - segment[0][i]).powi(2))
                    .sum::<f64>()
                    .sqrt(),
            )?;
        }
        let offset: [f64; 3] = std::array::from_fn(|i| path_mm[0][i] - profile.frame.origin[i]);
        let first: [f64; 3] = std::array::from_fn(|i| path_mm[1][i] - path_mm[0][i]);
        let length = first.iter().map(|v| v * v).sum::<f64>().sqrt();
        let normal = profile.frame.normal();
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        if dot(offset, normal).abs() > policy.linear_tolerance_mm
            || (dot(first.map(|v| v / length), normal).abs() - 1.).abs() > 1e-10
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Sweep profile plane must contain the path start and be perpendicular to its first segment",
            ));
        }
        if path_mm[0]
            .into_iter()
            .zip(path_mm[path_mm.len() - 1])
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
            < policy.minimum_feature_mm
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Sweep requires an open path with distinct endpoints",
            ));
        }
        let mut result = Self::adopt(
            ffi::curved_profile_pipe(
                &checked.frame,
                &checked.edges,
                &path,
                policy.linear_tolerance_mm,
            )
            .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "sweep",
        )?;
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Sweep did not produce one connected solid",
            ));
        }
        result.name_advanced_profile(feature, &origins)?;
        Ok(result)
    }
    pub fn loft_profiles_named(
        profiles: &[CurvedProfile],
        feature: EntityId,
        section_origins: &[Vec<Origin>],
        ruled: bool,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if !(2..=16).contains(&profiles.len()) || section_origins.len() != profiles.len() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Loft needs 2 to 16 explicitly ordered closed sections",
            ));
        }
        let count = profiles[0].outer.len();
        let mut sections = vec![];
        for (profile, origins) in profiles.iter().zip(section_origins) {
            if !profile.holes.is_empty()
                || profile.outer.len() != count
                || origins.len() != count
                || profile
                    .outer
                    .iter()
                    .zip(&profiles[0].outer)
                    .any(|(a, b)| std::mem::discriminant(a) != std::mem::discriminant(b))
            {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Loft sections need one closed outer wire, no holes, and matching authored edge counts/kinds; repair correspondence explicitly",
                ));
            }
            let checked = profile.checked(policy)?;
            sections.push(ffi::LoftSectionData {
                frame: checked.frame,
                edges: checked.edges,
            });
        }
        let origins: Vec<_> = section_origins.iter().flatten().cloned().collect();
        origins_valid(feature, &origins)?;
        let mut result = Self::adopt(
            ffi::curved_profile_loft(&sections, ruled, policy.linear_tolerance_mm)
                .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "loft",
        )?;
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Loft did not produce one connected solid",
            ));
        }
        result.name_advanced_profile(feature, &origins)?;
        Ok(result)
    }
}
