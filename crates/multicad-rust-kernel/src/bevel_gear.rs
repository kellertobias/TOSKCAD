//! Spherical-involute straight bevel gears bounded by toe and heel back cones.
//!
//! Every tooth surface is a cone through the pitch apex over one curve on the
//! unit sphere. The solid is therefore the set of points `r * u` whose
//! direction `u` lies in the spherical tooth domain and whose distance `r`
//! lies between the two back cones, less an optional axial bore.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub mod clearance;
mod mesh;
mod topology;
pub mod view;
pub use topology::{Curve, Edge, Face, FaceKind, Shape, Topology};

pub type Point = [f64; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub teeth: u32,
    pub pitch_cone_angle_rad: f64,
    pub base_cone_angle_rad: f64,
    pub tip_cone_angle_rad: f64,
    pub root_cone_angle_rad: f64,
    pub outer_cone_distance_mm: f64,
    /// Distance along the pitch generator between the two back cones.
    pub face_width_mm: f64,
    pub bore_radius_mm: f64,
    pub half_base_tooth_angle_rad: f64,
    /// Small-circle radius on the outer reference sphere.
    pub root_fillet_radius_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDimensions,
    InvalidProfile,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum End {
    Heel,
    Toe,
}

impl End {
    pub const BOTH: [Self; 2] = [Self::Heel, Self::Toe];

    pub fn name(self) -> &'static str {
        match self {
            Self::Heel => "heel",
            Self::Toe => "toe",
        }
    }
}

/// One boundary curve of a tooth sector, in traversal order from the start
/// of the sector to its end at increasing azimuth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Piece {
    RootLeft,
    FilletLeft,
    MeridianLeft,
    InvoluteLeft,
    Tip,
    InvoluteRight,
    MeridianRight,
    FilletRight,
    RootRight,
}

impl Piece {
    /// Native face roles; both root lands of a sector share one role.
    pub fn role(self) -> &'static str {
        match self {
            Self::RootLeft | Self::RootRight => "root",
            Self::FilletLeft => "fillet.left",
            Self::MeridianLeft => "meridian.left",
            Self::InvoluteLeft => "spherical_involute.left",
            Self::Tip => "tip",
            Self::InvoluteRight => "spherical_involute.right",
            Self::MeridianRight => "meridian.right",
            Self::FilletRight => "fillet.right",
        }
    }

    fn left(self) -> bool {
        matches!(
            self,
            Self::RootLeft | Self::FilletLeft | Self::MeridianLeft | Self::InvoluteLeft
        )
    }
}

/// Direction, its derivative with respect to the piece parameter, and the
/// derived polar coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub direction: Point,
    pub derivative: Point,
    pub gamma: f64,
    pub phi: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Containment {
    Inside,
    Boundary,
    Outside,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bevel {
    spec: Spec,
    rounding: f64,
    center: Point,
    center_gamma: f64,
    center_phi: f64,
    fillet_axes: [Point; 2],
    arc: f64,
    touch_gamma: f64,
    touch_t: f64,
    meridian: bool,
    tip_t: f64,
    half_tip: f64,
    inner_gamma: f64,
    pieces: Vec<Piece>,
}

pub(crate) fn polar(gamma: f64, phi: f64) -> Point {
    let (sg, cg) = gamma.sin_cos();
    let (sp, cp) = phi.sin_cos();
    [sg * cp, sg * sp, cg]
}

pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: Point, s: f64) -> Point {
    a.map(|v| v * s)
}

pub(crate) fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: Point) -> Point {
    scale(a, 1. / norm(a))
}

fn rotate_z(p: Point, angle: f64) -> Point {
    let (s, c) = angle.sin_cos();
    [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]]
}

fn mirror(p: Point) -> Point {
    [p[0], -p[1], p[2]]
}

