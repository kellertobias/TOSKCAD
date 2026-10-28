//! Bounded exact taper. At path distance t the material section is offset by
//! t*tan(alpha) in profile UV and translated by t*unit_direction. Polygon joins
//! are miters; positive alpha widens material and shrinks holes. This is an
//! offset rate, independent of oblique sweep direction, not uniform scaling.
use crate::{
    CurvedEdge2d, CurvedProfile, GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi,
};
use multicad_core::{
    document::EntityId,
    references::{Origin, ReferenceFrame},
};
fn input(message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "tapered extrusion",
        kind: GeometryErrorKind::Input,
        message: message.into(),
    }
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn offset(
    wire: &[CurvedEdge2d],
    delta: f64,
    p: GeometryPolicy,
) -> Result<Vec<CurvedEdge2d>, GeometryError> {
    if let [CurvedEdge2d::Circle { center, radius_mm }] = wire {
        let radius_mm = radius_mm + delta;
        if radius_mm < p.minimum_feature_mm || !radius_mm.is_finite() {
            return Err(input(
                "Taper collapses a circular boundary; topology-changing taper is unsupported",
            ));
        }
        return Ok(vec![CurvedEdge2d::Circle {
            center: *center,
            radius_mm,
        }]);
    }
    if wire.len() < 3 || wire.iter().any(|e| !matches!(e, CurvedEdge2d::Line { .. })) {
        return Err(input(
            "Nonzero taper supports strict convex all-line polygons or single-circle loops only; mixed, arc and spline offsets are unsupported",
        ));
    }
    let vertices: Vec<_> = wire
        .iter()
        .map(|e| match e {
            CurvedEdge2d::Line { a, .. } => *a,
            _ => unreachable!(),
        })
        .collect();
    let area: f64 = (0..vertices.len())
        .map(|i| {
            cross(
                sub(vertices[i], vertices[0]),
                sub(vertices[(i + 1) % vertices.len()], vertices[0]),
            )
        })
        .sum();
    if !area.is_finite() || area.abs() < p.minimum_feature_mm.powi(2) {
        return Err(input("Taper polygon has no bounded area"));
    }
    let orientation = area.signum();
    let mut directions = vec![];
    let mut normals = vec![];
    for (i, edge) in wire.iter().enumerate() {
        let CurvedEdge2d::Line { a, b } = edge else {
            unreachable!()
        };
        if *b != vertices[(i + 1) % vertices.len()] {
            return Err(input(
                "Taper polygons require exactly shared endpoints; repair coincident but separate boundary points before tapering",
            ));
        }
        let e = sub(*b, *a);
        let length = e[0].hypot(e[1]);
        let u = e.map(|x| x / length);
        directions.push(u);
        normals.push([orientation * u[1], -orientation * u[0]]);
    }
    for i in 0..wire.len() {
        if orientation * cross(directions[i], directions[(i + 1) % wire.len()]) <= 1e-8 {
            return Err(input(
                "Taper polygons must be strictly convex without collinear or nearly parallel adjacent edges",
            ));
        }
    }
    let mut end = vec![];
    for i in 0..wire.len() {
        let a = normals[(i + wire.len() - 1) % wire.len()];
        let b = normals[i];
        let determinant = cross(a, b);
        end.push([
            vertices[i][0] + delta * (b[1] - a[1]) / determinant,
            vertices[i][1] + delta * (a[0] - b[0]) / determinant,
        ]);
    }
    for i in 0..wire.len() {
        let span = sub(end[(i + 1) % wire.len()], end[i]);
        if dot(span, directions[i]) < p.minimum_feature_mm {
            return Err(input(
                "Taper collapses or reverses a polygon edge; topology-changing offsets are unsupported",
            ));
        }
        for j in 0..wire.len() {
            if dot(normals[j], sub(end[i], vertices[j])) > delta + p.linear_tolerance_mm {
                return Err(input(
                    "Taper changes polygon topology or violates an offset half-plane",
                ));
            }
        }
        p.point([end[i][0], end[i][1], 0.])?;
    }
    Ok((0..wire.len())
        .map(|i| CurvedEdge2d::Line {
            a: end[i],
            b: end[(i + 1) % wire.len()],
        })
        .collect())
}
fn end_profile(
    profile: &CurvedProfile,
    displacement: [f64; 3],
    taper_rad: f64,
    p: GeometryPolicy,
) -> Result<CurvedProfile, GeometryError> {
    profile.checked(p)?;
    p.point(displacement)?;
    let length = displacement.into_iter().map(|x| x * x).sum::<f64>().sqrt();
    p.length(length)?;
    let height: f64 = displacement
        .into_iter()
        .zip(profile.frame.normal())
        .map(|(a, b)| a * b)
        .sum();
    if height.abs() < p.minimum_feature_mm {
        return Err(input(
            "Taper sweep must cross the profile plane with finite policy-sized height",
        ));
    }
    if !taper_rad.is_finite() || taper_rad.abs() >= std::f64::consts::FRAC_PI_2 {
        return Err(input(
            "Taper angle must be finite and strictly between -90 and 90 degrees",
        ));
    }
    let delta = length * taper_rad.tan();
    if !delta.is_finite() || delta.abs() > p.maximum_coordinate_mm {
        return Err(input("Taper offset exceeds the numerical policy"));
    }
    if delta.abs() < p.minimum_feature_mm {
        return Err(input(
            "Nonzero taper offset is below the minimum feature size",
        ));
    }
    let mut end = CurvedProfile {
        frame: profile.frame,
        outer: offset(&profile.outer, delta, p)?,
        holes: profile
            .holes
            .iter()
            .map(|wire| offset(wire, -delta, p))
            .collect::<Result<_, _>>()?,
    };
    end.frame.origin = std::array::from_fn(|i| profile.frame.origin[i] + displacement[i]);
    end.checked(p)?;
    Ok(end)
}
impl Solid {
    pub fn extrude_curved_profile_vector_tapered(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        taper_rad: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if taper_rad == 0. {
            return Self::extrude_curved_profile_vector(profile, displacement_mm, policy);
        }
        let end = end_profile(profile, displacement_mm, taper_rad, policy)?;
        let a = profile.checked(policy)?;
        let b = end.checked(policy)?;
        let solid = Self::adopt(
            ffi::curved_profile_ruled(
                &a.frame,
                &b.frame,
                &a.edges,
                &b.edges,
                &a.offsets,
                policy.linear_tolerance_mm,
            )
            .map_err(|e| GeometryError {
                operation: "tapered extrusion",
                kind: GeometryErrorKind::Native,
                message: e.to_string(),
            })?,
            policy,
            "tapered extrusion",
        )?;
        if solid.solid_count()? != 1 {
            return Err(input("Taper must produce one closed connected solid"));
        }
        Ok(solid)
    }
    pub fn extrude_curved_profile_vector_tapered_named(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        taper_rad: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if taper_rad == 0. {
            return Self::extrude_curved_profile_vector_named(
                profile,
                displacement_mm,
                feature,
                origins,
                policy,
            );
        }
        let end = end_profile(profile, displacement_mm, taper_rad, policy)?;
        let edges: Vec<_> = profile
            .outer
            .iter()
            .chain(profile.holes.iter().flatten())
            .collect();
        let end_edges: Vec<_> = end.outer.iter().chain(end.holes.iter().flatten()).collect();
        if feature.is_nil() || origins.len() != edges.len() {
            return Err(input(
                "Every taper edge needs an authored origin and feature identity",
            ));
        }
        let mut solid = Self::extrude_curved_profile_vector_tapered(
            profile,
            displacement_mm,
            taper_rad,
            policy,
        )?;
        let frames: Vec<_> = edges
            .into_iter()
            .zip(end_edges)
            .map(|(a, b)| match (a, b) {
                (CurvedEdge2d::Line { a, b }, CurvedEdge2d::Line { a: end_a, .. }) => {
                    let a = profile.frame.point(*a);
                    let b = profile.frame.point(*b);
                    let c = end.frame.point(*end_a);
                    let length = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
                    let u: [f64; 3] = std::array::from_fn(|i| (b[i] - a[i]) / length);
                    let along: f64 = (0..3).map(|i| u[i] * (c[i] - a[i])).sum();
                    let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i] - u[i] * along);
                    let length = v.into_iter().map(|x| x * x).sum::<f64>().sqrt();
                    Some(ReferenceFrame {
                        origin_mm: a,
                        u,
                        v: v.map(|x| x / length),
                    })
                }
                _ => None,
            })
            .collect();
        let rf = |f: crate::PlaneFrame| ReferenceFrame {
            origin_mm: f.origin,
            u: f.u,
            v: f.v,
        };
        solid.name_profile_sweep(
            [Some(rf(profile.frame)), Some(rf(end.frame))],
            feature,
            origins,
            &frames,
        )?;
        Ok(solid)
    }
}
