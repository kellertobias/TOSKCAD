//! Placed axial proof dispatch for continuous helical and herringbone pairs.
use super::*;
use multicad_rust_kernel::spur_gear::clearance::{self, Placement, ResultKind};

impl Solid {
    /// Prove the complete fixed-pose clearance of aligned helical or
    /// herringbone solids. Tilted pair charts require an additional proof.
    pub fn certify_rust_herringbone_pair(
        &self,
        other: &Self,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ResultKind, GeometryError> {
        const OPERATION: &str = "twisted gear pair interference";
        let unsupported = |message| error(OPERATION, GeometryErrorKind::Unsupported, message);
        policy.validate()?;
        if self.policy != policy || other.policy != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(unsupported(
                "Rust twisted gear interference requires a shared zero-fuzz policy",
            ));
        }
        let [Some((a, af)), Some((b, bf))] = [self.herringbone_body(), other.herringbone_body()]
        else {
            return Err(unsupported(
                "Rust twisted gear interference requires two continuous helical or herringbone solids",
            ));
        };
        let axis = af
            .normal()
            .iter()
            .position(|n| n.abs() > 0.99)
            .ok_or_else(|| {
                unsupported("Rust herringbone interference requires a common world axis")
            })?;
        let sense = af.normal()[axis].signum();
        if (af.origin[axis] - bf.origin[axis]).abs() > policy.linear_tolerance_mm
            || [af, bf].iter().any(|frame| {
                frame.u[axis].abs() > 1e-12
                    || frame.v[axis].abs() > 1e-12
                    || (frame.normal()[axis] - sense).abs() > 1e-12
            })
        {
            return Err(unsupported(
                "Rust herringbone interference requires aligned starts and codirected world axes",
            ));
        }

        let delta = std::array::from_fn(|i| bf.origin[i] - af.origin[i]);
        let separation = dot(delta, delta).sqrt();
        if !separation.is_finite() || separation <= policy.linear_tolerance_mm {
            return Err(unsupported(
                "Rust herringbone interference requires distinct parallel axes",
            ));
        }
        let x = delta.map(|x| x / separation);
        let normal = std::array::from_fn(|i| if i == axis { sense } else { 0. });
        let y = cross(normal, x);
        let project = |point| [dot(point, x), dot(point, y)];
        let mut placements = [Placement::default(); 2];
        for (i, (body, frame)) in [(a, af), (b, bf)].into_iter().enumerate() {
            let [u, v] = [project(frame.u), project(frame.v)];
            let magnitude = u[0].hypot(u[1]);
            let un = u.map(|c| c / magnitude);
            let vn = [-un[1], un[0]];
            let world_scale = frame.origin.into_iter().map(f64::abs).fold(
                separation.max(body.geometry.tip_radius_mm()).max(1.),
                f64::max,
            );
            let rotation_error =
                (u[0] - un[0]).hypot(u[1] - un[1]) + (v[0] - vn[0]).hypot(v[1] - vn[1]);
            let height = body.geometry.profile().height_mm();
            let radius = body.geometry.tip_radius_mm();
            // Bound the local height change needed to meet the common
            // world section, including roundoff from composed placements.
            let normal = frame.normal();
            let axial_error = ((frame.origin[axis] - af.origin[axis]).abs()
                + radius * (frame.u[axis].abs() + frame.v[axis].abs())
                + height * (normal[axis] - sense).abs())
                / normal[axis].abs();
            let phase_error = axial_error * body.geometry.twist_rad_per_mm().abs();
            let transverse_drift = height * dot(normal, x).hypot(dot(normal, y));
            let uncertainty_mm = radius * (rotation_error + phase_error)
                + transverse_drift
                + 1024. * f64::EPSILON * world_scale;
            if uncertainty_mm > policy.linear_tolerance_mm {
                return Err(unsupported(
                    "Rust herringbone interference cannot enclose this axial placement within policy",
                ));
            }
            placements[i] = Placement {
                center_mm: [i as f64 * separation, 0.],
                axes: [un, vn],
                uncertainty_mm,
            };
        }
        clearance::certify_herringbone(
            [&a.geometry, &b.geometry],
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
                OPERATION,
                kind,
                format!("Rust herringbone interference certificate: {cause:?}"),
            )
        })
    }
}
