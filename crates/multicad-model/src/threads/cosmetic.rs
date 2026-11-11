//! Display-only shaft envelope; manufacturing admission stays document-driven.
use super::*;
use multicad_kernel::rust_kernel::{Circle, Rational, cylinder::AxialCylinder};

pub(super) fn shaft(
    feature: EntityId,
    thread: &ResolvedThread,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let taper = thread.profile.radial_taper;
    let end_radius = taper.mul_add(thread.length_mm, thread.major_radius_mm);
    let limit = thread
        .npt
        .as_ref()
        .and_then(|npt| npt.stock_crest_radius_limit_mm);
    let clipped = |radius: f64| limit.map_or(radius, |maximum| radius.min(maximum));
    let radius = clipped(thread.major_radius_mm);
    let last = clipped(end_radius);
    if taper == 0. {
        return cylinder(feature, radius, thread, policy);
    }
    let admitted = |value: f64| {
        value.is_finite()
            && value >= policy.minimum_feature_mm
            && value <= policy.maximum_coordinate_mm
    };
    if [thread.major_radius_mm, end_radius, radius, last]
        .into_iter()
        .any(|r| !admitted(r))
    {
        return Err(error(
            Some(feature),
            "Cosmetic taper dimensions exceed the active geometry policy",
        ));
    }
    if end_radius == thread.major_radius_mm {
        return Err(error(
            Some(feature),
            "Nonzero cosmetic taper is below representable radius precision",
        ));
    }
    if radius == last {
        return cylinder(feature, radius, thread, policy);
    }
    let mut outside = vec![[radius, 0.]];
    if let Some(limit) = limit {
        let z = (limit - thread.major_radius_mm) / taper;
        if z > 0. && z < thread.length_mm {
            outside.push([limit, z]);
        }
    }
    outside.push([last, thread.length_mm]);
    let mut points = vec![[0., 0.]];
    points.extend(&outside);
    points.push([0., thread.length_mm]);
    let edges = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| CurvedEdge2d::Line { a, b })
        .collect::<Vec<_>>();
    let mut roles = vec!["entry"];
    roles.extend(outside.windows(2).map(|ends| {
        if ends[0][0] == ends[1][0] {
            "stock_limit"
        } else {
            "taper"
        }
    }));
    roles.extend(["exit", "axis"]);
    let origins = roles
        .into_iter()
        .map(|role| Origin {
            source: feature,
            role: format!("thread.cosmetic.envelope/{role}"),
        })
        .collect::<Vec<_>>();
    let local = Solid::rust_revolve_curved_profile_named(
        &CurvedProfile {
            frame: PlaneFrame {
                v: [0., 0., 1.],
                ..Default::default()
            },
            outer: edges,
            holes: vec![],
        },
        multicad_kernel::RevolutionAxis {
            origin_mm: [0.; 3],
            direction: [0., 0., 1.],
        },
        std::f64::consts::TAU,
        feature,
        &origins,
        policy,
    )
    .map_err(|cause| error(Some(feature), cause))?;
    local
        .transformed(RigidTransform {
            rotation: std::array::from_fn(|i| {
                [thread.frame.u[i], thread.frame.v[i], thread.frame.n[i]]
            }),
            translation_mm: thread.frame.origin,
        })
        .map_err(|cause| error(Some(feature), cause))
}

fn cylinder(
    feature: EntityId,
    radius: f64,
    thread: &ResolvedThread,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let exact = |value: f64| {
        Rational::parse_decimal(&value.to_string())
            .map_err(|cause| error(Some(feature), format!("{cause:?}")))
    };
    let zero = Rational::new(0, 1).expect("zero is exact");
    let circle = Circle::new([zero; 2], exact(radius)?)
        .map_err(|cause| error(Some(feature), format!("{cause:?}")))?;
    let geometry = AxialCylinder::new(feature.uuid_bits(), circle, zero, exact(thread.length_mm)?)
        .map_err(|cause| error(Some(feature), format!("{cause:?}")))?;
    Solid::from_axial_cylinder_sweep(
        geometry,
        PlaneFrame {
            origin: thread.frame.origin,
            u: thread.frame.u,
            v: thread.frame.v,
        },
        [0.; 2],
        false,
        Origin {
            source: feature,
            role: "thread.cosmetic.envelope".into(),
        },
        policy,
    )
    .map_err(|cause| error(Some(feature), cause))
}