/// Sixteen-point Gauss-Legendre rule over `parts` equal subintervals.
pub(crate) fn integrate(f: impl Fn(f64) -> f64, a: f64, b: f64, parts: usize) -> f64 {
    const N: usize = 16;
    let mut nodes = [(0., 0.); N];
    for (i, node) in nodes.iter_mut().enumerate() {
        let mut x = (PI * (i as f64 + 0.75) / (N as f64 + 0.5)).cos();
        let mut derivative = 1.;
        for _ in 0..32 {
            let (mut p0, mut p1) = (1., x);
            for k in 2..=N {
                let k = k as f64;
                (p0, p1) = (p1, ((2. * k - 1.) * x * p1 - (k - 1.) * p0) / k);
            }
            derivative = N as f64 * (x * p1 - p0) / (x * x - 1.);
            x -= p1 / derivative;
        }
        *node = (x, 2. / ((1. - x * x) * derivative * derivative));
    }
    let h = (b - a) / parts as f64;
    (0..parts)
        .map(|part| {
            let mid = a + h * (part as f64 + 0.5);
            nodes
                .iter()
                .map(|(x, w)| w * f(mid + h * x / 2.))
                .sum::<f64>()
                * h
                / 2.
        })
        .sum()
}

impl Bevel {
    pub fn new(spec: Spec, minimum_mm: f64, maximum_coordinate_mm: f64) -> Result<Self, Error> {
        let Spec {
            teeth,
            pitch_cone_angle_rad: pitch,
            base_cone_angle_rad: base,
            tip_cone_angle_rad: tip,
            root_cone_angle_rad: root,
            outer_cone_distance_mm: distance,
            face_width_mm: width,
            bore_radius_mm: bore,
            half_base_tooth_angle_rad: half,
            root_fillet_radius_mm: fillet,
        } = spec;
        let values = [
            pitch, base, tip, root, distance, width, bore, half, fillet, minimum_mm,
        ];
        if !(4..=64).contains(&teeth)
            || values.iter().any(|v| !v.is_finite())
            || minimum_mm <= 0.
            || !(root > 0. && root < pitch)
            || !(base > 0. && base < pitch)
            || !(pitch < tip && tip < FRAC_PI_2)
            || !(width > 0. && width < distance)
            || bore < 0.
            || !(fillet > 0. && fillet < distance)
            || !(half > 0. && half < FRAC_PI_2)
        {
            return Err(Error::InvalidDimensions);
        }
        let heel_tip = distance * tip.sin() / (tip - pitch).cos();
        if heel_tip.max(distance / pitch.cos()) > maximum_coordinate_mm
            || maximum_coordinate_mm.is_nan()
        {
            return Err(Error::Limit);
        }
        let toe = distance - width;
        if toe * root.sin() / (root - pitch).cos() - bore < minimum_mm {
            return Err(Error::InvalidProfile);
        }
        let mut rounding = (fillet / distance).asin();
        // The fillet touches the meridian plane at γ_t with
        // cos γ_t = cos(root + ρ) / cos ρ. A meridian between γ_t and the base
        // cone shorter than the minimum feature is absorbed, as natively:
        // the rounding grows to the root ρ* of
        // g(ρ) = cos(root + ρ) − cos β cos ρ, where γ_t = β exactly and the
        // radially starting involute continues the fillet tangentially.
        // g decreases, dγ_t/dρ = sin(root) / (sin γ_t cos²ρ), so the fillet
        // radius changes by at most `minimum · sin β / sin(root)`.
        let touch_of = |rho: f64| ((root + rho).cos() / rho.cos()).clamp(-1., 1.).acos();
        let gap = |rho: f64| (root + rho).cos() - base.cos() * rho.cos();
        let sliver = gap(rounding) >= 0.
            && base - touch_of(rounding) > 0.
            && base - touch_of(rounding) < minimum_mm / distance;
        if sliver {
            let (mut lo, mut hi) = (rounding, FRAC_PI_2 - root);
            for _ in 0..200 {
                let middle = (lo + hi) / 2.;
                if gap(middle) >= 0. {
                    lo = middle;
                } else {
                    hi = middle;
                }
            }
            rounding = lo;
        }
        let center_gamma = root + rounding;
        let meridian = sliver || center_gamma <= (base.cos() * rounding.cos()).acos();
        let mut gear = Self {
            spec,
            rounding,
            center: [0.; 3],
            center_gamma,
            center_phi: 0.,
            fillet_axes: [[0.; 3]; 2],
            arc: 0.,
            touch_gamma: 0.,
            touch_t: 0.,
            meridian,
            tip_t: (tip.cos() / base.cos()).acos() / base.sin(),
            half_tip: 0.,
            inner_gamma: root / 2.,
            pieces: vec![],
        };
        let touch = if meridian {
            gear.touch_gamma = if sliver {
                base
            } else {
                (center_gamma.cos() / rounding.cos()).acos()
            };
            gear.center_phi = half + (rounding.sin() / center_gamma.sin()).asin();
            gear.center = polar(center_gamma, gear.center_phi);
            polar(gear.touch_gamma, half)
        } else {
            let sigma = (center_gamma.cos() / base.cos()).acos() - rounding;
            gear.touch_t = sigma / base.sin();
            let touch = gear.involute(gear.touch_t).0;
            let normal = unit(cross(touch, unit(gear.involute(gear.touch_t).1)));
            gear.center =
                std::array::from_fn(|i| touch[i] * rounding.cos() + normal[i] * rounding.sin());
            gear.center_gamma = gear.center[2].acos();
            gear.center_phi = gear.center[1].atan2(gear.center[0]);
            gear.touch_gamma = touch[2].acos();
            touch
        };
        let root_point = polar(root, gear.center_phi);
        let a = unit(sub(root_point, scale(gear.center, rounding.cos())));
        let b = unit(cross(gear.center, a));
        gear.fillet_axes = [a, b];
        gear.arc = dot(touch, b).atan2(dot(touch, a));
        let tip_point = gear.involute(gear.tip_t).0;
        gear.half_tip = tip_point[1].atan2(tip_point[0]);
        let toe_scale = toe / (root - pitch).cos();
        let spacing = PI / f64::from(teeth);
        let meridian_length = base - gear.touch_gamma;
        if !gear.arc.is_finite()
            || gear.arc.abs() < 1e-8
            || gear.center_phi <= 0.
            || gear.center_phi >= spacing - minimum_mm / toe_scale
            || gear.touch_gamma >= pitch
            || gear.touch_gamma >= tip
            || gear.half_tip <= minimum_mm / (distance * tip.sin())
            || gear.half_tip >= gear.center_phi
            || root + rounding >= tip
            || (meridian && meridian_length < -1e-12)
            || (meridian && meridian_length > 0. && meridian_length < minimum_mm / distance)
            || [End::Heel, End::Toe]
                .into_iter()
                .any(|end| (gear.end_radius(end, gear.inner_gamma) - bore).abs() < minimum_mm)
        {
            return Err(Error::InvalidProfile);
        }
        gear.meridian = meridian && meridian_length > 0.;
        gear.pieces = [
            Piece::RootLeft,
            Piece::FilletLeft,
            Piece::MeridianLeft,
            Piece::InvoluteLeft,
            Piece::Tip,
            Piece::InvoluteRight,
            Piece::MeridianRight,
            Piece::FilletRight,
            Piece::RootRight,
        ]
        .into_iter()
        .filter(|piece| {
            gear.meridian || !matches!(piece, Piece::MeridianLeft | Piece::MeridianRight)
        })
        .collect();
        Ok(gear)
    }

