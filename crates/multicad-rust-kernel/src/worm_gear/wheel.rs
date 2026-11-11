//! Generated ZA worm wheel as a stack of exact axial sections.
//!
//! In module units the native cutter is a worm of tip radius 6.25 at center
//! distance `5 + teeth/2`. Its generating patches give, at each wheel axial
//! coordinate `v`, one closed tooth-space curve: the generated flank on the
//! negative-angle side, a swept tip-edge transition, the circular root, the
//! mirrored transition and the positive-angle flank. The wheel section is
//! the blank disk of radius `teeth/2 + 1` less one rotated space per sector
//! and the optional bore. Sector `k` is centered on its space at angle
//! `2πk/teeth` and spans half a tooth on either side.
use super::{jet::Jet, jet::Real, *};
use crate::bevel_gear::integrate;
use std::f64::consts::{PI, TAU};

const LEAD: f64 = 0.5;
pub(crate) const CUTTER_TIP: f64 = 6.25;
pub(crate) const WORM_ROOT: f64 = 3.75;
const STATIONS: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelSpec {
    pub teeth: u32,
    pub axial_module_mm: f64,
    pub face_width_mm: f64,
    pub bore_radius_mm: f64,
    pub pitch_thinning_mm: f64,
}

/// Generated tooth-space pieces in counterclockwise order. The index is the
/// native generating patch kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Piece {
    FlankMinus,
    EdgeMinus,
    Torus,
    EdgePlus,
    FlankPlus,
}

pub const PIECES: [Piece; 5] = [
    Piece::FlankMinus,
    Piece::EdgeMinus,
    Piece::Torus,
    Piece::EdgePlus,
    Piece::FlankPlus,
];

impl Piece {
    pub fn role(self) -> &'static str {
        [
            "flank.minus",
            "root.edge.minus",
            "root.torus",
            "root.edge.plus",
            "flank.plus",
        ][self as usize]
    }

    pub(crate) fn flank_side(self) -> Option<f64> {
        match self {
            Self::FlankMinus => Some(1.),
            Self::FlankPlus => Some(-1.),
            _ => None,
        }
    }
}

/// Trim of the two flanks at one section by the blank cylinder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Section {
    pub(crate) v: f64,
    /// Cutter radii where the minus and plus flanks leave the blank.
    pub(crate) crossing: [f64; 2],
    /// Section angles of those exits in sector 0.
    pub(crate) exit: [f64; 2],
}

/// Derived data computed once per wheel. It never affects equality.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cache<T>(std::sync::OnceLock<T>);

