//! Sixfold tangent-circle engineering contours; not an ISO gauge certificate.
use super::invalid;
use crate::document::DocumentError;
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HexalobularArc {
    pub start: [f64; 2],
    pub mid: [f64; 2],
    pub end: [f64; 2],
    pub center: [f64; 2],
    pub radius: f64,
    /// Convex arcs run counterclockwise; concave arcs run clockwise.
    pub convex: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HexalobularContour {
    pub arcs: [HexalobularArc; 12],
    pub inner_valley_radius_mm: f64,
}
impl HexalobularContour {
    /// Construct exact circular descriptors from independent A, B and re inputs.
    /// The derived ri enforces external tangency. Radius ratios are engineering
    /// choices, not normative recess/driver/GO-gauge equivalence.
    pub fn new(a: f64, b_diameter: f64, re: f64) -> Result<Self, DocumentError> {
        let fail = || {
            invalid(
                "Hexalobular contour needs finite positive A > B, valid tangent radii and a simple angularly monotone boundary",
            )
        };
        if ![a, b_diameter, re].iter().all(|v| v.is_finite() && *v > 0.)
            || a <= b_diameter
            || re >= a / 2.
        {
            return Err(fail());
        }
        // Normalize first to avoid scale-dependent overflow and cancellation.
        let b = b_diameter / a / 2.;
        let c = 0.5 - re / a;
        let r = re / a;
        let cos30 = 3_f64.sqrt() / 2.;
        let denominator = 2. * (b - c * cos30 - r);
        if !denominator.is_finite() || denominator == 0. {
            return Err(fail());
        }
        let ri = (r * r - b * b - c * c + 2. * b * c * cos30) / denominator;
        let d = b + ri;
        if !ri.is_finite() || ri <= 0. || !d.is_finite() {
            return Err(fail());
        }
        let sum = r + ri;
        let contact = [c + (d * cos30 - c) * r / sum, d * 0.5 * r / sum];
        let theta = contact[1].atan2(contact[0]);
        let outer_half = contact[1].atan2(contact[0] - c);
        let inward = [-cos30, -0.5];
        let inner_direction = [(contact[0] - d * cos30) / ri, (contact[1] - d * 0.5) / ri];
        let inner_cos = inward[0] * inner_direction[0] + inward[1] * inner_direction[1];
        // These derivative bounds hold throughout each symmetric circular arc.
        // They make polar angle strictly increasing, partitioning the whole
        // boundary into disjoint sectors: no nonadjacent arc can intersect.
        if !(theta > 0.
            && theta < PI / 6.
            && outer_half > 0.
            && outer_half < PI
            && c * outer_half.cos() + r > 0.
            && inner_cos > 0.
            && d * inner_cos - ri > 0.)
        {
            return Err(fail());
        }
        let rotate = |p: [f64; 2], angle: f64| {
            let (s, c) = angle.sin_cos();
            [a * (c * p[0] - s * p[1]), a * (s * p[0] + c * p[1])]
        };
        let outer_centers: [[f64; 2]; 6] =
            std::array::from_fn(|i| rotate([c, 0.], i as f64 * TAU / 6.));
        let inner_centers: [[f64; 2]; 6] =
            std::array::from_fn(|i| rotate([d * cos30, d * 0.5], i as f64 * TAU / 6.));
        // Each shared endpoint is calculated ONCE and copied to both arcs.
        let contacts: [[f64; 2]; 12] = std::array::from_fn(|i| {
            let tooth = i / 2;
            if i % 2 == 0 {
                rotate(contact, tooth as f64 * TAU / 6.)
            } else {
                rotate(
                    [contact[0], -contact[1]],
                    ((tooth + 1) % 6) as f64 * TAU / 6.,
                )
            }
        });
        let arcs = std::array::from_fn(|i| {
            let tooth = i / 2;
            let convex = i % 2 == 0;
            HexalobularArc {
                start: contacts[(i + 11) % 12],
                end: contacts[i],
                mid: if convex {
                    rotate([0.5, 0.], tooth as f64 * TAU / 6.)
                } else {
                    rotate([b * cos30, b * 0.5], tooth as f64 * TAU / 6.)
                },
                center: if convex {
                    outer_centers[tooth]
                } else {
                    inner_centers[tooth]
                },
                radius: if convex { re } else { ri * a },
                convex,
            }
        });
        if arcs.iter().any(|arc| {
            !arc.radius.is_finite()
                || arc.radius <= 0.
                || arc
                    .start
                    .iter()
                    .chain(arc.mid.iter())
                    .chain(arc.end.iter())
                    .chain(arc.center.iter())
                    .any(|v| !v.is_finite())
        }) {
            return Err(fail());
        }
        Ok(Self {
            arcs,
            inner_valley_radius_mm: ri * a,
        })
    }
}
