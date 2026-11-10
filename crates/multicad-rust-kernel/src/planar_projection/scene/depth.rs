//! Exact view-depth clipping of spatial drawing candidates and face hits.
use super::*;

/// The retained view-depth interval. Positive depth points toward the viewer.
#[derive(Clone, Copy, Debug)]
pub struct DepthRange {
    pub(super) near: f64,
    pub(super) far: Option<f64>,
}

impl DepthRange {
    /// Keep depths at or behind `near_mm` and, if present, at or ahead of
    /// `far_mm`. Both finite limits use the view frame's depth coordinate.
    pub fn new(near_mm: f64, far_mm: Option<f64>) -> Result<Self, ProjectionError> {
        let bounded = |depth: f64| depth.is_finite() && depth.abs() <= 1e8;
        if !bounded(near_mm) || far_mm.is_some_and(|far| !bounded(far) || far >= near_mm) {
            return Err(ProjectionError::Invalid);
        }
        Ok(Self {
            near: near_mm,
            far: far_mm,
        })
    }

    /// Retain a straight candidate's positive-length portion in this range.
    pub fn clip_line(
        self,
        candidate: LineCandidate,
    ) -> Result<Option<LineCandidate>, ProjectionError> {
        candidate.spatial()?;
        let [a, b] = candidate.points;
        if self.contains(a[2]) && self.contains(b[2]) {
            return Ok(Some(candidate));
        }
        let [lo, hi] = [a[2].min(b[2]), a[2].max(b[2])];
        if self.near <= lo || self.far.is_some_and(|far| far >= hi) {
            return Ok(None);
        }
        let ends = [a, b];
        let points = std::array::from_fn(|i| {
            let [x, y, z] = ends[i];
            let depth = z.clamp(self.far.unwrap_or(f64::NEG_INFINITY), self.near);
            if depth == z {
                return ends[i];
            }
            let [u, v, w] = ends[1 - i];
            let t = (depth - z) / (w - z);
            [(u - x).mul_add(t, x), (v - y).mul_add(t, y), depth]
        });
        Ok(Some(LineCandidate {
            points,
            ..candidate
        }))
    }

    /// Clip a spatial circular edge by its exact harmonic depth. A cap may
    /// split the original trim into several intervals without changing its
    /// ellipse, parameter basis, or source.
    pub fn clip_rim(
        self,
        candidate: SpatialRimCandidate,
    ) -> Result<Vec<SpatialRimCandidate>, ProjectionError> {
        let rim = candidate.spatial()?;
        let depth = Harmonic([rim.depth, 0., rim.depth_sine]);
        if self.contains(rim.depth - rim.depth_sine.abs())
            && self.contains(rim.depth + rim.depth_sine.abs())
        {
            return Ok(vec![candidate]);
        }
        let trim = rim.ellipse.trim;
        let mut cuts = Vec::new();
        depth.level(self.near, trim, &mut cuts);
        if let Some(far) = self.far {
            depth.level(far, trim, &mut cuts);
        }
        let mut output = Vec::new();
        drawing_events::spans(
            &mut cuts,
            trim,
            |t| self.contains(depth.at(t)),
            |trim| {
                output.push(SpatialRimCandidate {
                    ellipse: crate::curved_projection::Ellipse {
                        trim,
                        ..candidate.ellipse
                    },
                    ..candidate
                })
            },
        );
        Ok(output)
    }

    fn contains(self, depth: f64) -> bool {
        depth <= self.near && self.far.is_none_or(|far| depth >= far)
    }
}

#[cfg(test)]
mod tests;
