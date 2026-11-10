//! The eye-facing hemisphere of an exact sphere.
use super::*;

pub(super) struct Sphere {
    center: Point,
    radius: f64,
}

impl AnalyticFace {
    /// An outward spherical surface in drawing coordinates.
    pub fn sphere(center: Point, radius: f64) -> Result<Self, ProjectionError> {
        if !bounded(center) || !(1e-7..=1e7).contains(&radius) {
            return Err(ProjectionError::Invalid);
        }
        Ok(Self {
            surface: Surface::Sphere(Sphere { center, radius }),
            bounds: Bounds {
                min: center.map(|value| value - radius),
                max: center.map(|value| value + radius),
            },
            depth: [None; 2],
        })
    }
}

impl Sphere {
    pub(super) fn hide(
        &self,
        path: Path,
        span: Span,
        gap: f64,
        roundoff: f64,
        limits: [Option<f64>; 2],
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        let [x, y, z] = std::array::from_fn(|axis| {
            path.coordinate(self.center, std::array::from_fn(|i| f64::from(i == axis)))
        });
        let shifted = z.offset(gap);
        if limits == [None; 2] && self.coincident_contour(path, gap, roundoff) {
            work.hidden.push(span);
            return Ok(());
        }
        work.charge(96)?;
        Trace::norm([x, y], self.radius - roundoff, span, &mut work.crossings);
        Trace::norm([x, y, shifted], self.radius, span, &mut work.crossings);
        let [low, high] = limits.map(|limit| limit.map(|value| value - self.center[2]));
        for level in [low, high].into_iter().flatten() {
            if (0. ..self.radius).contains(&level) {
                let radius = ((self.radius - level) * (self.radius + level)).sqrt();
                Trace::norm([x, y], radius, span, &mut work.crossings);
            }
        }
        drawing_events::spans(
            &mut work.crossings,
            span,
            |t| {
                let radial = x.at(t).hypot(y.at(t));
                if radial >= self.radius - roundoff {
                    return false;
                }
                let hit = ((self.radius - radial) * (self.radius + radial)).sqrt();
                hit > shifted.at(t)
                    && low.is_none_or(|low| hit >= low)
                    && high.is_none_or(|high| hit <= high)
            },
            |span| work.hidden.push(span),
        );
        Ok(())
    }

    fn coincident_contour(&self, path: Path, gap: f64, roundoff: f64) -> bool {
        let delta = sub(path.origin, self.center);
        let [u, v] = [path.b, path.c];
        path.harmonic
            && delta[2] < -gap
            && delta[0].hypot(delta[1]) <= roundoff
            && u[2].abs().max(v[2].abs()) <= roundoff
            && (length(u) - self.radius).abs() <= roundoff
            && (length(v) - self.radius).abs() <= roundoff
            && dot(u, v).abs() <= roundoff * self.radius
    }
}