    pub fn spec(&self) -> Spec {
        self.spec
    }

    pub fn teeth(&self) -> u32 {
        self.spec.teeth
    }

    /// Ordered boundary pieces of every tooth sector.
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    pub fn has_meridian(&self) -> bool {
        self.meridian
    }

    pub fn bore_radius_mm(&self) -> Option<f64> {
        (self.spec.bore_radius_mm > 0.).then_some(self.spec.bore_radius_mm)
    }

    pub fn end_distance_mm(&self, end: End) -> f64 {
        match end {
            End::Heel => self.spec.outer_cone_distance_mm,
            End::Toe => self.spec.outer_cone_distance_mm - self.spec.face_width_mm,
        }
    }

    /// Distance from the pitch apex to the back cone along polar angle γ.
    pub fn end_distance_along(&self, end: End, gamma: f64) -> f64 {
        self.end_distance_mm(end) / (gamma - self.spec.pitch_cone_angle_rad).cos()
    }

    /// Distance from the axis of a back-cone point at polar angle γ.
    pub fn end_radius(&self, end: End, gamma: f64) -> f64 {
        self.end_distance_along(end, gamma) * gamma.sin()
    }

    /// Axial height of the apex of one back cone above the pitch apex.
    pub fn back_cone_apex_mm(&self, end: End) -> f64 {
        self.end_distance_mm(end) / self.spec.pitch_cone_angle_rad.cos()
    }

