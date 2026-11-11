//! Display-only shaft envelope; manufacturing admission stays document-driven.
use super::*;
use multicad_kernel::rust_kernel::{Circle, Rational, cylinder::AxialCylinder};

pub(super) fn shaft(
    feature: EntityId,
    thread: &ResolvedThread,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    if thread.profile.radial_taper != 0. {
        return Err(error(
            Some(feature),
            "Rust cosmetic shafts currently require an untapered envelope",
        ));
    }
    let exact = |value: f64| {
        Rational::parse_decimal(&value.to_string())
            .map_err(|cause| error(Some(feature), format!("{cause:?}")))
    };
    let zero = Rational::new(0, 1).expect("zero is exact");
    let circle = Circle::new([zero; 2], exact(thread.major_radius_mm)?)
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
