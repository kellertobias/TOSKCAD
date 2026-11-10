use super::{Edge, Point, Profile, normal_sweep, scale, sub};
use crate::{
    display::{DisplayMesh, Footprint, MeshError, Tessellation},
    revolution::length,
};

const MAX_EDGE_SEGMENTS: usize = 65_536;

impl Edge {
    /// Evaluate at a fraction in 0..=1 along the directed edge.
    pub fn point(self, fraction: f64) -> Point {
        match self {
            Self::Planar(edge) => edge.point(fraction),
            Self::Involute(edge) => edge.point(fraction),
        }
    }

    pub fn tangent(self, fraction: f64) -> Point {
        match self {
            Self::Planar(normal_sweep::Edge::Line { a, b }) => {
                let direction = sub(b, a);
                scale(direction, 1. / length(direction))
            }
            Self::Planar(
                edge @ (normal_sweep::Edge::Arc {
                    center, clockwise, ..
                }
                | normal_sweep::Edge::Circle {
                    center, clockwise, ..
                }),
            ) => {
                let [x, y] = sub(edge.point(fraction), center);
                let sign = if clockwise { -1. } else { 1. };
                scale([-y, x], sign / x.hypot(y))
            }
            Self::Involute(edge) => edge.tangent(fraction),
        }
    }

    pub fn length_mm(self) -> f64 {
        match self {
            Self::Planar(edge) => edge.length_mm(),
            Self::Involute(edge) => edge.length_mm(),
        }
    }

    pub fn centroid(self) -> Point {
        match self {
            Self::Planar(edge) => edge.centroid(),
            Self::Involute(edge) => edge.centroid(),
        }
    }

    pub fn support(self, direction: Point) -> f64 {
        match self {
            Self::Planar(edge) => edge.support(direction),
            Self::Involute(edge) => edge.support(direction),
        }
    }

    /// Sample the directed edge, including its exact endpoints.
    pub fn sample(
        self,
        deflection_mm: f64,
        angular_deflection_rad: f64,
    ) -> Result<Vec<Point>, MeshError> {
        let options = Tessellation {
            deflection: deflection_mm,
            angle: angular_deflection_rad,
            triangle_limit: MAX_EDGE_SEGMENTS,
        };
        options.check()?;
        let count = self.segment_count(options)?;
        Ok((0..=count)
            .map(|i| self.point(i as f64 / count as f64))
            .collect())
    }

    fn segment_count(self, options: Tessellation) -> Result<usize, MeshError> {
        let count = match self {
            Self::Planar(edge) => match edge.radius_mm().zip(edge.signed_sweep_rad()) {
                Some((radius, angle)) => options.arc_segments(radius, angle)?.max(2),
                None => 1,
            },
            Self::Involute(edge) => {
                let [a, b] = edge.interval();
                let range = (b - a).abs();
                // The involute's second derivative has norm r sqrt(1+t²).
                // Linear interpolation on a parameter span h therefore
                // deviates by at most max|f''| h²/8. Its directed tangent
                // rotates through exactly h, including the limit at t=0.
                let second_derivative = edge.base_radius_mm() * a.max(b).hypot(1.);
                let max_span = (8. * options.deflection / second_derivative)
                    .sqrt()
                    .min(options.angle);
                let needed = (range / max_span).ceil();
                if !needed.is_finite() || needed > options.triangle_limit as f64 {
                    return Err(MeshError::BudgetExceeded);
                }
                (needed as usize).max(1)
            }
        };
        Ok(count)
    }
}

impl Profile {
    pub fn surface_area_mm2(&self) -> f64 {
        let perimeter = self.outline.iter().map(|e| e.edge.length_mm()).sum::<f64>()
            + self.bore.map_or(0., normal_sweep::Edge::length_mm);
        2. * self.area_mm2 + perimeter * self.height_mm
    }

    /// Support of the swept material in a local 3D direction.
    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if !direction.into_iter().all(f64::is_finite) {
            return f64::NAN;
        }
        let magnitude = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if magnitude == 0. {
            return 0.;
        }
        let [x, y, z] = direction.map(|component| component / magnitude);
        let radial = self
            .outline
            .iter()
            .map(|e| e.edge.support([x, y]))
            .fold(f64::NEG_INFINITY, f64::max);
        (radial + (z * self.height_mm).max(0.)) * magnitude
    }

    pub fn bounds(&self) -> [[f64; 3]; 2] {
        [
            [
                -self.support([-1., 0., 0.]),
                -self.support([0., -1., 0.]),
                0.,
            ],
            [
                self.support([1., 0., 0.]),
                self.support([0., 1., 0.]),
                self.height_mm,
            ],
        ]
    }

    /// Mesh with cap face indices 0 and 1, then one face per outline edge
    /// in authored order, followed by the optional bore wall.
    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let bore = self.bore.and_then(normal_sweep::Edge::radius_mm);
        // The exact outer rim never enters the root circle. Restrict
        // chord sagitta so its samples cannot cross a near-root bore.
        let outer_options = Tessellation {
            deflection: bore.map_or(options.deflection, |radius| {
                options.deflection.min((self.root_radius_mm - radius) / 2.)
            }),
            ..options
        };
        let mut outer = Vec::with_capacity(self.outline.len());
        let limit = (options.triangle_limit + 4) / 4;
        for (index, entry) in self.outline.iter().enumerate() {
            let count = entry.edge.segment_count(outer_options)?;
            if outer.len() + count > limit {
                return Err(MeshError::BudgetExceeded);
            }
            let face = (index + 2) as u32;
            outer.extend((0..count).map(|i| (entry.edge.point(i as f64 / count as f64), face)));
        }
        let mut footprint = Footprint::default();
        footprint.ring(outer.into_iter(), options)?;
        if let Some(radius) = bore {
            footprint.circle(
                [0.; 2],
                radius,
                (self.outline.len() + 2) as u32,
                true,
                options,
            )?;
        }
        footprint.tessellate(0., self.height_mm)
    }
}
