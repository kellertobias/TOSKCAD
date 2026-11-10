//! Exact revolutions of bounded planar sections. The initial support gate keeps
//! the axis in the section plane and the entire section on one side of it.
use crate::{
    CurvedEdge2d, CurvedProfile, GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi,
};
use multicad_core::{
    document::EntityId,
    references::{Origin, ReferenceFrame},
};
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RevolutionAxis {
    pub origin_mm: [f64; 3],
    /// Unit world-space direction; angles use the right-hand rule.
    pub direction: [f64; 3],
}
fn error(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "revolution",
        kind,
        message: message.into(),
    }
}
fn input(message: impl Into<String>) -> GeometryError {
    error(GeometryErrorKind::Input, message)
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(v: [f64; 3]) -> [f64; 3] {
    let n = dot(v, v).sqrt();
    v.map(|x| x / n)
}
fn point(v: [f64; 3]) -> ffi::Point3 {
    ffi::Point3 {
        x: v[0],
        y: v[1],
        z: v[2],
    }
}
impl RevolutionAxis {
    fn rotate_vector(self, v: [f64; 3], angle: f64) -> [f64; 3] {
        let (s, c) = angle.sin_cos();
        let n = cross(self.direction, v);
        let d = dot(self.direction, v);
        std::array::from_fn(|i| v[i] * c + n[i] * s + self.direction[i] * d * (1. - c))
    }
    fn rotate_point(self, p: [f64; 3], angle: f64) -> [f64; 3] {
        let v = self.rotate_vector(sub(p, self.origin_mm), angle);
        std::array::from_fn(|i| self.origin_mm[i] + v[i])
    }
    pub(crate) fn validate(
        self,
        profile: &CurvedProfile,
        angle: f64,
        policy: GeometryPolicy,
    ) -> Result<(), GeometryError> {
        policy.point(self.origin_mm)?;
        if self.direction.into_iter().any(|x| !x.is_finite())
            || (dot(self.direction, self.direction) - 1.).abs() > 1e-10
        {
            return Err(input(
                "Revolution axis direction must be finite and unit length",
            ));
        }
        let n = profile.frame.normal();
        if dot(self.direction, n).abs() > 1e-10
            || dot(sub(self.origin_mm, profile.frame.origin), n).abs() > policy.linear_tolerance_mm
        {
            return Err(input("Revolution axis must lie in the profile plane"));
        }
        if !angle.is_finite() || angle == 0. || angle.abs() > TAU {
            return Err(input(
                "Revolution angle must be nonzero and no greater than one full turn",
            ));
        }
        // Positive rational splines remain inside their control hull. This is
        // conservative but exact: a crossing hull is rejected, never sampled.
        let radial = unit(cross(self.direction, n));
        let au = dot(radial, profile.frame.u);
        let av = dot(radial, profile.frame.v);
        let signed = |p: [f64; 2]| dot(sub(profile.frame.point(p), self.origin_mm), radial);
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        let mut add = |x: f64| {
            lo = lo.min(x);
            hi = hi.max(x);
        };
        for edge in profile.outer.iter().chain(profile.holes.iter().flatten()) {
            match edge {
                CurvedEdge2d::Line { a, b } => {
                    add(signed(*a));
                    add(signed(*b));
                }
                CurvedEdge2d::BSpline(s) => {
                    for p in &s.poles {
                        add(signed(*p));
                    }
                }
                CurvedEdge2d::Circle { center, radius_mm } => {
                    let r = radius_mm * au.hypot(av);
                    add(signed(*center) - r);
                    add(signed(*center) + r);
                }
                CurvedEdge2d::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                } => {
                    add(signed(*a));
                    add(signed(*b));
                    let start = (a[1] - center[1]).atan2(a[0] - center[0]);
                    let end = (b[1] - center[1]).atan2(b[0] - center[0]);
                    let sweep = if *clockwise { start - end } else { end - start }.rem_euclid(TAU);
                    let r = (a[0] - center[0]).hypot(a[1] - center[1]);
                    for t in [av.atan2(au), av.atan2(au) + PI] {
                        let delta = if *clockwise { start - t } else { t - start }.rem_euclid(TAU);
                        if delta <= sweep {
                            add(signed(*center) + r * (au * t.cos() + av * t.sin()));
                        }
                    }
                }
            }
        }
        if lo < -policy.linear_tolerance_mm && hi > policy.linear_tolerance_mm {
            return Err(input(
                "Revolution profile crosses the axis (or its spline control hull does); split it into a one-sided section",
            ));
        }
        let radius = lo.abs().max(hi.abs());
        if radius * angle.abs() < policy.minimum_feature_mm
            || (angle.abs() != TAU && radius * (TAU - angle.abs()) < policy.minimum_feature_mm)
        {
            return Err(input(
                "Revolution sweep or remaining full-turn gap is below the minimum feature size",
            ));
        }
        Ok(())
    }
}
impl Solid {
    pub fn revolve_curved_profile(
        profile: &CurvedProfile,
        axis: RevolutionAxis,
        angle_rad: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let checked = profile.checked(policy)?;
        axis.validate(profile, angle_rad, policy)?;
        Self::adopt(
            ffi::curved_profile_revolve(
                &checked.frame,
                &checked.edges,
                &checked.offsets,
                policy.linear_tolerance_mm,
                &point(axis.origin_mm),
                &point(axis.direction),
                angle_rad,
            )
            .map_err(|e| error(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "revolution",
        )
    }
    pub fn revolve_curved_profile_named(
        profile: &CurvedProfile,
        axis: RevolutionAxis,
        angle_rad: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let edges: Vec<_> = profile
            .outer
            .iter()
            .chain(profile.holes.iter().flatten())
            .collect();
        if feature.is_nil() || origins.len() != edges.len() {
            return Err(input(
                "Every revolution profile edge needs an authored origin and feature identity",
            ));
        }
        let mut solid = Self::revolve_curved_profile(profile, axis, angle_rad, policy)?;
        let start = ReferenceFrame {
            origin_mm: profile.frame.origin,
            u: profile.frame.u,
            v: profile.frame.v,
        };
        let caps = if angle_rad.abs() == TAU {
            [None, None]
        } else {
            [
                Some(start),
                Some(ReferenceFrame {
                    origin_mm: axis.rotate_point(start.origin_mm, angle_rad),
                    u: axis.rotate_vector(start.u, angle_rad),
                    v: axis.rotate_vector(start.v, angle_rad),
                }),
            ]
        };
        // Only native-confirmed planar generated faces get attachment frames.
        // Cylindrical/conical/spherical/spline surfaces deliberately have none.
        let faces = solid.faces()?;
        let mut frames = vec![None; edges.len()];
        let mut seen = vec![false; edges.len()];
        let mut rejected = vec![false; edges.len()];
        for seed in ffi::seeds(solid.occt()?)
            .map_err(|e| error(GeometryErrorKind::Native, e.to_string()))?
        {
            if seed.role != 2 {
                continue;
            }
            let i = seed.source_index as usize;
            let edge = edges
                .get(i)
                .ok_or_else(|| input("Missing revolution edge origin"))?;
            let face = faces
                .iter()
                .find(|f| f.index == seed.face)
                .ok_or_else(|| input("Missing revolution face"))?;
            let Some((origin, normal)) = face.plane else {
                rejected[i] = true;
                continue;
            };
            let local = match edge {
                CurvedEdge2d::Line { a, .. } | CurvedEdge2d::Arc { a, .. } => *a,
                CurvedEdge2d::Circle { center, radius_mm } => [center[0] + radius_mm, center[1]],
                CurvedEdge2d::BSpline(s) => s.poles[0],
            };
            let anchor = profile.frame.point(local);
            if dot(sub(anchor, origin), normal).abs() > policy.linear_tolerance_mm {
                rejected[i] = true;
                continue;
            }
            let normal = if dot(normal, axis.direction) < 0. {
                normal.map(|n| -n)
            } else {
                normal
            };
            let mut u = sub(
                profile.frame.u,
                normal.map(|n| n * dot(profile.frame.u, normal)),
            );
            if dot(u, u) < 1e-12 {
                u = sub(
                    profile.frame.v,
                    normal.map(|n| n * dot(profile.frame.v, normal)),
                );
            }
            let u = unit(u);
            let candidate = ReferenceFrame {
                origin_mm: anchor,
                u,
                v: cross(normal, u),
            };
            if seen[i] && frames[i] != Some(candidate) {
                rejected[i] = true;
            }
            frames[i] = Some(candidate);
            seen[i] = true;
        }
        for (i, rejected) in rejected.into_iter().enumerate() {
            if rejected {
                frames[i] = None;
            }
        }
        solid.name_profile_sweep(caps, feature, origins, &frames)?;
        Ok(solid)
    }
}
