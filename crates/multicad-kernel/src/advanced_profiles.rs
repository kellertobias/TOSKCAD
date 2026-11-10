//! Closed profile sweeps and ordered lofts with authored curve identities.
//! Compatibility guessing, profile relocation and implicit correction
//! are disabled; unsupported holes/path/correspondence remain explicit errors.
use crate::{
    CurvedEdge2d, CurvedProfile, GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    Solid, ffi,
};
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
    let authored: BTreeSet<_> = origins.iter().collect();
    if feature.is_nil()
        || origins.is_empty()
        || origins.len() > 1024
        || authored.len() != origins.len()
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
        let side = Origin {
            source: feature,
            role: format!("side/{}/{}", origin.source, origin.role),
        };
        if authored.contains(&side) {
            return Err(failure(
                GeometryErrorKind::Input,
                "Authored curve origin collides with a generated side identity",
            ));
        }
        GeometryIntent {
            id: feature,
            kind: TopologyKind::Face,
            origins: BTreeSet::from([origin.clone(), side]),
            boundary: None,
            attachment: None,
        }
        .validate()
        .map_err(|e| failure(GeometryErrorKind::Input, e))?;
    }
    Ok(())
}
fn sweep_origins(
    profile: &CurvedProfile,
    path_mm: &[[f64; 3]],
    feature: EntityId,
    profile_origins: &[Origin],
    path_origins: &[Origin],
) -> Result<Vec<Origin>, GeometryError> {
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
    let origins: Vec<_> = profile_origins
        .iter()
        .chain(path_origins)
        .cloned()
        .collect();
    origins_valid(feature, &origins)?;
    Ok(origins)
}

fn sweep_start_height(
    frame: PlaneFrame,
    start: [f64; 3],
    end: [f64; 3],
    policy: GeometryPolicy,
) -> Result<f64, GeometryError> {
    frame.checked(policy)?;
    policy.point(start)?;
    policy.point(end)?;
    let offset = std::array::from_fn(|i| start[i] - frame.origin[i]);
    let delta: [f64; 3] = std::array::from_fn(|i| end[i] - start[i]);
    let length = delta[0].hypot(delta[1]).hypot(delta[2]);
    policy.length(length)?;
    let normal = frame.normal();
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let height = dot(delta, normal) / dot(normal, normal);
    let lateral: [f64; 3] = std::array::from_fn(|i| delta[i] - height * normal[i]);
    let roundoff = 64.
        * f64::EPSILON
        * start
            .into_iter()
            .chain(end)
            .map(f64::abs)
            .fold(length.max(1.), f64::max);
    if dot(offset, normal).abs() > policy.linear_tolerance_mm
        || lateral[0].hypot(lateral[1]).hypot(lateral[2]) > roundoff.min(policy.linear_tolerance_mm)
    {
        return Err(failure(
            GeometryErrorKind::Input,
            "Sweep profile plane must contain the path start and be perpendicular to its first segment",
        ));
    }
    Ok(height)
}

impl Solid {
    /// Sweep a closed profile along an authored straight or mitered polyline.
    pub fn rust_sweep_polyline_named(
        profile: &CurvedProfile,
        path_mm: &[[f64; 3]],
        feature: EntityId,
        profile_origins: &[Origin],
        path_origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if path_mm.len() == 2 {
            return Self::rust_sweep_straight_named(
                profile,
                path_mm,
                feature,
                profile_origins,
                path_origins,
                policy,
            );
        }
        policy.validate()?;
        sweep_origins(profile, path_mm, feature, profile_origins, path_origins)?;
        if let ([CurvedEdge2d::Circle { center, radius_mm }], &[start, corner, end]) =
            (profile.outer.as_slice(), path_mm)
        {
            sweep_start_height(profile.frame, start, corner, policy)?;
            return Self::rust_sweep_circular_miter_named(
                profile.frame,
                *center,
                *radius_mm,
                [start, corner, end],
                feature,
                [&profile_origins[0], &path_origins[0], &path_origins[1]],
                policy,
            );
        }
        Self::rust_sweep_miter_named(
            profile,
            path_mm,
            feature,
            profile_origins,
            path_origins,
            policy,
        )
    }

