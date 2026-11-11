use super::*;
use crate::revolution::{dot, length};
use std::f64::consts::FRAC_PI_2;

fn angle([x, y]: Point) -> f64 {
    y.atan2(x)
}

/// At most one equivalent phase lies in any admitted interval.
fn in_interval(angle: f64, from: f64, to: f64) -> Option<f64> {
    let lo = from.min(to);
    let hi = from.max(to);
    let wrapped = angle + TAU * ((lo - angle) / TAU).ceil();
    (wrapped <= hi).then_some(wrapped)
}

fn roots(sine: f64) -> impl Iterator<Item = f64> {
    let first = sine.asin();
    [first, PI - first].into_iter().filter(|a| a.is_finite())
}

impl Herringbone {
    /// Exact support in the supplied local direction. Bounds of a placed
    /// gear query this function with the direction pulled into its frame.
    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.iter().any(|x| !x.is_finite()) {
            return f64::NAN;
        }
        let magnitude = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if magnitude == 0. {
            return 0.;
        }
        let [dx, dy, dz] = direction.map(|x| x / magnitude);
        let radial = [dx, dy];
        let q = length(radial);
        let height = self.profile.height_mm();
        if q == 0. {
            return (height * dz).max(0.) * magnitude;
        }
        let azimuth = angle(radial);
        let mid = height * 0.5;
        let mut best = f64::NEG_INFINITY;
        for z in [0., mid, height] {
            let phase = self.phase_at(z);
            let projected = rotate(radial, -phase);
            for item in self.profile.outline() {
                best = best.max(item.edge.support(projected) + dz * z);
            }
        }
        for band in [Band::Lower, Band::Upper] {
            let [low, high] = band.axial_interval_mm(height);
            let phase = self.phase_at(low);
            let high_phase = self.phase_at(high);
            let slope = self.band_twist_rad_per_mm(band);
            let mut check = |p: Point, a| {
                if let Some(a) = in_interval(a, phase, high_phase) {
                    let z = low + (a - phase) / slope;
                    best = best.max(dot(rotate(p, a), radial) + dz * z);
                }
            };
            for item in self.profile.outline() {
                let edge = item.edge;
                // A longitudinal rail is a circular helix. Solve its
                // projection derivative directly over the band's phase.
                let p = edge.point(0.);
                for root in roots(dz / (q * length(p) * slope)) {
                    check(p, azimuth - angle(p) + root);
                }
                // In the patch interior the radial projection is normal
                // to the edge. Its axial derivative fixes p·unit_t.
                for side in [-1., 1.] {
                    let tangent_dot = -dz / (side * q * slope);
                    for t in tangent_roots(edge, tangent_dot) {
                        check(
                            edge.point(t),
                            azimuth - angle(edge.tangent(t)) - side * FRAC_PI_2,
                        );
                    }
                }
            }
        }
        best * magnitude
    }
}

/// Isolated roots of p·unit_t = target. Centered arcs and involutes
/// have a constant value. Their stationary branch reaches a patch
/// boundary with the same support, covered by the cap and rail tests.
fn tangent_roots(edge: Edge, target: f64) -> impl Iterator<Item = f64> {
    let candidates = match edge {
        Edge::Planar(normal_sweep::Edge::Line { a, .. }) => [
            (target - dot(a, edge.tangent(0.))) / edge.length_mm(),
            f64::NAN,
        ],
        Edge::Planar(arc @ normal_sweep::Edge::Arc { center, a, .. }) if center != [0.; 2] => {
            let start = angle(super::super::sub(a, center));
            let sweep = arc.signed_sweep_rad().expect("circular arc");
            let ratio = target / (length(center) * sweep.signum());
            let first = ratio.asin();
            [first, PI - first].map(|root| {
                in_interval(angle(center) - root, start, start + sweep)
                    .map_or(f64::NAN, |a| ((a - start) / sweep).clamp(0., 1.))
            })
        }
        _ => [f64::NAN; 2],
    };
    candidates.into_iter().filter(|t| (0. ..=1.).contains(t))
}
