use super::{Error, Point, add, rotate, scale, sub};

const MAX_SPANS: usize = 127;

/// An exact circular involute, oriented along a bounded parameter interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Involute {
    base_radius_mm: f64,
    interval: [f64; 2],
    angle_rad: f64,
    mirrored: bool,
}

/// A nonrational, clamped cubic B-spline. Internal knots have multiplicity two.
#[derive(Clone, Debug, PartialEq)]
pub struct CubicSpline {
    pub poles_mm: Vec<Point>,
    pub knots: Vec<f64>,
    pub multiplicities: Vec<u8>,
    pub error_bound_mm: f64,
}

impl Involute {
    pub fn new(base_radius_mm: f64, interval: [f64; 2], angle_rad: f64) -> Result<Self, Error> {
        let [start, end] = interval;
        if !base_radius_mm.is_finite()
            || !(0. < base_radius_mm && base_radius_mm <= super::MAX_COORDINATE_MM)
            || !(0. <= start && start < end && end <= std::f64::consts::PI)
            || !angle_rad.is_finite()
        {
            return Err(Error::InvalidDimensions);
        }
        Ok(Self {
            base_radius_mm,
            interval,
            angle_rad,
            mirrored: false,
        })
    }

    pub fn reversed(self) -> Self {
        Self {
            interval: [self.interval[1], self.interval[0]],
            ..self
        }
    }

    pub fn reflected(self) -> Self {
        Self {
            angle_rad: -self.angle_rad,
            mirrored: !self.mirrored,
            ..self
        }
    }

    pub fn rotated(self, angle_rad: f64) -> Result<Self, Error> {
        let angle_rad = self.angle_rad + angle_rad;
        if !angle_rad.is_finite() {
            return Err(Error::InvalidDimensions);
        }
        Ok(Self { angle_rad, ..self })
    }

    pub fn interval(self) -> [f64; 2] {
        self.interval
    }

    /// Evaluate at a fraction in 0..=1 along the oriented flank.
    pub fn point(self, fraction: f64) -> Point {
        self.at(self.parameter(fraction))
    }

    /// Unit tangent along the flank, including the one-sided limit at its base.
    pub fn tangent(self, fraction: f64) -> Point {
        let (sin, cos) = self.parameter(fraction).sin_cos();
        let direction = (self.interval[1] - self.interval[0]).signum();
        scale(self.map([cos, sin]), direction)
    }

    pub fn length_mm(self) -> f64 {
        let [a, b] = self.interval;
        0.5 * self.base_radius_mm * (b - a).abs() * (b + a)
    }

    /// Signed Green integral of this directed boundary about its base center.
    pub fn signed_area_mm2(self) -> f64 {
        let [a, b] = self.interval;
        let handedness = if self.mirrored { -1. } else { 1. };
        handedness * self.base_radius_mm.powi(2) * (b - a) * (b * b + b * a + a * a) / 6.
    }

    /// Composite cubic Hermite export with the requested geometric error.
    pub fn cubic_spline(self, tolerance_mm: f64) -> Result<CubicSpline, Error> {
        if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
            return Err(Error::InvalidDimensions);
        }
        let [start, end] = self.interval;
        let delta = (end - start).abs();
        let furthest = start.max(end);
        let roundoff = 64. * f64::EPSILON * self.base_radius_mm * (1. + furthest);
        // The Hermite remainder is bounded by max |f''''| h^4 / 384.
        // Each coordinate has |f''''| <= r sqrt(9+t^2). Reserve 10% of
        // the budget for evaluation and representation of control points.
        let fourth = 2_f64.sqrt() * self.base_radius_mm * 3_f64.hypot(furthest);
        let wanted = (delta / (384. * 0.9 * tolerance_mm / fourth).powf(0.25)).ceil();
        if !wanted.is_finite() || wanted > MAX_SPANS as f64 || roundoff > tolerance_mm * 0.1 {
            return Err(Error::Limit);
        }
        let count = (wanted as usize).max(1);
        let mut poles_mm = Vec::with_capacity(2 * count + 2);
        poles_mm.push(self.at(start));
        let knots: Vec<_> = (0..=count).map(|i| i as f64 / count as f64).collect();
        for pair in knots.windows(2) {
            let a = self.parameter(pair[0]);
            let b = self.parameter(pair[1]);
            let h = (b - a) / 3.;
            poles_mm.push(add(self.at(a), scale(self.derivative(a), h)));
            poles_mm.push(sub(self.at(b), scale(self.derivative(b), h)));
        }
        poles_mm.push(self.at(end));
        let mut multiplicities = vec![2; count + 1];
        multiplicities[0] = 4;
        multiplicities[count] = 4;
        let error_bound_mm = fourth * (delta / count as f64).powi(4) / 384. + roundoff;
        Ok(CubicSpline {
            poles_mm,
            knots,
            multiplicities,
            error_bound_mm,
        })
    }

    fn parameter(self, fraction: f64) -> f64 {
        let [a, b] = self.interval;
        a * (1. - fraction) + b * fraction
    }

    fn map(self, [x, y]: Point) -> Point {
        let y = if self.mirrored { -y } else { y };
        rotate([x, y], self.angle_rad)
    }

    fn at(self, t: f64) -> Point {
        let (sin, cos) = t.sin_cos();
        self.map(scale([cos + t * sin, sin - t * cos], self.base_radius_mm))
    }

    fn derivative(self, t: f64) -> Point {
        let (sin, cos) = t.sin_cos();
        scale(self.map([cos, sin]), self.base_radius_mm * t)
    }
}