    /// Axial height of a back-cone point at axial radius ρ.
    pub fn back_cone_height_mm(&self, end: End, radius: f64) -> f64 {
        self.back_cone_apex_mm(end) - radius * self.spec.pitch_cone_angle_rad.tan()
    }

    pub fn point(&self, end: End, direction: Point, gamma: f64) -> Point {
        scale(direction, self.end_distance_along(end, gamma))
    }

    /// Polar angle where the bore meets one back cone.
    pub fn bore_gamma(&self, end: End) -> Option<f64> {
        let bore = self.bore_radius_mm()?;
        // ρ(γ) = d sin γ / cos(γ - δ) ⇒ tan γ = ρ cos δ / (d - ρ sin δ).
        let (s, c) = self.spec.pitch_cone_angle_rad.sin_cos();
        Some((bore * c).atan2(self.end_distance_mm(end) - bore * s))
    }

    /// Whether the inner apex sector of one cap survives the bore.
    pub fn has_core(&self, end: End) -> bool {
        self.bore_radius_mm()
            .is_none_or(|bore| bore < self.end_radius(end, self.inner_gamma))
    }

    pub fn inner_gamma(&self) -> f64 {
        self.inner_gamma
    }

    pub fn root_gamma(&self) -> f64 {
        self.spec.root_cone_angle_rad
    }

    pub fn tip_gamma(&self) -> f64 {
        self.spec.tip_cone_angle_rad
    }

    pub fn sector_start(&self, sector: u32) -> f64 {
        TAU * f64::from(sector) / f64::from(self.spec.teeth) - PI / f64::from(self.spec.teeth)
    }

    fn involute(&self, t: f64) -> (Point, Point) {
        let (b, c) = self.spec.base_cone_angle_rad.sin_cos();
        let v = b * t;
        let (st, ct) = t.sin_cos();
        let (sv, cv) = v.sin_cos();
        let x = b * ct * cv + st * sv;
        let y = b * st * cv - ct * sv;
        let dx = c * c * ct * sv;
        let dy = c * c * st * sv;
        let (sh, ch) = self.spec.half_base_tooth_angle_rad.sin_cos();
        (
            [x * ch + y * sh, x * sh - y * ch, c * cv],
            [dx * ch + dy * sh, dx * sh - dy * ch, -b * c * sv],
        )
    }

    fn fillet(&self, theta: f64) -> (Point, Point) {
        let [a, b] = self.fillet_axes;
        let (s, c) = theta.sin_cos();
        let (sr, cr) = self.rounding.sin_cos();
        (
            std::array::from_fn(|i| cr * self.center[i] + sr * (a[i] * c + b[i] * s)),
            std::array::from_fn(|i| sr * (-a[i] * s + b[i] * c)),
        )
    }

    fn circle(gamma: f64, phi: f64, rate: f64) -> (Point, Point) {
        let (sp, cp) = phi.sin_cos();
        let s = gamma.sin();
        (
            [s * cp, s * sp, gamma.cos()],
            [-s * sp * rate, s * cp * rate, 0.],
        )
    }

