//! Filled-cylinder pairs with actual-chart placement error bounds.
use super::*;
use crate::geometry::arithmetic::{difference_with_error, dot_error};
use multicad_rust_kernel::cylinder::parallel_cylinder;

fn cardinal(frame: PlaneFrame) -> bool {
    [frame.u, frame.v, frame.normal()].into_iter().all(|axis| {
        axis.into_iter().filter(|x| x.abs() == 1.).count() == 1
            && axis.into_iter().all(|x| x == 0. || x.abs() == 1.)
    })
}
fn world(frame: PlaneFrame, point: Point, budget: f64) -> Result<Point, GeometryError> {
    let mut result = [0.; 3];
    let mut error = 0.;
    for (axis, value) in result.iter_mut().enumerate() {
        let (rotation, roundoff) =
            dot_error([frame.u[axis], frame.v[axis], frame.normal()[axis]], point);
        *value = frame.origin[axis] + rotation;
        let split = *value - frame.origin[axis];
        error = (error + roundoff).next_up();
        error = (error
            + ((frame.origin[axis] - (*value - split)) + (rotation - split))
                .abs()
                .next_up())
        .next_up();
    }
    if !error.is_finite() || error > budget {
        return Err(failure("World witness arithmetic is unresolved"));
    }
    Ok(result)
}
// A cylinder's outer dilation minus inner erosion bounds material changed by
// a Hausdorff displacement. Policy admission gives error < min(R,H)/2.
fn tube(radius: f64, height: f64, error: f64) -> f64 {
    let rh = (radius * height).next_up();
    let rr = (radius * radius).next_up();
    let ee = (error * error).next_up();
    let sum = ((rh + rr).next_up() + ee).next_up();
    (((4. * std::f64::consts::PI.next_up()).next_up() * error).next_up() * sum).next_up()
}

impl Solid {
    pub(in crate::geometry) fn rust_parallel_filled_cylinders(
        &self,
        other: &Self,
    ) -> Option<Result<(MinimumDistance, f64), GeometryError>> {
        let (a, af) = self.axial_cylinder_frame()?;
        let (b, bf) = other.axial_cylinder_frame()?;
        Some((|| {
            let tolerance = self
                .policy
                .linear_tolerance_mm
                .min(other.policy.linear_tolerance_mm);
            if cross(af.normal(), bf.normal()) != [0.; 3] {
                return Err(failure("Cylinder axes are not parallel"));
            }
            let unit = |p: Point| {
                let length = norm(p);
                p.map(|v| v / length)
            };
            let n = unit(af.normal());
            let u = unit(af.u);
            let v = cross(n, u);
            let sign = if dot(bf.normal(), n) < 0. { -1. } else { 1. };
            let (relative, relative_error) = difference_with_error(bf.origin, af.origin);
            let exact = cardinal(af)
                && cardinal(bf)
                && relative_error == [0.; 3]
                && bf.normal() == af.normal().map(|v| sign * v);
            let radii = [
                a.circle().radius().as_f64().next_up(),
                b.circle().radius().as_f64().next_up(),
            ];
            let heights = [a.height().as_f64().next_up(), b.height().as_f64().next_up()];
            let mut projected = [0.; 3];
            let mut projection_error = 0.;
            for (i, axis) in [u, v, n].into_iter().enumerate() {
                if exact {
                    projected[i] = dot(relative, axis);
                } else {
                    let (value, error) = dot_error(relative, axis);
                    projected[i] = value;
                    projection_error =
                        (projection_error + error + dot(relative_error, axis.map(f64::abs)))
                            .next_up();
                }
            }
            let bu = unit(std::array::from_fn(|i| bf.u[i] - dot(bf.u, n) * n[i]));
            let bv = cross(n.map(|v| sign * v), bu);
            // All reconstruction operands are near-unit. The fixed 64-eps
            // reserve bounds unit/Gram arithmetic over the relative geometry;
            // exact signed-cardinal charts need no such idealization.
            let mut uncertainty = if exact {
                0.
            } else {
                (norm(difference(af.u, u)) + norm(difference(af.v, v))) * radii[0]
                    + norm(difference(af.normal(), n)) * heights[0]
                    + (norm(difference(bf.u, bu)) + norm(difference(bf.v, bv))) * radii[1]
                    + norm(difference(bf.normal(), n.map(|v| sign * v))) * heights[1]
                    + projection_error
                    + 64.
                        * f64::EPSILON
                        * (norm(relative)
                            + radii.into_iter().sum::<f64>()
                            + heights.into_iter().sum::<f64>())
            };
            uncertainty = if exact { 0. } else { uncertainty.next_up() };
            if !uncertainty.is_finite() || uncertainty > tolerance / 32. {
                return Err(failure(
                    "Parallel-cylinder placement cannot be proved at model precision",
                ));
            }
            let contact =
                parallel_cylinder::measure(a, b, projected, sign < 0.).map_err(|cause| {
                    error(
                        "parallel cylinder inspection",
                        match cause {
                            parallel_cylinder::Error::Invalid => GeometryErrorKind::Input,
                            parallel_cylinder::Error::Limit => GeometryErrorKind::Limit,
                            parallel_cylinder::Error::Unresolved => GeometryErrorKind::Unsupported,
                        },
                        format!("Finite cylinder contact is unresolved: {cause:?}"),
                    )
                })?;
            let distance_error = (contact.distance_mm - contact.distance_bounds_mm[0])
                .abs()
                .max((contact.distance_bounds_mm[1] - contact.distance_mm).abs());
            if distance_error + uncertainty > tolerance / 8.
                || contact.witness_error_mm + uncertainty > tolerance / 8.
            {
                return Err(failure(
                    "Finite cylinder query exceeds its numerical budget",
                ));
            }
            if !exact {
                if contact.common_volume_mm3 > 0. {
                    let error = tube(radii[0], heights[0], uncertainty)
                        + tube(radii[1], heights[1], uncertainty);
                    if contact.common_volume_bounds_mm3[0] <= error {
                        return Err(failure(
                            "Positive cylinder overlap is below the placement proof budget",
                        ));
                    }
                } else if contact.distance_bounds_mm[0] <= uncertainty {
                    return Err(failure(
                        "Cylinder contact may contain unresolved positive material",
                    ));
                }
            }
            let frame = PlaneFrame {
                origin: af.origin,
                u,
                v,
            };
            let witness = DistanceWitness {
                a_mm: world(frame, contact.witness[0], tolerance / 16.)?,
                b_mm: world(frame, contact.witness[1], tolerance / 16.)?,
            };
            if (norm(difference(witness.a_mm, witness.b_mm)) - contact.distance_mm).abs()
                > tolerance / 4.
            {
                return Err(failure(
                    "Finite cylinder witness does not reproduce its distance",
                ));
            }
            let total = if exact { contact.total_solutions } else { None };
            Ok((
                MinimumDistance {
                    distance_mm: contact.distance_mm,
                    witnesses: vec![witness],
                    witnesses_complete: total.is_some(),
                    total_solutions: total,
                    inner_solution: contact.common_volume_mm3 > 0.,
                },
                contact.common_volume_mm3,
            ))
        })())
    }
}