    /// A straight path has a constant corrected Frenet frame. The authored
    /// section translates along that segment with no relocation or rotation.
    pub fn rust_sweep_straight_named(
        profile: &CurvedProfile,
        path_mm: &[[f64; 3]],
        feature: EntityId,
        profile_origins: &[Origin],
        path_origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        sweep_origins(profile, path_mm, feature, profile_origins, path_origins)?;
        let &[start, end] = path_mm else {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust sweep supports one straight path segment; corner transitions are unavailable",
            ));
        };
        let height = sweep_start_height(profile.frame, start, end, policy)?;
        Self::rust_extrude_normal_profile_with_path(
            profile,
            height,
            feature,
            profile_origins,
            Some(path_origins[0].clone()),
            policy,
        )
    }

    /// Sweep one outer wire along an open world-space polyline. The profile
    /// starts in its given plane, normal to the first path segment. Every path
    /// segment has a stable supplied origin; corners use native right transitions.
    pub fn sweep_polyline_named(
        profile: &CurvedProfile,
        path_mm: &[[f64; 3]],
        feature: EntityId,
        profile_origins: &[Origin],
        path_origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        let origins = sweep_origins(profile, path_mm, feature, profile_origins, path_origins)?;
        let checked = profile.checked(policy)?;
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
    /// Loft ordered authored sections with exact planar or round walls.
    pub fn rust_loft_profiles_named(
        profiles: &[CurvedProfile],
        feature: EntityId,
        section_origins: &[Vec<Origin>],
        ruled: bool,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if ruled
            && profiles.iter().all(|profile| {
                profile
                    .outer
                    .iter()
                    .all(|edge| matches!(edge, CurvedEdge2d::Line { .. }))
            })
        {
            return Self::rust_loft_ruled_polygons_named(
                profiles,
                feature,
                section_origins,
                policy,
            );
        }
        if !ruled && profiles.len() != 2 {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust loft needs ruled construction for more than two sections",
            ));
        }
        Self::rust_loft_round_sections(profiles, feature, section_origins, policy)
    }

    /// Loft two full circles on one axis with the same authored seam phase.
    pub fn rust_loft_coaxial_circles_named(
        profiles: &[CurvedProfile],
        feature: EntityId,
        section_origins: &[Vec<Origin>],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if profiles.len() != 2 {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust loft requires two ordered circular sections",
            ));
        }
        Self::rust_loft_round_sections(profiles, feature, section_origins, policy)
    }

    fn rust_loft_round_sections(
        profiles: &[CurvedProfile],
        feature: EntityId,
        section_origins: &[Vec<Origin>],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if !(2..=16).contains(&profiles.len()) || section_origins.len() != profiles.len() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Rust loft needs 2 to 16 ordered sections with authored identities",
            ));
        }
        let start = profiles[0].frame;
        let normal = start.normal();
        let distance = |a: [f64; 3], b: [f64; 3]| {
            let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
            x.hypot(y).hypot(z)
        };
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        let angle_eps = 64. * f64::EPSILON;
        let mut sections: Vec<[f64; 2]> = Vec::with_capacity(profiles.len());
        let mut origins = Vec::with_capacity(profiles.len());
        let mut axis_origin = [0.; 3];
        let mut sign = 1.;
        for (index, (profile, section_origins)) in profiles.iter().zip(section_origins).enumerate()
        {
            let ([CurvedEdge2d::Circle { center, radius_mm }], [origin], []) = (
                profile.outer.as_slice(),
                section_origins.as_slice(),
                profile.holes.as_slice(),
            ) else {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust round loft needs one full authored circle without holes in each section",
                ));
            };
            profile.frame.checked(policy)?;
            let center = profile.frame.point(*center);
            policy.point(center)?;
            policy.length(*radius_mm)?;
            if distance(start.u, profile.frame.u) > angle_eps
                || distance(start.v, profile.frame.v) > angle_eps
            {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust loft requires matching circle seam directions",
                ));
            }
            if index == 0 {
                axis_origin = center;
            }
            let delta = std::array::from_fn(|i| center[i] - axis_origin[i]);
            let height = dot(delta, normal) / dot(normal, normal);
            if index == 1 {
                sign = height.signum();
            }
            if let Some(&[_, previous]) = sections.last() {
                policy.length(height * sign - previous)?;
            }
            let roundoff = axis_origin
                .into_iter()
                .chain(center)
                .map(f64::abs)
                .fold(height.abs().max(1.), f64::max)
                * angle_eps;
            if distance(delta, normal.map(|component| component * height))
                > roundoff.min(policy.linear_tolerance_mm)
            {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust loft requires coaxial circle centers",
                ));
            }
            sections.push([*radius_mm, height * sign]);
            origins.push(origin.clone());
        }
        origins_valid(feature, &origins)?;
        let frame = PlaneFrame {
            origin: axis_origin,
            u: start.u,
            v: start.v.map(|component| component * sign),
        };
        Self::rust_revolve_circle_loft(frame, &sections, feature, &origins, policy)
    }

    /// Loft 2..16 closed sections in supplied order, keeping their edge order
    /// and winding. Matching edge kinds/counts define explicit correspondence.
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