    /// Exact boundary point of sector 0 at parameter `s` in `[0, 1]`.
    pub fn sample(&self, piece: Piece, s: f64) -> Sample {
        let spacing = PI / f64::from(self.spec.teeth);
        let base = self.spec.base_cone_angle_rad;
        let half = self.spec.half_base_tooth_angle_rad;
        let (t0, t1) = (self.touch_t, self.tip_t);
        let (p, d) = match piece {
            Piece::RootLeft | Piece::RootRight => {
                let (from, to) = if piece == Piece::RootLeft {
                    (spacing, self.center_phi)
                } else {
                    (self.center_phi, spacing)
                };
                Self::circle(
                    self.spec.root_cone_angle_rad,
                    from + (to - from) * s,
                    to - from,
                )
            }
            Piece::Tip => Self::circle(
                self.spec.tip_cone_angle_rad,
                -self.half_tip + 2. * self.half_tip * s,
                2. * self.half_tip,
            ),
            Piece::FilletLeft => {
                let (p, d) = self.fillet(self.arc * s);
                (p, scale(d, self.arc))
            }
            Piece::FilletRight => {
                let (p, d) = self.fillet(self.arc * (1. - s));
                (p, scale(d, -self.arc))
            }
            Piece::MeridianLeft | Piece::MeridianRight => {
                let (from, to) = if piece == Piece::MeridianLeft {
                    (self.touch_gamma, base)
                } else {
                    (base, self.touch_gamma)
                };
                let gamma = from + (to - from) * s;
                let (sg, cg) = gamma.sin_cos();
                let (sh, ch) = half.sin_cos();
                (
                    [sg * ch, sg * sh, cg],
                    scale([cg * ch, cg * sh, -sg], to - from),
                )
            }
            Piece::InvoluteLeft => {
                let (p, d) = self.involute(t0 + (t1 - t0) * s);
                (p, scale(d, t1 - t0))
            }
            Piece::InvoluteRight => {
                let (p, d) = self.involute(t1 + (t0 - t1) * s);
                (p, scale(d, t0 - t1))
            }
        };
        let (p, d) = if piece.left() {
            (mirror(p), mirror(d))
        } else {
            (p, d)
        };
        Sample {
            direction: p,
            derivative: d,
            gamma: p[2].clamp(-1., 1.).acos(),
            phi: p[1].atan2(p[0]),
        }
    }

    /// Conservative bound of |d²u/ds²| on the unit sphere for one piece.
    pub(crate) fn curvature_bound(&self, piece: Piece) -> f64 {
        match piece {
            Piece::RootLeft | Piece::RootRight => {
                let span = PI / f64::from(self.spec.teeth) - self.center_phi;
                span * span * self.spec.root_cone_angle_rad.sin()
            }
            Piece::Tip => 4. * self.half_tip * self.half_tip * self.spec.tip_cone_angle_rad.sin(),
            Piece::FilletLeft | Piece::FilletRight => self.arc * self.arc * self.rounding.sin(),
            Piece::MeridianLeft | Piece::MeridianRight => {
                (self.spec.base_cone_angle_rad - self.touch_gamma).powi(2)
            }
            // Each planar component combines products of cos/sin(t) and
            // cos/sin(t sin β) with unit coefficients: |x''|, |y''| ≤ 8 and
            // |z''| ≤ 1 before the orthogonal half-tooth rotation.
            Piece::InvoluteLeft | Piece::InvoluteRight => 12. * (self.tip_t - self.touch_t).powi(2),
        }
    }

    /// Half tooth width in azimuth at polar angle γ in `[root, tip]`.
    pub fn half_width(&self, gamma: f64) -> f64 {
        let half = self.spec.half_base_tooth_angle_rad;
        let base = self.spec.base_cone_angle_rad;
        if gamma <= self.touch_gamma {
            let (sc, cc) = self.center_gamma.sin_cos();
            let (sg, cg) = gamma.sin_cos();
            let ratio = (self.rounding.cos() - cg * cc) / (sg * sc);
            self.center_phi - ratio.clamp(-1., 1.).acos()
        } else if self.meridian && gamma <= base {
            half
        } else {
            let sigma = (gamma.cos() / base.cos()).clamp(-1., 1.).acos();
            half - (sigma / base.sin() - sigma.sin().atan2(base.sin() * sigma.cos()))
        }
    }

    /// Signed angular margin of a unit direction from the tooth domain.
    pub fn direction_margin(&self, direction: Point) -> f64 {
        let gamma = direction[2].clamp(-1., 1.).acos();
        let pitch = TAU / f64::from(self.spec.teeth);
        let phi = direction[1].atan2(direction[0]);
        let local = phi - (phi / pitch).round() * pitch;
        let root = self.spec.root_cone_angle_rad;
        let tip = self.spec.tip_cone_angle_rad;
        let level = gamma.clamp(root, tip);
        let tooth = (tip - gamma).min((self.half_width(level) - local.abs()) * gamma.sin());
        (root - gamma).max(tooth)
    }

