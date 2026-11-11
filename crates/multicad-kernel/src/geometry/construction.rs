//! Shared owned-kernel construction behind the public constructors in builds
//! without OpenCascade. Enabled native constructors retain their oracle path.
use super::*;
use crate::{CurvedEdge2d, CurvedProfile};
use std::collections::BTreeSet;

fn exact(value: f64) -> Result<Rational, GeometryError> {
    if !value.is_finite() {
        return Err(input("Construction length must be finite"));
    }
    Rational::parse_decimal(&value.to_string()).map_err(|cause| {
        error(
            "construction",
            GeometryErrorKind::Limit,
            format!("Length exceeds the exact decimal arithmetic budget: {cause:?}"),
        )
    })
}
fn cylinder_coordinates(frame: PlaneFrame) -> [Rational; 3] {
    let zero = Rational::new(0, 1).expect("zero");
    if frame.u[2] == 0.
        && frame.v[2] == 0.
        && frame.normal()[2] > 0.
        && let [Ok(x), Ok(y), Ok(z)] = frame.origin.map(exact)
    {
        return [x, y, z];
    }
    // A local exact circle plus the unchanged floating-point pose remains
    // authoritative when an exact world-coordinate optimization is unavailable.
    [zero; 3]
}
pub(crate) fn origins(feature: EntityId, count: usize) -> Vec<Origin> {
    (0..count)
        .map(|i| Origin {
            source: feature,
            role: format!("profile.edge.{i}"),
        })
        .collect()
}
fn validate_origins(
    feature: EntityId,
    origins: &[Origin],
    count: usize,
) -> Result<(), GeometryError> {
    if feature.is_nil()
        || origins.len() != count
        || origins.iter().collect::<BTreeSet<_>>().len() != count
    {
        return Err(input(
            "Every profile curve needs a distinct source origin and feature identity",
        ));
    }
    for origin in origins {
        let probe = multicad_core::references::GeometryIntent {
            id: EntityId::new(),
            kind: multicad_core::references::TopologyKind::Face,
            origins: BTreeSet::from([
                origin.clone(),
                Origin {
                    source: feature,
                    role: format!("side/{}/{}", origin.source, origin.role),
                },
            ]),
            boundary: None,
            attachment: None,
        };
        probe.validate().map_err(input)?;
    }
    Ok(())
}
pub(crate) fn box_solid(
    size: [f64; 3],
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    policy.validate()?;
    for value in size {
        policy.length(value)?;
    }
    let size = [exact(size[0])?, exact(size[1])?, exact(size[2])?];
    let prism = Prism::rectangular(feature.uuid_bits(), size).map_err(|cause| {
        error(
            "box",
            if cause
                == multicad_rust_kernel::prism::PrismError::Arithmetic(
                    multicad_rust_kernel::Error::OutOfRange,
                )
            {
                GeometryErrorKind::Limit
            } else {
                GeometryErrorKind::Input
            },
            format!("Rust box admission failed: {cause:?}"),
        )
    })?;
    Solid::from_rust_prism(prism, policy)
}
pub(crate) fn named_box(
    size: [f64; 3],
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    policy.validate()?;
    for value in size {
        policy.length(value)?;
    }
    let curves = ["y.min", "x.max", "y.max", "x.min"].map(|role| Origin {
        source: feature,
        role: role.into(),
    });
    validate_origins(feature, &curves, 4)?;
    let [w, d, h] = [exact(size[0])?, exact(size[1])?, exact(size[2])?];
    let zero = Rational::new(0, 1).expect("zero");
    let shape = PolygonPrism::new(vec![vec![[zero, zero], [w, zero], [w, d], [zero, d]]], h)
        .map_err(|cause| {
            error(
                "box",
                GeometryErrorKind::Limit,
                format!("Box exceeds exact arithmetic admission: {cause:?}"),
            )
        })?;
    Solid::from_polygon_prism_sweep(
        shape,
        PlaneFrame::default(),
        false,
        feature,
        &curves,
        &[false; 4],
        policy,
    )
}
pub(crate) fn cylinder(
    frame: PlaneFrame,
    radius: f64,
    height: f64,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    policy.validate()?;
    frame.checked(policy)?;
    policy.length(radius)?;
    policy.length(height)?;
    let [x, y, z] = cylinder_coordinates(frame);
    let circle = Circle::new([x, y], exact(radius)?)
        .map_err(|cause| input(format!("Invalid cylinder radius: {cause:?}")))?;
    let cylinder =
        AxialCylinder::new(feature.uuid_bits(), circle, z, exact(height)?).map_err(|cause| {
            error(
                "cylinder",
                GeometryErrorKind::Limit,
                format!("Cylinder exceeds exact arithmetic admission: {cause:?}"),
            )
        })?;
    // Non-axial stock uses the same exact local descriptor and its proper pose.
    Solid::from_axial_cylinder_sweep(
        cylinder,
        frame,
        [0.; 2],
        false,
        Origin {
            source: feature,
            role: "primitive.circle".into(),
        },
        policy,
    )
}
pub(crate) fn profile(
    source: &Profile,
    height: f64,
    feature: EntityId,
    origins: &[Origin],
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    policy.validate()?;
    source.checked(policy)?;
    let convert = |edge: &ProfileEdge| match *edge {
        ProfileEdge::Line { a, b } => CurvedEdge2d::Line { a, b },
        ProfileEdge::Arc {
            center,
            a,
            b,
            clockwise,
        } => CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        },
        ProfileEdge::Circle { center, radius_mm } => CurvedEdge2d::Circle { center, radius_mm },
    };
    normal(
        &CurvedProfile {
            frame: source.frame,
            outer: source.outer.iter().map(convert).collect(),
            holes: source
                .holes
                .iter()
                .map(|wire| wire.iter().map(convert).collect())
                .collect(),
        },
        height,
        feature,
        origins,
        policy,
    )
}
pub(crate) fn normal(
    profile: &CurvedProfile,
    height: f64,
    feature: EntityId,
    origins: &[Origin],
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    policy.validate()?;
    policy.length(height.abs())?;
    validate_origins(
        feature,
        origins,
        profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>(),
    )?;
    let all_straight = profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
        .all(|edge| matches!(edge, CurvedEdge2d::Line { .. }));
    if all_straight {
        return Solid::rust_extrude_linear_validated_named(
            profile,
            profile.frame.normal().map(|n| n * height),
            0.,
            feature,
            origins,
            policy,
        );
    }
    if let ([CurvedEdge2d::Circle { center, radius_mm }], [], [origin]) =
        (profile.outer.as_slice(), profile.holes.as_slice(), origins)
    {
        let reverse = height < 0.;
        let frame = PlaneFrame {
            origin: profile.frame.point(*center),
            u: profile.frame.u,
            v: profile.frame.v.map(|v| if reverse { -v } else { v }),
        };
        let [x, y, z] = cylinder_coordinates(frame);
        let circle = Circle::new([x, y], exact(*radius_mm)?)
            .map_err(|cause| input(format!("Invalid circular profile: {cause:?}")))?;
        let cylinder = AxialCylinder::new(feature.uuid_bits(), circle, z, exact(height.abs())?)
            .map_err(|cause| {
                error(
                    "circle extrusion",
                    GeometryErrorKind::Limit,
                    format!("Circular extrusion exceeds exact arithmetic admission: {cause:?}"),
                )
            })?;
        return Solid::from_axial_cylinder_sweep(
            cylinder,
            frame,
            *center,
            reverse,
            origin.clone(),
            policy,
        );
    }
    if let (
        [
            CurvedEdge2d::Circle {
                center,
                radius_mm: outer,
            },
        ],
        [hole],
        [outer_origin, inner_origin],
    ) = (profile.outer.as_slice(), profile.holes.as_slice(), origins)
        && let [
            CurvedEdge2d::Circle {
                center: inner_center,
                radius_mm: inner,
            },
        ] = hole.as_slice()
        && center == inner_center
    {
        return Solid::rust_annulus_from_sketch(
            [*inner, *outer],
            height,
            feature,
            profile.frame,
            *center,
            [inner_origin.clone(), outer_origin.clone()],
            policy,
        );
    }
    Solid::rust_extrude_normal_profile_named(profile, height, feature, origins, policy)
}
pub(crate) fn vector(
    profile: &CurvedProfile,
    displacement: [f64; 3],
    feature: EntityId,
    origins: &[Origin],
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    policy.validate()?;
    policy.point(displacement)?;
    validate_origins(
        feature,
        origins,
        profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>(),
    )?;
    if profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
        .all(|edge| matches!(edge, CurvedEdge2d::Line { .. }))
    {
        return Solid::rust_extrude_linear_validated_named(
            profile,
            displacement,
            0.,
            feature,
            origins,
            policy,
        );
    }
    let normal = profile.frame.normal();
    let axis = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("three axes");
    let height = displacement[axis] / normal[axis];
    let residual: [f64; 3] = std::array::from_fn(|i| displacement[i] - normal[i] * height);
    let length = |p: [f64; 3]| p[0].hypot(p[1]).hypot(p[2]);
    let roundoff = 64. * f64::EPSILON * length(displacement).max(1.);
    if !height.is_finite() || length(residual) > roundoff.min(policy.linear_tolerance_mm / 64.) {
        return Err(error(
            "vector extrusion",
            GeometryErrorKind::Unsupported,
            "Curved extrusion requires a certified normal displacement; oblique curved sweeps are unsupported",
        ));
    }
    self::normal(profile, height, feature, origins, policy)
}

#[cfg(test)]
mod tests;
