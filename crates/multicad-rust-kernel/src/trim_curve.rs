//! Analytic trim curves shared by intersecting plane and cylinder supports.
use std::f64::consts::{SQRT_2, TAU};

type Point = [f64; 3];
const DIRECTION_EPSILON: f64 = 64. * f64::EPSILON;
const MAX_COORDINATE_MM: f64 = 1e12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Unsupported,
    Limit,
}

/// An infinite right-circular support. Axial end caps are separate trims.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cylinder {
    origin_mm: Point,
    axis: Point,
    radius_mm: f64,
}

impl Cylinder {
    pub fn new(
        axis_origin_mm: Point,
        axis_direction: Point,
        radius_mm: f64,
    ) -> Result<Self, Error> {
        check_point(axis_origin_mm)?;
        check_radius(radius_mm)?;
        Ok(Self {
            origin_mm: axis_origin_mm,
            axis: unit(axis_direction)?,
            radius_mm,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    origin_mm: Point,
    normal: Point,
}

impl Plane {
    pub fn new(origin_mm: Point, normal: Point) -> Result<Self, Error> {
        check_point(origin_mm)?;
        Ok(Self {
            origin_mm,
            normal: unit(normal)?,
        })
    }
}

/// A right-handed frame around an analytic circle or ellipse. Parameters are
/// radians from the positive major axis towards the positive minor axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    center_mm: Point,
    normal: Point,
    major_axis: Point,
    minor_axis: Point,
    semiaxes_mm: [f64; 2],
}

impl Ellipse {
    pub fn new(
        center_mm: Point,
        normal: Point,
        major_direction: Point,
        semiaxes_mm: [f64; 2],
    ) -> Result<Self, Error> {
        check_point(center_mm)?;
        let [major, minor] = semiaxes_mm;
        check_radius(major)?;
        check_radius(minor)?;
        let normal = unit(normal)?;
        let reference = unit(major_direction)?;
        if minor > major || dot(reference, normal).abs() > DIRECTION_EPSILON {
            return Err(Error::Invalid);
        }
        let minor_axis = unit(cross(normal, reference))?;
        Ok(Self {
            center_mm,
            normal,
            major_axis: cross(minor_axis, normal),
            minor_axis,
            semiaxes_mm,
        })
    }

    pub fn center_mm(self) -> Point {
        self.center_mm
    }

    pub fn normal(self) -> Point {
        self.normal
    }

    pub fn major_axis(self) -> Point {
        self.major_axis
    }

    pub fn minor_axis(self) -> Point {
        self.minor_axis
    }

    pub fn semiaxes_mm(self) -> [f64; 2] {
        self.semiaxes_mm
    }

    pub fn point_at(self, parameter_rad: f64) -> Point {
        let (sin, cos) = parameter_rad.sin_cos();
        let [major, minor] = self.semiaxes_mm;
        std::array::from_fn(|i| {
            self.center_mm[i] + self.major_axis[i] * major * cos + self.minor_axis[i] * minor * sin
        })
    }

    /// Derivative with respect to the increasing ellipse parameter.
    pub fn tangent_at(self, parameter_rad: f64) -> Point {
        let (sin, cos) = parameter_rad.sin_cos();
        let [major, minor] = self.semiaxes_mm;
        std::array::from_fn(|i| {
            -self.major_axis[i] * major * sin + self.minor_axis[i] * minor * cos
        })
    }

    /// Return a parameter in [0, 2π) only for a point within the given tolerance.
    pub fn parameter_at(self, point_mm: Point, tolerance_mm: f64) -> Option<f64> {
        if !tolerance_mm.is_finite() || !(0. ..self.semiaxes_mm[1]).contains(&tolerance_mm) {
            return None;
        }
        let delta = sub(point_mm, self.center_mm);
        let x = dot(delta, self.major_axis) / self.semiaxes_mm[0];
        let y = dot(delta, self.minor_axis) / self.semiaxes_mm[1];
        let parameter = angle(y.atan2(x));
        (length(sub(point_mm, self.point_at(parameter))) <= tolerance_mm).then_some(parameter)
    }

    /// Intersect the ellipse with an infinite plane. Roots are parameters in
    /// [0, 2π), ordered numerically when there are two crossings.
    pub fn plane_crossings(self, plane: Plane, tolerance_mm: f64) -> Result<PlaneCrossings, Error> {
        if !tolerance_mm.is_finite() || !(0. ..self.semiaxes_mm[1]).contains(&tolerance_mm) {
            return Err(Error::Invalid);
        }
        let offset = dot(sub(plane.origin_mm, self.center_mm), plane.normal);
        let [a, b] = self.projected_coefficients(plane.normal);
        let extent = a.hypot(b);
        if offset.abs() + extent <= tolerance_mm {
            return Ok(PlaneCrossings::Coincident);
        }
        if offset.abs() > extent + tolerance_mm {
            return Ok(PlaneCrossings::None);
        }
        let peak = b.atan2(a);
        if (offset.abs() - extent).abs() <= tolerance_mm {
            let extremum = if offset >= 0. { peak } else { peak + TAU * 0.5 };
            return Ok(PlaneCrossings::Tangent(angle(extremum)));
        }
        let displacement = (offset / extent).acos();
        let mut crossings = [angle(peak - displacement), angle(peak + displacement)];
        crossings.sort_by(f64::total_cmp);
        Ok(PlaneCrossings::Crossing(crossings))
    }

    fn projected_coefficients(self, direction: Point) -> [f64; 2] {
        [
            self.semiaxes_mm[0] * dot(self.major_axis, direction),
            self.semiaxes_mm[1] * dot(self.minor_axis, direction),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaneCrossings {
    None,
    Coincident,
    Tangent(f64),
    Crossing([f64; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EllipseArc {
    ellipse: Ellipse,
    start_rad: f64,
    sweep_rad: f64,
}

impl EllipseArc {
    /// A positive sweep follows the ellipse's normal. A complete turn is
    /// explicit, as coincident trim vertices alone do not select a full loop.
    pub fn new(ellipse: Ellipse, start_rad: f64, sweep_rad: f64) -> Result<Self, Error> {
        if !start_rad.is_finite()
            || !sweep_rad.is_finite()
            || sweep_rad == 0.
            || sweep_rad.abs() > TAU
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            ellipse,
            start_rad: angle(start_rad),
            sweep_rad,
        })
    }

    pub fn from_endpoints(
        ellipse: Ellipse,
        [start, end]: [Point; 2],
        clockwise: bool,
        tolerance_mm: f64,
    ) -> Result<Self, Error> {
        let start_rad = ellipse
            .parameter_at(start, tolerance_mm)
            .ok_or(Error::Invalid)?;
        let end_rad = ellipse
            .parameter_at(end, tolerance_mm)
            .ok_or(Error::Invalid)?;
        if length(sub(start, end)) <= tolerance_mm {
            return Err(Error::Invalid);
        }
        let sweep = if clockwise {
            -angle(start_rad - end_rad)
        } else {
            angle(end_rad - start_rad)
        };
        Self::new(ellipse, start_rad, sweep)
    }

    pub fn ellipse(self) -> Ellipse {
        self.ellipse
    }

    pub fn start_rad(self) -> f64 {
        self.start_rad
    }

    pub fn sweep_rad(self) -> f64 {
        self.sweep_rad
    }

    pub fn endpoints(self) -> [Point; 2] {
        [self.start_rad, self.start_rad + self.sweep_rad].map(|t| self.ellipse.point_at(t))
    }

    /// Directional range in millimeters. For an AABB use the three unit axes.
    pub fn range_along(self, direction: Point) -> [f64; 2] {
        let [a, b] = self.ellipse.projected_coefficients(direction);
        let center = dot(self.ellipse.center_mm, direction);
        let projection = |t: f64| center + a * t.cos() + b * t.sin();
        let [start, end] = [self.start_rad, self.start_rad + self.sweep_rad].map(projection);
        let mut range = [start.min(end), start.max(end)];
        let peak = b.atan2(a);
        let extent = a.hypot(b);
        for (i, extremum) in [peak + TAU * 0.5, peak].into_iter().enumerate() {
            let traveled = angle((extremum - self.start_rad) * self.sweep_rad.signum());
            if traveled <= self.sweep_rad.abs() {
                range[i] = center + if i == 0 { -extent } else { extent };
            }
        }
        range
    }
}

/// Intersect an infinite cylinder with a nonparallel plane. A plane normal to
/// the cylinder axis produces a circle. Parallel cuts have a different curve family.
pub fn cylinder_plane(cylinder: Cylinder, plane: Plane) -> Result<Ellipse, Error> {
    let cosine = dot(plane.normal, cylinder.axis);
    if cosine.abs() <= DIRECTION_EPSILON {
        return Err(Error::Unsupported);
    }
    let along_axis = dot(sub(plane.origin_mm, cylinder.origin_mm), plane.normal) / cosine;
    let center = std::array::from_fn(|i| cylinder.origin_mm[i] + along_axis * cylinder.axis[i]);
    let side = cross(plane.normal, cylinder.axis);
    let major = if length(side) <= DIRECTION_EPSILON {
        let [a, b, c] = plane.normal.map(f64::abs);
        let least = if a <= b && a <= c {
            [1., 0., 0.]
        } else if b <= c {
            [0., 1., 0.]
        } else {
            [0., 0., 1.]
        };
        cross(plane.normal, least)
    } else {
        cross(side, plane.normal)
    };
    let radius = cylinder.radius_mm;
    Ellipse::new(
        center,
        plane.normal,
        major,
        [radius / cosine.abs().min(1.), radius],
    )
}

/// Two equal-radius perpendicular cylinders with intersecting axes meet on
/// the two bisector planes. Skew axes and distinct radii have a different
/// intersection family. Only frame roundoff is tolerated in axis coincidence.
pub fn equal_perpendicular_cylinders(
    a: Cylinder,
    b: Cylinder,
    tolerance_mm: f64,
) -> Result<[Ellipse; 2], Error> {
    if !tolerance_mm.is_finite() || tolerance_mm < 0. {
        return Err(Error::Invalid);
    }
    let cosine = dot(a.axis, b.axis);
    if a.radius_mm != b.radius_mm || cosine.abs() > DIRECTION_EPSILON {
        return Err(Error::Unsupported);
    }
    let delta = sub(b.origin_mm, a.origin_mm);
    let x = dot(delta, a.axis);
    let y = dot(delta, b.axis);
    let determinant = 1. - cosine * cosine;
    let s = (x - cosine * y) / determinant;
    let t = (cosine * x - y) / determinant;
    let p = std::array::from_fn(|i| a.origin_mm[i] + s * a.axis[i]);
    let q = std::array::from_fn(|i| b.origin_mm[i] + t * b.axis[i]);
    let roundoff = DIRECTION_EPSILON * length(delta).max(1.);
    if length(sub(p, q)) > tolerance_mm.min(roundoff) {
        return Err(Error::Unsupported);
    }
    let center = std::array::from_fn(|i| f64::midpoint(p[i], q[i]));
    let plus = std::array::from_fn(|i| a.axis[i] + b.axis[i]);
    let minus = sub(a.axis, b.axis);
    let radii = [a.radius_mm * SQRT_2, a.radius_mm];
    Ok([
        Ellipse::new(center, plus, minus, radii)?,
        Ellipse::new(center, minus, plus, radii)?,
    ])
}

fn check_point(point: Point) -> Result<(), Error> {
    if point.into_iter().any(|x| !x.is_finite()) {
        return Err(Error::Invalid);
    }
    if point.into_iter().any(|x| x.abs() > MAX_COORDINATE_MM) {
        return Err(Error::Limit);
    }
    Ok(())
}

fn check_radius(radius: f64) -> Result<(), Error> {
    if !radius.is_finite() || radius <= 0. {
        return Err(Error::Invalid);
    }
    if radius > MAX_COORDINATE_MM {
        return Err(Error::Limit);
    }
    Ok(())
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot([x, y, z]: Point, [a, b, c]: Point) -> f64 {
    x * a + y * b + z * c
}

fn cross([x, y, z]: Point, [a, b, c]: Point) -> Point {
    [y * c - z * b, z * a - x * c, x * b - y * a]
}

fn length([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

fn unit(vector: Point) -> Result<Point, Error> {
    let max = vector.into_iter().map(f64::abs).fold(0., f64::max);
    if max == 0. || !vector.into_iter().all(f64::is_finite) {
        return Err(Error::Invalid);
    }
    let scaled = vector.map(|x| x / max);
    let norm = length(scaled);
    Ok(scaled.map(|x| x / norm))
}

fn angle(radians: f64) -> f64 {
    let value = radians.rem_euclid(TAU);
    if value == TAU { 0. } else { value }
}

#[cfg(test)]
mod tests;
