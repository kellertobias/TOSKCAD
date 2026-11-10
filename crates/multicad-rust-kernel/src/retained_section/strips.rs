use super::heights::{Height, difference, sort_unique};
use super::{AxialProfile, Body, Point, Profile, Result, Span, bounded, intersect};
use crate::planar_projection::ProjectionError;

#[derive(Clone, Copy)]
pub(super) struct Strip {
    pub(super) body: usize,
    pub(super) u: Span,
    pub(super) v: Span,
    pub(super) depth: [Height; 2],
    pub(super) removed: bool,
}

pub(super) struct Collector<'a> {
    strips: &'a mut Vec<Strip>,
    body: usize,
    window: [Span; 2],
    retained: Span,
}

impl<'a> Collector<'a> {
    pub(super) fn new(
        strips: &'a mut Vec<Strip>,
        body: usize,
        window: [Span; 2],
        retained: Span,
    ) -> Self {
        Self {
            strips,
            body,
            window,
            retained,
        }
    }

    pub(super) fn add(&mut self, body: &Body) -> Result<()> {
        for stock in &body.stock {
            let [u, v, depth] = std::array::from_fn(|i| [stock.min[i], stock.max[i]]);
            if [u, v, depth]
                .into_iter()
                .any(|[a, b]| !bounded(a) || !bounded(b) || a >= b)
            {
                return Err(ProjectionError::Invalid);
            }
            self.strip(u, v, depth, depth.map(Height::flat), false);
        }
        for (profiles, removed) in [(&body.positive, false), (&body.cuts, true)] {
            for profile in profiles {
                self.axial(profile, removed)?;
            }
        }
        Ok(())
    }

    fn axial(&mut self, profile: &AxialProfile, removed: bool) -> Result<()> {
        let [lo, hi] = profile.span;
        if !bounded(lo) || !bounded(hi) || lo >= hi {
            return Err(ProjectionError::Invalid);
        }
        match profile.profile {
            Profile::Circle { center, radius } => self.disk(profile.span, center, radius, removed),
            Profile::Stadium(stadium) => {
                for center in stadium.centers() {
                    self.disk(profile.span, center, stadium.radius_mm(), removed)?;
                }
                self.strip_between_disks(profile.span, stadium.corners(), removed)
            }
        }
    }

    fn disk(&mut self, u: Span, center: Point, radius: f64, removed: bool) -> Result<()> {
        let [v, depth] = center.map(|coordinate| [coordinate - radius, coordinate + radius]);
        if !radius.is_finite() || radius <= 0. || ![v, depth].into_iter().flatten().all(bounded) {
            return Err(ProjectionError::Invalid);
        }
        let edges = [-1., 1.].map(|sign| Height::Arc {
            center,
            radius,
            sign,
        });
        self.strip(u, v, depth, edges, removed);
        Ok(())
    }

    fn strip_between_disks(&mut self, u: Span, corners: [Point; 4], removed: bool) -> Result<()> {
        // The straight central rectangle plus the end disks is the stadium.
        // Split the rectangle at its vertices so each strip has two supports.
        let mut events = corners.map(|point| point[0]).to_vec();
        sort_unique(&mut events);
        let lines: Vec<_> = corners
            .into_iter()
            .zip(corners.into_iter().cycle().skip(1))
            .filter_map(|(a, b)| {
                let (lo, hi) = if a[0] < b[0] { (a, b) } else { (b, a) };
                let [dv, dz] = difference(hi, lo);
                if dv == 0. {
                    return None;
                }
                let length = dv.hypot(dz);
                Some((
                    [lo[0], hi[0]],
                    Height::Line {
                        origin: lo,
                        tangent: [dv / length, dz / length],
                    },
                ))
            })
            .collect();
        for pair in events.windows(2) {
            let v = [pair[0], pair[1]];
            let center = v[0].midpoint(v[1]);
            let mut edges = lines
                .iter()
                .filter(|(span, _)| span[0] < center && center < span[1])
                .map(|(_, line)| *line);
            let (Some(a), Some(b), None) = (edges.next(), edges.next(), edges.next()) else {
                return Err(ProjectionError::Invalid);
            };
            let depth = if a.at(center) < b.at(center) {
                [a, b]
            } else {
                [b, a]
            };
            let bounds = [
                depth[0].at(v[0]).min(depth[0].at(v[1])),
                depth[1].at(v[0]).max(depth[1].at(v[1])),
            ];
            self.strip(u, v, bounds, depth, removed);
        }
        Ok(())
    }

    fn strip(&mut self, u: Span, v: Span, bounds: Span, depth: [Height; 2], removed: bool) {
        if let Some(u) = intersect(u, self.window[0])
            && let Some(v) = intersect(v, self.window[1])
            && intersect(bounds, self.retained).is_some()
        {
            self.strips.push(Strip {
                body: self.body,
                u,
                v,
                depth,
                removed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_must_have_two_resolvable_depth_supports() {
        let mut strips = Vec::new();
        let mut collector = Collector::new(&mut strips, 0, [[0., 8.]; 2], [0., 8.]);
        let corners = [[0., 0.], [1., 0.], [f64::NAN, 1.], [f64::NAN, 1.]];
        assert_eq!(
            collector.strip_between_disks([0., 8.], corners, false),
            Err(ProjectionError::Invalid)
        );
    }
}