    /// Material classification of a local point with a distance band.
    pub fn classify(&self, point: Point, tolerance_mm: f64) -> Containment {
        let distance = norm(point);
        if distance <= tolerance_mm {
            return Containment::Outside;
        }
        let direction = scale(point, 1. / distance);
        let phi = point[1].atan2(point[0]);
        let (sp, cp) = self.spec.pitch_cone_angle_rad.sin_cos();
        let along = dot(point, [sp * phi.cos(), sp * phi.sin(), cp]);
        let mut margin = (self.end_distance_mm(End::Heel) - along)
            .min(along - self.end_distance_mm(End::Toe))
            .min(self.direction_margin(direction) * distance);
        if let Some(bore) = self.bore_radius_mm() {
            margin = margin.min(point[0].hypot(point[1]) - bore);
        }
        if margin > tolerance_mm {
            Containment::Inside
        } else if margin < -tolerance_mm {
            Containment::Outside
        } else {
            Containment::Boundary
        }
    }

    /// ∮ G(γ) dφ over the right half of sector 0, from the tooth center to
    /// the sector end, for a γ-only antiderivative `G` with `G(0) = 0`.
    fn half_sector_integral(&self, antiderivative: impl Fn(f64) -> f64) -> f64 {
        self.pieces
            .iter()
            .filter(|piece| !piece.left() && **piece != Piece::Tip)
            .map(|&piece| {
                integrate(
                    |s| {
                        let sample = self.sample(piece, s);
                        let [x, y, _] = sample.direction;
                        let [dx, dy, _] = sample.derivative;
                        antiderivative(sample.gamma) * (x * dy - y * dx) / (x * x + y * y)
                    },
                    0.,
                    1.,
                    8,
                )
            })
            .sum::<f64>()
            + antiderivative(self.spec.tip_cone_angle_rad) * self.half_tip
    }

    /// Exact solid volume in mm³ up to quadrature roundoff.
    pub fn volume_mm3(&self) -> f64 {
        let delta = self.spec.pitch_cone_angle_rad;
        let [heel, toe] = End::BOTH.map(|end| self.end_distance_mm(end));
        let k = (heel.powi(3) - toe.powi(3)) / 3.;
        // ∫ sin γ / cos³(γ - δ) dγ = cos δ / (2 cos²(γ-δ)) + sin δ tan(γ-δ).
        let primitive = |g: f64| {
            let x = g - delta;
            delta.cos() / (2. * x.cos().powi(2)) + delta.sin() * x.tan()
        };
        let zero = primitive(0.);
        let sector = 2. * self.half_sector_integral(|g| k * (primitive(g) - zero));
        let bore = self.bore_radius_mm().unwrap_or(0.);
        f64::from(self.spec.teeth) * sector
            - PI * bore * bore * self.spec.face_width_mm / delta.cos()
    }

    /// Area of one ruled side piece of one tooth.
    pub fn side_area_mm2(&self, piece: Piece) -> f64 {
        let delta = self.spec.pitch_cone_angle_rad;
        let [heel, toe] = End::BOTH.map(|end| self.end_distance_mm(end));
        integrate(
            |s| {
                let sample = self.sample(piece, s);
                (heel * heel - toe * toe) / (2. * (sample.gamma - delta).cos().powi(2))
                    * norm(sample.derivative)
            },
            0.,
            1.,
            8,
        )
    }

    /// Areas of the outer and optional inner cap sectors at one end.
    pub fn cap_areas_mm2(&self, end: End) -> (f64, Option<f64>) {
        let radius = |g: f64| self.end_radius(end, g);
        let half = |g: f64| radius(g).powi(2) / 2.;
        let outline = 2. * self.half_sector_integral(half);
        let sector = TAU / f64::from(self.spec.teeth);
        let inner = half(self.inner_gamma) * sector;
        let hole = self.bore_radius_mm().unwrap_or(0.).powi(2) / 2. * sector;
        let slope = self.spec.pitch_cone_angle_rad.cos();
        if self.has_core(end) {
            ((outline - inner) / slope, Some((inner - hole) / slope))
        } else {
            ((outline - hole) / slope, None)
        }
    }

