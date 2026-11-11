//! Analytic parallel cylinder/prism inspection with explicit placement bounds.
use super::*;
use crate::{DistanceWitness, MinimumDistance};
use multicad_rust_kernel::{cylinder::parallel_prism, linear_sweep::Sweep};

type Point = [f64; 3];
type Section = (Vec<Vec<[f64; 2]>>, f64, PlaneFrame, bool);
fn norm(p: Point) -> f64 {
    p[0].hypot(p[1]).hypot(p[2])
}
fn difference(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn section(solid: &Solid) -> Option<Section> {
    if let Some((prism, frame)) = solid.rust_prism_frame() {
        if prism.rounded_end().is_some() || prism.inner_loops().len() != 0 {
            return None;
        }
        let [w, d, h] = prism.dimensions().map(Rational::as_f64);
        return Some((
            vec![vec![[0., 0.], [w, 0.], [w, d], [0., d]]],
            h,
            frame,
            false,
        ));
    }
    if let Some(p) = solid.polygon_prism() {
        if !p.prism.circular_holes().is_empty()
            || !p.pockets.is_empty()
            || p.bore_treatments.is_some()
        {
            return None;
        }
        return Some((
            p.prism
                .rings()
                .iter()
                .map(|ring| ring.iter().map(|p| p.map(Rational::as_f64)).collect())
                .collect(),
            p.prism.height().as_f64(),
            p.frame,
            p.reverse_sweep,
        ));
    }
    let (body, frame) = solid.linear_body()?;
    let (rings, height) = body.straight_section()?;
    Some((rings.to_vec(), height, frame, false))
}
fn failure(message: &str) -> GeometryError {
    error(
        "parallel cylinder inspection",
        GeometryErrorKind::Unsupported,
        message,
    )
}
impl Solid {
    pub(crate) fn rust_parallel_cylinder_pair(
        &self,
        other: &Self,
    ) -> Option<Result<(MinimumDistance, f64), GeometryError>> {
        if !self.uses_rust_geometry() || !other.uses_rust_geometry() {
            return None;
        }
        let (cylinder, cframe, profile, reverse) = if let Some((c, f)) = self.axial_cylinder_frame()
        {
            (c, f, section(other)?, false)
        } else {
            let (c, f) = other.axial_cylinder_frame()?;
            (c, f, section(self)?, true)
        };
        Some((|| {
            let tolerance = self
                .policy
                .linear_tolerance_mm
                .min(other.policy.linear_tolerance_mm);
            let unit = |p: Point| {
                let n = norm(p);
                p.map(|v| v / n)
            };
            let u = unit(cframe.u);
            let n = unit(cframe.normal());
            let v = cross(n, u);
            let (mut rings, height, pframe, reversed) = profile;
            let relative = difference(pframe.origin, cframe.origin);
            let radius = cylinder.circle().radius().as_f64();
            let ch = cylinder.height().as_f64();
            let length = rings
                .iter()
                .flatten()
                .map(|p| p[0].hypot(p[1]))
                .fold(radius.max(ch).max(height).max(1.), f64::max);
            let magnitude = pframe
                .origin
                .into_iter()
                .chain(cframe.origin)
                .map(f64::abs)
                .fold(length, f64::max);
            let uncertainty = (norm(difference(cframe.u, u)) + norm(difference(cframe.v, v)))
                * radius
                + norm(difference(cframe.normal(), n)) * ch
                + norm(cross(pframe.normal(), n)) * height
                + (dot(pframe.u, n).abs() + dot(pframe.v, n).abs()) * length
                + 256. * f64::EPSILON * magnitude;
            if !uncertainty.is_finite() || uncertainty > tolerance / 32. {
                return Err(failure(
                    "Parallel placement cannot be certified within model tolerance",
                ));
            }
            let sense = dot(pframe.normal(), n);
            for ring in &mut rings {
                for point in ring.iter_mut() {
                    let world = std::array::from_fn(|i| {
                        relative[i] + pframe.u[i] * point[0] + pframe.v[i] * point[1]
                    });
                    *point = [dot(world, u), dot(world, v)];
                }
                if sense < 0. {
                    ring.reverse();
                }
            }
            let z = dot(relative, n);
            let end = z + sense * height * if reversed { -1. } else { 1. };
            let vertex_count = rings.iter().map(Vec::len).sum::<usize>();
            let sweep = Sweep::new(rings.clone(), rings, (end - z).abs()).map_err(|cause| {
                error(
                    "parallel cylinder inspection",
                    match cause {
                        multicad_rust_kernel::linear_sweep::Error::Limit => {
                            GeometryErrorKind::Limit
                        }
                        _ => GeometryErrorKind::Unsupported,
                    },
                    "Projected section cannot be certified",
                )
            })?;
            let contact = parallel_prism::measure(cylinder, &sweep, z.min(end)).map_err(|_| {
                failure("Finite circular contact or overlap is numerically unresolved")
            })?;
            if contact.distance_error_mm > tolerance / 8. {
                return Err(failure(
                    "Finite contact exceeds the numerical distance budget",
                ));
            }
            // Bound both cap areas and every side rectangle, including holes;
            // the placement allowance must grow with boundary complexity.
            let volume_error = contact.common_volume_error_mm3
                + (8. * vertex_count as f64 + 32.) * length * length * uncertainty;
            if (contact.common_volume_mm3 > 0. && contact.common_volume_mm3 <= volume_error)
                || (contact.common_volume_mm3 == 0. && contact.distance_mm <= uncertainty)
            {
                return Err(failure("Near-contact occupied volume cannot be certified"));
            }
            let world = |point: Point| {
                std::array::from_fn(|i| {
                    cframe.origin[i] + u[i] * point[0] + v[i] * point[1] + n[i] * point[2]
                })
            };
            let witnesses = contact
                .witnesses
                .iter()
                .map(|&[a, b]| {
                    let a = world(a);
                    let b = world(b);
                    let distance = norm(difference(a, b));
                    if !distance.is_finite()
                        || (distance - contact.distance_mm).abs() > tolerance / 4.
                    {
                        return Err(failure("World clearance witness exceeds model tolerance"));
                    }
                    Ok(if reverse {
                        DistanceWitness { a_mm: b, b_mm: a }
                    } else {
                        DistanceWitness { a_mm: a, b_mm: b }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                MinimumDistance {
                    distance_mm: contact.distance_mm,
                    witnesses_complete: witnesses.len() == contact.total_solutions as usize,
                    total_solutions: Some(contact.total_solutions),
                    witnesses,
                    inner_solution: contact.common_volume_mm3 > 0.,
                },
                contact.common_volume_mm3,
            ))
        })())
    }
}
