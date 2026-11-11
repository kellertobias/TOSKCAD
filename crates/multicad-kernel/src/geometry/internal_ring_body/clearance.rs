use super::super::spur_gear_body::{Support, coaxial_placements};
use super::*;
use crate::rust_kernel::spur_gear::clearance::{self, ResultKind};

const PAIR: &str = "internal pair interference";

impl Solid {
    /// Prove positive clearance or a strict common-interior point for this
    /// placed Rust internal ring and a placed external Rust spur pinion with
    /// a shared axial direction.
    pub fn certify_rust_internal_pair(
        &self,
        pinion: &Self,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ResultKind, GeometryError> {
        policy.validate()?;
        let unsupported = |message| error(PAIR, GeometryErrorKind::Unsupported, message);
        if self.policy != policy || pinion.policy != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(unsupported(
                "Rust internal pair interference requires a shared zero-fuzz policy",
            ));
        }
        let (Some((ring, rf)), Some((gear, gf))) =
            (self.internal_ring_body(), pinion.spur_gear_body())
        else {
            return Err(unsupported(
                "Rust internal pair interference requires an actual internal ring and an external spur pinion",
            ));
        };
        if gear.profile.hub().is_some() {
            return Err(unsupported(
                "Rust internal pair interference requires a pinion without a hub",
            ));
        }
        let placements = coaxial_placements(
            "Rust internal pair interference",
            policy,
            [rf, gf],
            [
                (
                    ring.ring.height_mm(),
                    Box::new(|direction| ring.ring.support(direction)) as Support<'_>,
                ),
                (
                    gear.profile.height_mm(),
                    Box::new(|direction| gear.profile.support(direction)),
                ),
            ],
        )?;
        clearance::certify_internal(
            &ring.ring,
            &gear.profile,
            placements,
            policy.linear_tolerance_mm,
            interrupt,
        )
        .map_err(|cause| {
            let kind = match cause {
                clearance::Error::Invalid => GeometryErrorKind::Unsupported,
                clearance::Error::Limit => GeometryErrorKind::Limit,
                clearance::Error::Cancelled => GeometryErrorKind::Cancelled,
            };
            error(
                PAIR,
                kind,
                format!("Rust internal pair interference certificate: {cause:?}"),
            )
        })
    }
}