    pub fn bore_area_mm2(&self) -> Option<f64> {
        self.bore_radius_mm()
            .map(|bore| TAU * bore * self.spec.face_width_mm / self.spec.pitch_cone_angle_rad.cos())
    }

    /// Maximum of `direction · p` over the local solid.
    pub fn support(&self, direction: Point) -> f64 {
        let mut best = f64::NEG_INFINITY;
        for end in End::BOTH {
            if let Some(bore) = self.bore_radius_mm() {
                best = best.max(
                    self.back_cone_height_mm(end, bore) * direction[2]
                        + bore * direction[0].hypot(direction[1]),
                );
            } else {
                best = best.max(self.back_cone_apex_mm(end) * direction[2]);
            }
            for sector in 0..self.spec.teeth {
                let turn = self.sector_start(sector) + PI / f64::from(self.spec.teeth);
                for &piece in &self.pieces {
                    let value = |s: f64| {
                        let sample = self.sample(piece, s);
                        dot(
                            rotate_z(self.point(end, sample.direction, sample.gamma), turn),
                            direction,
                        )
                    };
                    const SAMPLES: usize = 24;
                    let (index, _) = (0..=SAMPLES)
                        .map(|i| value(i as f64 / SAMPLES as f64))
                        .enumerate()
                        .fold(
                            (0, f64::NEG_INFINITY),
                            |acc, (i, v)| {
                                if v > acc.1 { (i, v) } else { acc }
                            },
                        );
                    let mut lo = index.saturating_sub(1) as f64 / SAMPLES as f64;
                    let mut hi = (index + 1).min(SAMPLES) as f64 / SAMPLES as f64;
                    for _ in 0..48 {
                        let a = lo + (hi - lo) * 0.381_966_011_250_105_1;
                        let b = hi - (hi - lo) * 0.381_966_011_250_105_1;
                        if value(a) < value(b) {
                            lo = a;
                        } else {
                            hi = b;
                        }
                    }
                    best = best
                        .max(value(lo))
                        .max(value(hi))
                        .max(value(index as f64 / SAMPLES as f64));
                }
            }
        }
        best
    }

    /// Sector and side piece whose cone through the apex carries a side-face
    /// point given in local coordinates. Root lands lie beyond the fillet
    /// center azimuth; along a flank the polar angle orders the pieces.
    pub fn side_piece_at(&self, point: Point) -> Option<(u32, Piece)> {
        let length = norm(point);
        let direction = scale(point, 1. / length);
        let gamma = direction[2].clamp(-1., 1.).acos();
        let teeth = self.spec.teeth;
        let pitch = TAU / f64::from(teeth);
        let phi = direction[1].atan2(direction[0]);
        let turns = (phi / pitch).round();
        let local = phi - turns * pitch;
        let sector = (turns as i64).rem_euclid(i64::from(teeth)) as u32;
        let left = local < 0.;
        let side = |l: Piece, r: Piece| if left { l } else { r };
        let piece = if local.abs() > self.center_phi {
            side(Piece::RootLeft, Piece::RootRight)
        } else if gamma <= self.touch_gamma {
            side(Piece::FilletLeft, Piece::FilletRight)
        } else if self.meridian && gamma <= self.spec.base_cone_angle_rad {
            side(Piece::MeridianLeft, Piece::MeridianRight)
        } else if gamma < self.spec.tip_cone_angle_rad - 1e-12 {
            side(Piece::InvoluteLeft, Piece::InvoluteRight)
        } else {
            Piece::Tip
        };
        (length > 0.).then_some((sector, piece))
    }

    /// Tooth center direction of one sector on the pitch cone.
    pub fn pitch_direction(&self, sector: u32) -> Point {
        polar(
            self.spec.pitch_cone_angle_rad,
            self.sector_start(sector) + PI / f64::from(self.spec.teeth),
        )
    }

    pub(crate) fn rotate(&self, sector: u32, p: Point) -> Point {
        rotate_z(
            p,
            self.sector_start(sector) + PI / f64::from(self.spec.teeth),
        )
    }
}

#[cfg(test)]
pub(crate) mod tests;
