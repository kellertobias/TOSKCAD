//! Exact view-volume clipping of spatial drawing candidates.
use super::*;

/// The retained view-depth interval with optional U and V candidate windows.
/// Positive depth points toward the viewer.
#[derive(Clone, Copy, Debug)]
pub struct DepthRange {
    pub(super) near: f64,
    pub(super) far: Option<f64>,
    lateral: [Option<Span>; 2],
}

impl DepthRange {
    /// Keep depths at or behind `near_mm` and, if present, at or ahead of
    /// `far_mm`. Both finite limits use the view frame's depth coordinate.
    pub fn new(near_mm: f64, far_mm: Option<f64>) -> Result<Self, ProjectionError> {
        if !bounded(near_mm) || far_mm.is_some_and(|far| !bounded(far) || far >= near_mm) {
            return Err(ProjectionError::Invalid);
        }
        Ok(Self {
            near: near_mm,
            far: far_mm,
            lateral: [None; 2],
        })
    }

    /// Limit candidates to these U and V windows before projecting visibility.
    /// Face blockers only use the depth interval because a view ray has
    /// the same U and V as the candidate all along its depth.
    pub fn with_lateral(
        mut self,
        windows_mm: [Option<[f64; 2]>; 2],
    ) -> Result<Self, ProjectionError> {
        if windows_mm
            .into_iter()
            .flatten()
            .any(|[lo, hi]| !bounded(lo) || !bounded(hi) || lo >= hi)
        {
            return Err(ProjectionError::Invalid);
        }
        self.lateral = windows_mm;
        Ok(self)
    }

    /// Retain a straight candidate's positive-length portion in this volume.
    pub fn clip_line(
        self,
        candidate: LineCandidate,
    ) -> Result<Option<LineCandidate>, ProjectionError> {
        candidate.spatial()?;
        let [a, b] = candidate.points;
        let bounds = self.bounds();
        let mut trim = [0., 1.];
        let mut snaps = [[None; 3]; 2];
        let delta = sub(b, a);
        for (axis, [lo, hi]) in bounds.into_iter().enumerate() {
            if lo == f64::NEG_INFINITY && hi == f64::INFINITY {
                continue;
            }
            let ends = [a[axis], b[axis]];
            let clipped = ends.map(|v| v.clamp(lo, hi));
            if ends == clipped {
                continue;
            }
            if hi <= ends[0].min(ends[1]) || lo >= ends[0].max(ends[1]) {
                return Ok(None);
            }
            for end in 0..2 {
                if ends[end] == clipped[end] {
                    continue;
                }
                let root = (clipped[end] - a[axis]) / delta[axis];
                if (end == 0 && root > trim[end]) || (end == 1 && root < trim[end]) {
                    trim[end] = root;
                    snaps[end] = [None; 3];
                }
                if root == trim[end] {
                    snaps[end][axis] = Some(clipped[end]);
                }
            }
        }
        if trim[0] >= trim[1] {
            return Ok(None);
        }
        let points = std::array::from_fn(|end| {
            let t = trim[end];
            let unchanged = t == end as f64;
            if unchanged && snaps[end] == [None; 3] {
                return candidate.points[end];
            }
            std::array::from_fn(|axis| {
                let [lo, hi] = bounds[axis];
                snaps[end][axis].unwrap_or_else(|| {
                    let value = if unchanged {
                        candidate.points[end][axis]
                    } else {
                        delta[axis].mul_add(t, a[axis])
                    };
                    value.clamp(lo, hi)
                })
            })
        });
        Ok(Some(LineCandidate {
            points,
            ..candidate
        }))
    }

    /// Clip a spatial circular edge by exact coordinate harmonics. The
    /// window can split a trim without changing its ellipse or source.
    pub fn clip_rim(
        self,
        candidate: SpatialRimCandidate,
    ) -> Result<Vec<SpatialRimCandidate>, ProjectionError> {
        Ok(
            self.clip_curve(candidate.spatial()?, candidate, |candidate, trim| {
                SpatialRimCandidate {
                    ellipse: crate::curved_projection::Ellipse {
                        trim,
                        ..candidate.ellipse
                    },
                    ..candidate
                }
            }),
        )
    }

    /// Clip an exact spatial ellipse by the retained depth and lateral bounds.
    pub fn clip_ellipse(
        self,
        candidate: SpatialEllipseCandidate,
    ) -> Result<Vec<SpatialEllipseCandidate>, ProjectionError> {
        Ok(
            self.clip_curve(candidate.spatial()?, candidate, |candidate, trim| {
                SpatialEllipseCandidate {
                    ellipse: crate::curved_projection::Ellipse {
                        trim,
                        ..candidate.ellipse
                    },
                    ..candidate
                }
            }),
        )
    }

    fn clip_curve<C: Copy>(
        self,
        rim: Rim,
        candidate: C,
        with_trim: impl Fn(C, Span) -> C,
    ) -> Vec<C> {
        let [origin, cosine, sine] = rim.basis();
        let trim = rim.ellipse.trim;
        let mut cuts = Vec::new();
        let mut active = [const { None }; 3];
        for (axis, [lo, hi]) in self.bounds().into_iter().enumerate() {
            if lo == f64::NEG_INFINITY && hi == f64::INFINITY {
                continue;
            }
            let [center, a, b] = [origin[axis], cosine[axis], sine[axis]];
            let amplitude = a.hypot(b);
            let [min, max] = [center - amplitude, center + amplitude];
            if lo > max || hi < min {
                return Vec::new();
            }
            if lo <= min && hi >= max {
                continue;
            }
            let trace = Harmonic([center, a, b]);
            active[axis] = Some((trace, lo..=hi));
            if lo > min {
                trace.level(lo, trim, &mut cuts);
            }
            if hi < max {
                trace.level(hi, trim, &mut cuts);
            }
        }
        if active.iter().all(Option::is_none) {
            return vec![candidate];
        }
        let inside = |t: f64| {
            let (sin, cos) = t.sin_cos();
            active.iter().flatten().all(|(trace, range)| {
                let [center, a, b] = trace.0;
                range.contains(&a.mul_add(cos, b.mul_add(sin, center)))
            })
        };
        if cuts.is_empty() {
            return inside(trim[0].midpoint(trim[1]))
                .then_some(candidate)
                .into_iter()
                .collect();
        }
        let mut output = Vec::new();
        drawing_events::spans(&mut cuts, trim, inside, |trim| {
            output.push(with_trim(candidate, trim))
        });
        output
    }

    fn bounds(self) -> [Span; 3] {
        let [u, v] = self
            .lateral
            .map(|window| window.unwrap_or([f64::NEG_INFINITY, f64::INFINITY]));
        [u, v, [self.far.unwrap_or(f64::NEG_INFINITY), self.near]]
    }
}

fn bounded(value: f64) -> bool {
    value.is_finite() && value.abs() <= 1e8
}

#[cfg(test)]
mod tests;
