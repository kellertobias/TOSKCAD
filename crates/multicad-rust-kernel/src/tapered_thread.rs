//! Retained legacy Custom cone-spine transport, distinct from axial pipe threads.
//!
//! The source section starts at `-lead` and uses OCCT's fixed axial binormal.
//! Its transported horizontal heading therefore changes along a conical spine.
//! NPT's axial-flat and BSPT's meridian laws are deliberately outside this API.
use crate::helical_thread::{Spec, Thread as StraightThread, ThreadError};
mod bivariate;
pub(crate) mod interval;
mod law;
mod mass;
mod mesh;
mod series;
mod support;
mod tensor;
use interval::{I, J};
pub use mesh::{tessellate, tessellate_with_interrupt};
pub use tensor::{
    MANUFACTURING_DEGREE, ManufacturingTensor, ManufacturingTensors, TaylorEnclosure,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unsupported,
    Unresolved,
    Limit,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thread {
    source: StraightThread,
    taper: f64,
    tolerance: f64,
    minimum: f64,
    feet: [f64; 2],
    slopes: [f64; 2],
    window: f64,
}
#[derive(Clone, Copy)]
struct Transport {
    phase: J,
    radius: J,
    excess: J,
}
struct Work<'a> {
    remaining: usize,
    interrupt: Option<&'a dyn Fn() -> bool>,
}
impl Work<'_> {
    fn step(&mut self) -> Result<(), Error> {
        if self.interrupt.is_some_and(|f| f()) {
            return Err(Error::Cancelled);
        }
        self.remaining = self.remaining.checked_sub(1).ok_or(Error::Limit)?;
        Ok(())
    }
}
impl Thread {
    pub fn new(spec: Spec, taper: f64, tolerance: f64, minimum: f64) -> Result<Self, Error> {
        Self::new_with_interrupt(spec, taper, tolerance, minimum, None)
    }
    pub fn new_with_interrupt(
        spec: Spec,
        taper: f64,
        tolerance: f64,
        minimum: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        if !taper.is_finite()
            || taper.abs() > 0.1
            || !tolerance.is_finite()
            || tolerance <= 0.
            || !minimum.is_finite()
            || minimum <= 0.
        {
            return Err(Error::Invalid);
        }
        if spec.root_radius_mm.is_some() {
            return Err(Error::Unsupported);
        }
        let source = StraightThread::new(spec).map_err(|e| match e {
            ThreadError::Invalid => Error::Invalid,
            ThreadError::Limit => Error::Limit,
        })?;
        let lead = source.lead_mm();
        let root = source.root_radius_mm();
        // Native construction extends the root cone and tool on both sides.
        if [
            root - 2. * taper * lead,
            root + taper * (spec.length_mm + 2. * lead),
        ]
        .into_iter()
        .any(|r| r < minimum || r > 1e7)
        {
            return Err(Error::Invalid);
        }
        let overlap = (0.02 * spec.pitch_mm).max(64. * tolerance);
        if [
            spec.major_radius_mm,
            spec.pitch_mm,
            spec.length_mm,
            spec.depth_mm,
            spec.crest_width_mm,
        ]
        .into_iter()
        .any(|v| v < minimum)
            || spec.crest_width_mm < 100. * tolerance
            || root - overlap < minimum
        {
            return Err(Error::Invalid);
        }
        for z in [-2. * lead, spec.length_mm + 2. * lead] {
            if z.abs() > 1e7
                || root + taper * z - overlap < minimum
                || spec.major_radius_mm + taper * z > 1e7
            {
                return Err(Error::Invalid);
            }
        }
        let slopes = [spec.lower_flank_deg, spec.upper_flank_deg].map(|a| a.to_radians().tan());
        let feet = [
            -spec.crest_width_mm / 2. - (spec.depth_mm + overlap) * slopes[0],
            spec.crest_width_mm / 2. + (spec.depth_mm + overlap) * slopes[1],
        ];
        if feet[1] - feet[0] >= spec.pitch_mm - minimum {
            return Err(Error::Invalid);
        }
        let thread = Self {
            source,
            taper,
            tolerance,
            minimum,
            feet,
            slopes,
            window: (feet[0] + feet[1] - spec.pitch_mm) * 0.5,
        };
        let mut work = Work {
            remaining: 32768,
            interrupt,
        };
        // Prove one-to-one angular transport and exactly two cone crossings.
        for (piece, range) in thread.ranges().into_iter().enumerate() {
            thread.prove(piece, range, I::new(0., spec.length_mm), 0, &mut work)?;
        }
        for (d, crest) in [
            (feet[0], false),
            (feet[1], false),
            (-spec.crest_width_mm / 2., true),
            (spec.crest_width_mm / 2., true),
        ] {
            thread.prove_station(d, crest, I::new(0., spec.length_mm), 0, &mut work)?;
        }
        Ok(thread)
    }
    pub fn spec(&self) -> Spec {
        self.source.spec()
    }
    pub fn radial_taper(&self) -> f64 {
        self.taper
    }
    pub fn tolerance_mm(&self) -> f64 {
        self.tolerance
    }
    pub fn minimum_feature_mm(&self) -> f64 {
        self.minimum
    }
    pub fn profile_stations(&self) -> [f64; 4] {
        let c = self.spec().crest_width_mm / 2.;
        [self.feet[0], -c, c, self.feet[1]]
    }
    pub fn root_radius_at(&self, z: f64) -> Result<[f64; 2], Error> {
        if !z.is_finite() || !(0. ..=self.spec().length_mm).contains(&z) {
            return Err(Error::Invalid);
        }
        let r = I::point(self.source.root_radius_mm()) + I::point(self.taper) * I::point(z);
        Ok([r.lo, r.hi])
    }
    fn ranges(&self) -> [I; 3] {
        let c = self.spec().crest_width_mm / 2.;
        [
            I::new(self.feet[0], -c),
            I::new(-c, c),
            I::new(c, self.feet[1]),
        ]
    }
    fn piece(&self, d: f64) -> usize {
        let c = self.spec().crest_width_mm / 2.;
        if d < -c {
            0
        } else if d > c {
            2
        } else {
            1
        }
    }
    fn transport(&self, piece: usize, d: I, z: I) -> Transport {
        self.transport_j(piece, J::d(d), J::z(z))
    }
    fn transport_j(&self, piece: usize, d: J, z: J) -> Transport {
        let p = law::evaluate(self, piece, d, z);
        Transport {
            phase: d - J::constant(self.source.helical_rate()) * (p.im / p.re).atan(),
            radius: p.radius_squared.sqrt(),
            excess: p.excess,
        }
    }
    fn prove(&self, piece: usize, d: I, z: I, depth: u32, w: &mut Work<'_>) -> Result<(), Error> {
        w.step()?;
        let t = self.transport(piece, d, z);
        let crossing =
            piece == 1 || (piece == 0 && t.excess.d.lo > 0.) || (piece == 2 && t.excess.d.hi < 0.);
        if [
            t.phase.v, t.phase.d, t.phase.z, t.radius.v, t.radius.d, t.radius.z, t.excess.v,
            t.excess.d, t.excess.z,
        ]
        .into_iter()
        .all(I::valid)
            && t.phase.d.lo > 0.
            && t.radius.v.lo > self.minimum
            && crossing
        {
            return Ok(());
        }
        if depth >= 24 {
            return Err(Error::Unresolved);
        }
        if (d.hi - d.lo) / self.spec().pitch_mm > (z.hi - z.lo) / self.spec().length_mm {
            let m = d.midpoint();
            self.prove(piece, I::new(d.lo, m), z, depth + 1, w)?;
            self.prove(piece, I::new(m, d.hi), z, depth + 1, w)
        } else {
            let m = z.midpoint();
            self.prove(piece, d, I::new(z.lo, m), depth + 1, w)?;
            self.prove(piece, d, I::new(m, z.hi), depth + 1, w)
        }
    }
    fn prove_station(
        &self,
        d: f64,
        crest: bool,
        z: I,
        depth: u32,
        w: &mut Work<'_>,
    ) -> Result<(), Error> {
        w.step()?;
        let t = self.transport(self.piece(d), I::point(d), z);
        let buried = if crest {
            t.excess.v.lo > 0.
        } else {
            t.excess.v.hi < 0.
        };
        if buried && t.phase.v.lo > self.window && t.phase.v.hi < self.window + self.spec().pitch_mm
        {
            return Ok(());
        }
        if depth >= 24 {
            return Err(Error::Unresolved);
        }
        let m = z.midpoint();
        self.prove_station(d, crest, I::new(z.lo, m), depth + 1, w)?;
        self.prove_station(d, crest, I::new(m, z.hi), depth + 1, w)
    }
    fn root_station(&self, piece: usize, z: I) -> Result<J, Error> {
        let mut range = self.ranges()[piece];
        for _ in 0..32 {
            let derivative = self.transport(piece, range, z).excess.d;
            if derivative.lo <= 0. && derivative.hi >= 0. {
                return Err(Error::Unresolved);
            }
            let middle = range.midpoint();
            let next =
                I::point(middle) - self.transport(piece, I::point(middle), z).excess.v / derivative;
            let narrowed = I::new(range.lo.max(next.lo), range.hi.min(next.hi));
            narrowed.checked()?;
            if narrowed.hi - narrowed.lo >= 0.99 * (range.hi - range.lo) {
                break;
            }
            range = narrowed;
        }
        let t = self.transport(piece, range, z);
        let derivative = -t.excess.z / t.excess.d;
        derivative.checked()?;
        let second = -(t.excess.zz
            + I::point(2.) * t.excess.dz * derivative
            + t.excess.dd * derivative.square())
            / t.excess.d;
        Ok(J {
            v: range,
            z: derivative,
            zz: second,
            ..J::constant(0.)
        })
    }
    /// Enclosure of one cone/flank shared seam in source coordinates.
    pub fn seam_point_bounds(&self, positive: bool, z: f64) -> Result<[[f64; 2]; 3], Error> {
        self.root_radius_at(z)?;
        Ok(self
            .root_point(if positive { 2 } else { 0 }, I::point(z))?
            .map(|p| [p.v.lo, p.v.hi]))
    }
    /// Enclosure of a point on a complete retained smooth wall patch.
    /// Bands are lower flank, crest, upper flank and exposed root cone.
    pub fn surface_point_bounds(
        &self,
        band: usize,
        parameter: f64,
        z: f64,
    ) -> Result<[[f64; 2]; 3], Error> {
        if band >= 4 || !parameter.is_finite() || !(0. ..=1.).contains(&parameter) {
            return Err(Error::Invalid);
        }
        self.root_radius_at(z)?;
        Ok(self
            .patch(band, I::point(parameter), I::point(z))?
            .map(|p| [p.v.lo, p.v.hi]))
    }
    fn root_point(&self, piece: usize, z: I) -> Result<[J; 3], Error> {
        let zz = J::z(z);
        let phase = self
            .transport_j(piece, self.root_station(piece, z)?, zz)
            .phase;
        let radius = J::constant(self.source.root_radius_mm()) + J::constant(self.taper) * zz;
        let theta = (zz - phase) / J::constant(self.source.helical_rate());
        Ok([radius * theta.cos(), radius * theta.sin(), zz])
    }
    fn patch(&self, band: usize, t: I, z: I) -> Result<[J; 3], Error> {
        let [radius, theta] = self.polar_patch(band, t, z)?;
        Ok([radius * theta.cos(), radius * theta.sin(), J::z(z)])
    }
    fn roots(&self, z: I) -> Result<[J; 2], Error> {
        Ok([self.root_station(0, z)?, self.root_station(2, z)?])
    }
    fn polar_patch(&self, band: usize, t: I, z: I) -> Result<[J; 2], Error> {
        self.polar_patch_roots(band, t, z, self.roots(z)?)
    }
    fn polar_patch_roots(&self, band: usize, t: I, z: I, roots: [J; 2]) -> Result<[J; 2], Error> {
        let zz = J::z(z);
        let tt = J::d(t);
        let half = self.spec().crest_width_mm / 2.;
        let (phase, radius) = match band {
            0 | 2 => {
                let root = roots[usize::from(band == 2)];
                let crest = J::constant(if band == 0 { -half } else { half });
                let d = if band == 0 {
                    root + (crest - root) * tt
                } else {
                    crest + (root - crest) * tt
                };
                let p = self.transport_j(if band == 0 { 0 } else { 2 }, d, zz);
                (p.phase, p.radius)
            }
            1 => {
                let p = self.transport_j(1, J::constant(-half) + J::constant(2. * half) * tt, zz);
                (p.phase, p.radius)
            }
            _ => {
                let lower =
                    self.transport_j(0, roots[0], zz).phase + J::constant(self.spec().pitch_mm);
                let upper = self.transport_j(2, roots[1], zz).phase;
                (
                    upper + (lower - upper) * tt,
                    J::constant(self.source.root_radius_mm()) + J::constant(self.taper) * zz,
                )
            }
        };
        let theta = (zz - phase) / J::constant(self.source.helical_rate());
        Ok([radius, theta])
    }
    /// Certified radial enclosure at the original axial phase in `[0,pitch)`.
    /// The result includes cone material and every retained helical start.
    pub fn radial_bounds(&self, phase: f64, z: f64, accuracy: f64) -> Result<[f64; 2], Error> {
        self.radial_bounds_with_interrupt(phase, z, accuracy, None)
    }
    pub fn radial_bounds_with_interrupt(
        &self,
        phase: f64,
        z: f64,
        accuracy: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<[f64; 2], Error> {
        let s = self.spec();
        if !phase.is_finite()
            || !(0. ..s.pitch_mm).contains(&phase)
            || !accuracy.is_finite()
            || accuracy <= 0.
        {
            return Err(Error::Invalid);
        }
        let [root_lo, root_hi] = self.root_radius_at(z)?;
        if I::new(root_lo, root_hi).width() > accuracy {
            return Err(Error::Unresolved);
        }
        let mut u = I::point(phase);
        if phase > self.window + s.pitch_mm {
            u = u - I::point(s.pitch_mm)
        }
        let zz = I::point(z);
        let mut w = Work {
            remaining: 256,
            interrupt,
        };
        w.step()?;
        let left = self.transport(0, I::point(self.feet[0]), zz).phase.v;
        let right = self.transport(2, I::point(self.feet[1]), zz).phase.v;
        if u.hi < left.lo || u.lo > right.hi {
            return Ok([root_lo, root_hi]);
        }
        let mut a = self.feet[0];
        let mut b = self.feet[1];
        for _ in 0..128 {
            w.step()?;
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for (piece, r) in self.ranges().into_iter().enumerate() {
                let d = I::new(a.max(r.lo), b.min(r.hi));
                if d.lo <= d.hi {
                    let rr = self.transport(piece, d, zz).radius.v;
                    lo = lo.min(rr.lo);
                    hi = hi.max(rr.hi)
                }
            }
            let result = [lo.max(root_lo), hi.max(root_hi)];
            if I::new(result[0], result[1]).width() <= accuracy {
                return Ok(result);
            }
            let m = I::new(a, b).split()?[0].hi;
            let t = self.transport(self.piece(m), I::point(m), zz);
            if t.phase.v.hi < u.lo {
                a = m
            } else if t.phase.v.lo > u.hi {
                b = m
            } else {
                // Strict monotonicity turns phase uncertainty into a certified
                // station interval, rather than guessing a bisection side.
                let mut derivative = f64::INFINITY;
                for (piece, r) in self.ranges().into_iter().enumerate() {
                    let interval = I::new(a.max(r.lo), b.min(r.hi));
                    if interval.lo <= interval.hi {
                        derivative = derivative.min(self.transport(piece, interval, zz).phase.d.lo)
                    }
                }
                I::point(derivative).positive()?;
                let width = ((t.phase.v - u) / I::point(derivative)).magnitude();
                a = a.max((m - width).next_down());
                b = b.min((m + width).next_up());
            }
        }
        Err(Error::Limit)
    }
}
#[cfg(test)]
mod tests;