impl<T> PartialEq for Cache<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Cache<T> {
    pub(crate) fn get(&self, compute: impl FnOnce() -> T) -> &T {
        self.0.get_or_init(compute)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wheel {
    spec: WheelSpec,
    /// Pitch radius, center distance, cutter half thickness and slope.
    radius: f64,
    center: f64,
    half: f64,
    slope: f64,
    span: f64,
    pub(crate) section_bounds: Cache<Option<[f64; 2]>>,
    pub(crate) remainders: Cache<Option<Box<[super::wheel_bounds::Remainder; 5]>>>,
}

pub(crate) fn turn<T: Real>([x, y]: [T; 2], angle: T) -> [T; 2] {
    let (s, c) = angle.sin_cos();
    [c * x - s * y, s * x + c * y]
}

impl Wheel {
    pub fn new(
        spec: WheelSpec,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
    ) -> Result<Self, Error> {
        let WheelSpec {
            teeth,
            axial_module_mm: m,
            face_width_mm: width,
            bore_radius_mm: bore,
            pitch_thinning_mm: thinning,
        } = spec;
        if ![m, width, bore, thinning, minimum_mm]
            .iter()
            .all(|v| v.is_finite())
            || !(0.5..=2.).contains(&m)
            || !(0. ..=0.1 * m).contains(&thinning)
            || bore < 0.
            || minimum_mm <= 0.
            || maximum_coordinate_mm.is_nan()
        {
            return Err(Error::InvalidDimensions);
        }
        if !(32..=64).contains(&teeth)
            || !(0.5 * m..=2. * m).contains(&width)
            || (f64::from(teeth) / 2. + 1.) * m > maximum_coordinate_mm
        {
            return Err(Error::Limit);
        }
        let radius = f64::from(teeth) / 2.;
        if bore > (radius - 1.25) * m - minimum_mm {
            return Err(Error::InvalidProfile);
        }
        let wheel = Self {
            spec,
            radius,
            center: 5. + radius,
            half: PI / 4. + thinning / (2. * m),
            slope: super::worm::slope(),
            span: PI / f64::from(teeth),
            section_bounds: Cache::default(),
            remainders: Cache::default(),
        };
        wheel.check(minimum_mm)?;
        Ok(wheel)
    }

    /// Verify the section assumptions at stations across the width.
    fn check(&self, minimum_mm: f64) -> Result<(), Error> {
        let m = self.spec.axial_module_mm;
        let blank = self.blank();
        let bore = self.spec.bore_radius_mm / m;
        let mut valid = true;
        for station in 0..STATIONS {
            let v = self.half_width() * (2. * station as f64 / (STATIONS - 1) as f64 - 1.);
            for side in [1., -1.] {
                let radii: Vec<_> = (0..=32)
                    .map(|i| {
                        let r = WORM_ROOT + (CUTTER_TIP - WORM_ROOT) * f64::from(i) / 32.;
                        let [x, y] = self.flank(r, v, side);
                        x.hypot(y)
                    })
                    .collect();
                valid &=
                    radii[0] > blank && radii[32] < blank && radii.windows(2).all(|w| w[1] < w[0]);
            }
            let section = self.section(v);
            let [low, high] = section.exit;
            let mut last = low;
            for piece in PIECES {
                for i in 0..=32 {
                    let u = Jet::variable(f64::from(i) / 32., 0);
                    let [x, y] = self.piece_point(&section, piece, u);
                    let angle = y.value.atan2(x.value);
                    let rate = x.value * y.d[0] - y.value * x.d[0];
                    valid &= rate > 0. && angle >= last - 1e-12 && x.value.hypot(y.value) > bore;
                    last = angle;
                }
            }
            let tip_land = (2. * self.span + low - high) * blank * m;
            let root = self.center - (CUTTER_TIP * CUTTER_TIP - v * v).sqrt();
            if !valid
                || low <= -self.span
                || high >= self.span
                || tip_land < minimum_mm
                || root - bore < minimum_mm / m
            {
                return Err(Error::InvalidProfile);
            }
        }
        Ok(())
    }

    pub fn spec(&self) -> WheelSpec {
        self.spec
    }

    pub fn teeth(&self) -> u32 {
        self.spec.teeth
    }

    pub fn module_mm(&self) -> f64 {
        self.spec.axial_module_mm
    }

    pub fn bore_radius_mm(&self) -> Option<f64> {
        (self.spec.bore_radius_mm > 0.).then_some(self.spec.bore_radius_mm)
    }

    /// Blank radius in module units.
    pub(crate) fn blank(&self) -> f64 {
        self.radius + 1.
    }

    /// Half the face width in module units.
    pub(crate) fn half_width(&self) -> f64 {
        self.spec.face_width_mm / (2. * self.spec.axial_module_mm)
    }

    /// Half the angular pitch: each sector spans `[-span, span]` about its space.
    pub(crate) fn span(&self) -> f64 {
        self.span
    }

    pub fn sector_angle(&self, sector: u32) -> f64 {
        2. * self.span * f64::from(sector)
    }

    /// Wheel radius of the circular root at section `v`.
    pub(crate) fn root_radius(&self, v: f64) -> f64 {
        self.center - (CUTTER_TIP * CUTTER_TIP - v * v).sqrt()
    }

    fn contact<T: Real>(&self, r: T, v: T, side: f64) -> [T; 3] {
        let c = T::constant;
        let q = (r * r - v * v).sqrt();
        let z = c(side) * (c(self.half) - c(self.slope) * (r - c(5.))) - c(LEAD) * (v / r).asin();
        let nx = c(LEAD) * v / r + c(-side * self.slope) * q;
        [q, z, r * (q - c(5.)) / nx]
    }

    pub(crate) fn flank<T: Real>(&self, r: T, v: T, side: f64) -> [T; 2] {
        let (q, angle) = self.flank_polar(r, v, side);
        turn(q, angle)
    }

    /// Flank point before its rotation about the wheel axis, and that angle.
    fn flank_polar<T: Real>(&self, r: T, v: T, side: f64) -> ([T; 2], T) {
        let [q, z, w] = self.contact(r, v, side);
        let c = T::constant;
        ([c(self.center) - q, w], -(z + w) / c(self.radius))
    }

    fn tip_edge<T: Real>(&self, u: T, v: T, side: f64) -> ([T; 2], T) {
        let c = T::constant;
        let [q, z, w] = self.contact(c(CUTTER_TIP), v, side);
        let travel = (c(1.) - u) * w;
        ([c(self.center) - q, travel], -(z + travel) / c(self.radius))
    }

    fn root_gap<T: Real>(&self, u: T, v: T) -> ([T; 2], T) {
        let c = T::constant;
        let q = (c(CUTTER_TIP * CUTTER_TIP) - v * v).sqrt();
        let crest = self.half - 1.25 * self.slope;
        let phase = c(LEAD / self.radius) * (v / c(CUTTER_TIP)).asin()
            + (c(2.) * u - c(1.)) * c(crest / self.radius);
        ([c(self.center) - q, c(0.)], phase)
    }

    /// Cutter radius where one flank leaves the blank at section `v`.
    pub(crate) fn crossing(&self, v: f64, side: f64) -> f64 {
        let (mut lo, mut hi) = (WORM_ROOT, CUTTER_TIP);
        for _ in 0..64 {
            let r = (lo + hi) / 2.;
            let [x, y] = self.flank(r, v, side);
            if x.hypot(y) > self.blank() {
                lo = r;
            } else {
                hi = r;
            }
        }
        (lo + hi) / 2.
    }

    pub(crate) fn section(&self, v: f64) -> Section {
        let crossing = [self.crossing(v, 1.), self.crossing(v, -1.)];
        let exit = [0, 1].map(|i| {
            let [x, y] = self.flank(crossing[i], v, [1., -1.][i]);
            y.atan2(x)
        });
        Section { v, crossing, exit }
    }

    /// Exact section point of sector 0, in module units, with `u` in `[0, 1]`
    /// increasing counterclockwise.
    pub(crate) fn piece_point<T: Real>(&self, section: &Section, piece: Piece, u: T) -> [T; 2] {
        let crossing = section.crossing[usize::from(piece == Piece::FlankPlus)];
        self.patch(piece, u, T::constant(section.v), T::constant(crossing))
    }

    /// Generating patch of sector 0 over `(u, v)` in `[0, 1] × [-h, h]`.
    /// `crossing` is the cutter radius where this piece's flank leaves the
    /// blank at `v`; other pieces ignore it. Flanks run linearly in cutter
    /// radius between that exit and the cutter tip.
    pub(crate) fn patch<T: Real>(&self, piece: Piece, u: T, v: T, crossing: T) -> [T; 2] {
        let (q, angle) = self.polar_patch(piece, u, v, crossing);
        turn(q, angle)
    }

    /// Patch point before its rotation about the wheel axis, and that angle.
    pub(crate) fn polar_patch<T: Real>(
        &self,
        piece: Piece,
        u: T,
        v: T,
        crossing: T,
    ) -> ([T; 2], T) {
        let c = T::constant;
        match piece {
            Piece::FlankMinus => self.flank_polar(crossing + (c(CUTTER_TIP) - crossing) * u, v, 1.),
            Piece::FlankPlus => {
                self.flank_polar(c(CUTTER_TIP) - (c(CUTTER_TIP) - crossing) * u, v, -1.)
            }
            Piece::EdgeMinus => self.tip_edge(u, v, 1.),
            Piece::Torus => self.root_gap(u, v),
            Piece::EdgePlus => self.tip_edge(c(1.) - u, v, -1.),
        }
    }

    /// Surface point over `(a, v)`: the cutter radius for flanks, else the
    /// profile parameter. Flank domains are trimmed at `crossing`.
    pub(crate) fn surface_point<T: Real>(&self, piece: Piece, a: T, v: T) -> [T; 2] {
        let (q, angle) = match piece {
            Piece::FlankMinus => self.flank_polar(a, v, 1.),
            Piece::FlankPlus => self.flank_polar(a, v, -1.),
            Piece::EdgeMinus => self.tip_edge(a, v, 1.),
            Piece::Torus => self.root_gap(a, v),
            Piece::EdgePlus => self.tip_edge(T::constant(1.) - a, v, -1.),
        };
        turn(q, angle)
    }

    /// Integrate `f(point, area element)` over one side piece of sector 0
    /// in module units.
    fn side_integral<const N: usize>(
        &self,
        piece: Piece,
        f: impl Fn([f64; 3], f64) -> [f64; N],
    ) -> [f64; N] {
        let h = self.half_width();
        let component = |k: usize| {
            integrate(
                |v| {
                    let (start, length) = if piece.flank_side().is_some() {
                        let section = self.section(v);
                        if piece == Piece::FlankMinus {
                            (section.crossing[0], CUTTER_TIP - section.crossing[0])
                        } else {
                            (section.crossing[1], CUTTER_TIP - section.crossing[1])
                        }
                    } else {
                        (0., 1.)
                    };
                    length
                        * integrate(
                            |t| {
                                let a = Jet::variable(start + length * t, 0);
                                let [x, y] = self.surface_point(piece, a, Jet::variable(v, 1));
                                let element = (x.d[0] * x.d[0]
                                    + y.d[0] * y.d[0]
                                    + (x.d[0] * y.d[1] - y.d[0] * x.d[1]).powi(2))
                                .sqrt();
                                f([x.value, y.value, v], element)[k]
                            },
                            0.,
                            1.,
                            4,
                        )
                },
                -h,
                h,
                2,
            )
        };
        std::array::from_fn(component)
    }

    /// Material area of sector 0 at a section and its first moments.
    fn sector_section(&self, v: f64) -> [f64; 3] {
        let section = self.section(v);
        let blank = self.blank();
        let bore = self.spec.bore_radius_mm / self.spec.axial_module_mm;
        let [low, high] = section.exit;
        let arc = |radius: f64, a: f64, b: f64| {
            let r3 = radius.powi(3) / 3.;
            [
                radius * radius * (b - a) / 2.,
                r3 * (b.sin() - a.sin()),
                r3 * (a.cos() - b.cos()),
            ]
        };
        let mut total = [0.; 3];
        for part in [
            arc(blank, -self.span, low),
            arc(blank, high, self.span),
            arc(bore, -self.span, self.span).map(|x| -x),
        ] {
            for i in 0..3 {
                total[i] += part[i];
            }
        }
        for piece in PIECES {
            for (i, slot) in total.iter_mut().enumerate() {
                *slot += integrate(
                    |u| {
                        let [x, y] = self.piece_point(&section, piece, Jet::variable(u, 0));
                        let twice = x.value * y.d[0] - y.value * x.d[0];
                        [twice / 2., x.value * twice / 3., y.value * twice / 3.][i]
                    },
                    0.,
                    1.,
                    8,
                );
            }
        }
        total
    }

    pub fn volume_mm3(&self) -> f64 {
        let h = self.half_width();
        let sector = integrate(|v| self.sector_section(v)[0], -h, h, 2);
        f64::from(self.spec.teeth) * sector * self.spec.axial_module_mm.powi(3)
    }

    /// Faces of sector 0 in topology slot order, then the bore.
    pub(crate) fn sector_faces(&self) -> (Vec<(f64, Point)>, Option<f64>) {
        let m = self.spec.axial_module_mm;
        let h = self.half_width();
        let mut faces = Vec::with_capacity(9);
        for v in [-h, h] {
            let [area, x, y] = self.sector_section(v);
            faces.push((area * m * m, [x / area * m, y / area * m, v * m]));
        }
        for piece in PIECES {
            let [area, x, y, z] =
                self.side_integral(piece, |[x, y, v], e| [e, x * e, y * e, v * e]);
            faces.push((area * m * m, [x / area * m, y / area * m, z / area * m]));
        }
        let blank = self.blank();
        for tip in 0..2 {
            let bounds = |v: f64| {
                let exit = self.section(v).exit;
                if tip == 0 {
                    (-self.span, exit[0])
                } else {
                    (exit[1], self.span)
                }
            };
            let [area, x, y, z] = std::array::from_fn(|k| {
                integrate(
                    |v| {
                        let (a, b) = bounds(v);
                        [
                            blank * (b - a),
                            blank * blank * (b.sin() - a.sin()),
                            blank * blank * (a.cos() - b.cos()),
                            blank * (b - a) * v,
                        ][k]
                    },
                    -h,
                    h,
                    2,
                )
            });
            faces.push((area * m * m, [x / area * m, y / area * m, z / area * m]));
        }
        let bore = self
            .bore_radius_mm()
            .map(|bore| TAU * bore * self.spec.face_width_mm);
        (faces, bore)
    }

    /// Radial material margin of a local point in module units, from the
    /// space boundary at its own section.
    fn space_margin(&self, x: f64, y: f64, v: f64) -> f64 {
        let theta = y.atan2(x);
        let pitch = 2. * self.span;
        let local = theta - (theta / pitch).round() * pitch;
        let radius = x.hypot(y);
        let section = self.section(v);
        let [low, high] = section.exit;
        if local <= low || local >= high {
            return self.blank() - radius;
        }
        let angle = |piece: Piece, u: f64| {
            let [x, y] = self.piece_point(&section, piece, u);
            y.atan2(x)
        };
        // The exit angle bounds the last piece.
        let piece = PIECES
            .into_iter()
            .find(|&piece| angle(piece, 1.) >= local)
            .unwrap_or(Piece::FlankPlus);
        {
            let (mut lo, mut hi) = (0., 1.);
            for _ in 0..60 {
                let mid = (lo + hi) / 2.;
                if angle(piece, mid) < local {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let [px, py] = self.piece_point(&section, piece, (lo + hi) / 2.);
            px.hypot(py) - radius
        }
    }

    /// Signed material margin of a local point in mm.
    pub(crate) fn margin(&self, [x, y, z]: Point) -> f64 {
        let m = self.spec.axial_module_mm;
        let radius = x.hypot(y);
        let mut margin = (self.spec.face_width_mm / 2. - z.abs()).min(self.blank() * m - radius);
        if let Some(bore) = self.bore_radius_mm() {
            margin = margin.min(radius - bore);
        }
        if margin <= 0. {
            return margin;
        }
        margin.min(self.space_margin(x / m, y / m, z / m) * m)
    }

    pub fn classify(&self, point: Point, tolerance_mm: f64) -> Containment {
        let margin = self.margin(point);
        if margin > tolerance_mm {
            Containment::Inside
        } else if margin < -tolerance_mm {
            Containment::Outside
        } else {
            Containment::Boundary
        }
    }

    /// Maximum of `direction · p` over one section, in module units.
    fn section_support(&self, v: f64, [dx, dy]: [f64; 2]) -> f64 {
        let section = self.section(v);
        let reach = dx.hypot(dy);
        let phi = dy.atan2(dx);
        let blank = self.blank();
        let arc = |a: f64, b: f64| {
            let offset = (phi - a).rem_euclid(TAU);
            if offset <= b - a {
                1_f64
            } else {
                (a - phi).cos().max((b - phi).cos())
            }
        };
        let [low, high] = section.exit;
        let mut best = f64::NEG_INFINITY;
        for sector in 0..self.spec.teeth {
            let turn = self.sector_angle(sector);
            best = best
                .max(blank * reach * arc(turn - self.span, turn + low))
                .max(blank * reach * arc(turn + high, turn + self.span));
        }
        for sector in 0..self.spec.teeth {
            let turn = self.sector_angle(sector);
            if blank * reach * arc(turn + low, turn + high) <= best {
                continue;
            }
            for piece in PIECES {
                let value = |u: f64| {
                    let [x, y] = self.piece_point(&section, piece, u);
                    let (s, c) = turn.sin_cos();
                    dx * (c * x - s * y) + dy * (s * x + c * y)
                };
                best = best.max(maximize(value, 0., 1.));
            }
        }
        best
    }

    /// Maximum of `direction · p` over the local solid in mm.
    pub fn support(&self, direction: Point) -> f64 {
        let m = self.spec.axial_module_mm;
        let h = self.half_width();
        let [dx, dy, dz] = direction;
        if dx == 0. && dy == 0. {
            return h * dz.abs() * m;
        }
        let value = |v: f64| self.section_support(v, [dx, dy]) + dz * v;
        maximize(value, -h, h) * m
    }

    /// End-plane height in module units: start at `-h`, end at `h`.
    pub(crate) fn end_v(&self, end: usize) -> f64 {
        if end == 0 {
            -self.half_width()
        } else {
            self.half_width()
        }
    }

    /// Joint `j` of sector 0 at section `v`: 0 at the sector start on the
    /// blank, 1..=6 the piece ends from the minus exit to the plus exit.
    pub(crate) fn joint(&self, section: &Section, joint: usize) -> [f64; 2] {
        match joint {
            0 => [
                self.blank() * self.span.cos(),
                -self.blank() * self.span.sin(),
            ],
            6 => self.piece_point(section, Piece::FlankPlus, 1.),
            j => self.piece_point(section, PIECES[j - 1], 0.),
        }
    }
}

/// Sampled maximum with a golden-section refinement around the best sample.
pub(crate) fn maximize(value: impl Fn(f64) -> f64, from: f64, to: f64) -> f64 {
    const SAMPLES: usize = 24;
    let at = |i: usize| from + (to - from) * i as f64 / SAMPLES as f64;
    let (index, sampled) = (0..=SAMPLES).map(|i| value(at(i))).enumerate().fold(
        (0, f64::NEG_INFINITY),
        |acc, (i, v)| if v > acc.1 { (i, v) } else { acc },
    );
    let mut lo = at(index.saturating_sub(1));
    let mut hi = at((index + 1).min(SAMPLES));
    for _ in 0..48 {
        let a = lo + (hi - lo) * 0.381_966_011_250_105_1;
        let b = hi - (hi - lo) * 0.381_966_011_250_105_1;
        if value(a) < value(b) {
            lo = a;
        } else {
            hi = b;
        }
    }
    sampled.max(value(lo)).max(value(hi))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn spec(m: f64, teeth: u32, width: f64, bore: f64, thinning: f64) -> WheelSpec {
        WheelSpec {
            teeth,
            axial_module_mm: m,
            face_width_mm: width * m,
            bore_radius_mm: bore,
            pitch_thinning_mm: thinning,
        }
    }

    pub(crate) fn wheel(spec: WheelSpec) -> Wheel {
        Wheel::new(spec, 1e-3, 1e7).unwrap()
    }

    #[test]
    fn pieces_join_continuously_and_close_against_the_blank() {
        let wheel = wheel(spec(1., 40, 1.5, 4., 0.05));
        for v in [-0.75, 0., 0.4, 0.75] {
            let section = wheel.section(v);
            for side in 0..2 {
                let [x, y] = wheel.flank(section.crossing[side], v, [1., -1.][side]);
                assert!((x.hypot(y) - wheel.blank()).abs() < 1e-12);
            }
            let mut previous = wheel.joint(&section, 1);
            for (j, piece) in PIECES.into_iter().enumerate() {
                let start = wheel.piece_point(&section, piece, 0.);
                assert!(
                    (start[0] - previous[0]).hypot(start[1] - previous[1]) < 1e-12,
                    "{j}"
                );
                previous = wheel.piece_point(&section, piece, 1.);
            }
            let root = wheel.piece_point(&section, Piece::Torus, 0.3);
            assert!((root[0].hypot(root[1]) - wheel.root_radius(v)).abs() < 1e-12);
            let tip = wheel.joint(&section, 0);
            assert!((tip[0].hypot(tip[1]) - wheel.blank()).abs() < 1e-12);
        }
        assert_eq!(Piece::Torus.role(), "root.torus");
        assert_eq!(wheel.teeth(), 40);
        assert_eq!(wheel.module_mm(), 1.);
        assert_eq!(wheel.spec().bore_radius_mm, 4.);
    }

    #[test]
    fn section_areas_and_volume_match_dense_polygons() {
        let wheel = wheel(spec(1., 32, 2., 3., 0.03));
        let h = wheel.half_width();
        let polygon = |v: f64| {
            let section = wheel.section(v);
            let mut points = vec![];
            let n = 4000;
            for p in 0..7 {
                for i in 0..n {
                    points.push(wheel.chain_point(&section, p, i as f64 / n as f64));
                }
            }
            points.push(wheel.chain_point(&section, 6, 1.));
            // Close through the bore arc back to the sector start.
            let bore = 3.;
            let span = wheel.span();
            for i in 0..=n {
                let t = span - 2. * span * i as f64 / n as f64;
                points.push([bore * t.cos(), bore * t.sin()]);
            }
            let area: f64 = (0..points.len())
                .map(|i| {
                    let [a, b] = [points[i], points[(i + 1) % points.len()]];
                    a[0] * b[1] - a[1] * b[0]
                })
                .sum::<f64>()
                / 2.;
            area
        };
        let rows = 16;
        let mut total = 0.;
        for i in 0..rows {
            let v = -h + 2. * h * (i as f64 + 0.5) / rows as f64;
            let exact = wheel.sector_section(v)[0];
            let dense = polygon(v);
            assert!((exact - dense).abs() / exact < 1e-6, "{exact} {dense}");
            total += exact * 2. * h / rows as f64;
        }
        let volume = wheel.volume_mm3();
        // The midpoint rule over 16 rows carries its own O(h²) error.
        assert!(
            (volume - 32. * total).abs() / volume < 1e-4,
            "{volume} {total}"
        );
    }
}
