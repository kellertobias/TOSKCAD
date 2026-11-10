use super::*;
use crate::rust_kernel::spur_gear::clearance::{self, Placement, ResultKind};

const OPERATION: &str = "spur pair interference";

impl Solid {
    /// Prove separation or a strict common-interior point for two actual
    /// placed Rust spur bands with a shared axial direction.
    pub fn certify_rust_spur_pair(
        &self,
        other: &Self,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ResultKind, GeometryError> {
        policy.validate()?;
        let unsupported = |message| error(OPERATION, GeometryErrorKind::Unsupported, message);
        if self.policy != policy || other.policy != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(unsupported(
                "Rust spur interference requires a shared zero-fuzz policy",
            ));
        }
        let Some((a, af)) = self.spur_gear_body() else {
            return Err(unsupported(
                "Rust spur interference requires actual external spur solids",
            ));
        };
        let Some((b, bf)) = other.spur_gear_body() else {
            return Err(unsupported(
                "Rust spur interference requires actual external spur solids",
            ));
        };
        let profiles = [&a.profile, &b.profile];
        if profiles.iter().any(|profile| profile.hub().is_some()) {
            return Err(unsupported(
                "Rust spur interference currently requires simple tooth bands without hubs",
            ));
        }
        let normal = af.normal();
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
        let project = |v| [dot(v, af.u), dot(v, af.v)];
        let mut shared = [f64::NEG_INFINITY, f64::INFINITY];
        let mut placements = [Placement::default(); 2];
        for (i, (profile, frame)) in profiles.into_iter().zip([af, bf]).enumerate() {
            let rise = frame.normal();
            let vertical = dot(rise, normal);
            if (vertical - 1.).abs() > 1e-12 {
                return Err(unsupported(
                    "Rust spur interference requires parallel, codirected axes",
                ));
            }
            let delta = std::array::from_fn(|axis| frame.origin[axis] - af.origin[axis]);
            let z = dot(delta, normal);
            let depth = [dot(frame.u, normal), dot(frame.v, normal), 0.];
            let height = profile.height_mm();
            let world_scale = frame
                .origin
                .into_iter()
                .chain(af.origin)
                .map(f64::abs)
                .fold(height.max(1.), f64::max);
            let roundoff = 1024. * f64::EPSILON * world_scale;
            let [dx, dy] = project(rise);
            let uncertainty_mm = dx.hypot(dy) * height + roundoff;
            if uncertainty_mm > policy.linear_tolerance_mm {
                return Err(unsupported(
                    "Rust spur interference cannot enclose this axial placement within policy",
                ));
            }
            // Every planar point has a valid height on this inner slab.
            // Bounding the projected drift makes its strict material witness
            // valid even when represented frames differ by roundoff.
            shared[0] = shared[0].max(z + profile.support(depth) + roundoff);
            shared[1] = shared[1]
                .min(z + height * vertical - profile.support(depth.map(|v| -v)) - roundoff);
            placements[i] = Placement {
                center_mm: project(delta),
                axes: [project(frame.u), project(frame.v)],
                uncertainty_mm,
            };
        }
        if shared[1] - shared[0] <= policy.linear_tolerance_mm {
            return Err(unsupported(
                "Rust spur interference requires a positive common axial band",
            ));
        }
        clearance::certify(profiles, placements, policy.linear_tolerance_mm, interrupt).map_err(
            |cause| {
                let kind = match cause {
                    clearance::Error::Invalid => GeometryErrorKind::Unsupported,
                    clearance::Error::Limit => GeometryErrorKind::Limit,
                    clearance::Error::Cancelled => GeometryErrorKind::Cancelled,
                };
                error(
                    OPERATION,
                    kind,
                    format!("Rust spur interference certificate: {cause:?}"),
                )
            },
        )
    }
}
