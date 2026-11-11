//! Interference of a placed worm and generated wheel with crossed axes.
//!
//! The worm boundary is covered by parameter rectangles. Each rectangle is
//! cleared when a lower bound on the distance from its center to the wheel
//! exceeds the rectangle's image radius. A cleared boundary and one wheel
//! point outside the worm prove disjoint solids. A rectangle center inside
//! the wheel material is moved into the worm; when both exact memberships
//! hold strictly the point is a common interior witness.
//!
//! The wheel distance bound uses exact boundary nodes at axial stations. The
//! section boundary moves at most `lipschitz` per unit height between
//! stations, so a planar gap `d` bounds the spatial gap by
//! `d / sqrt(1 + lipschitz²)`. Both the motion constant and the chord
//! sagitta come from interval Taylor bounds of the patch derivatives over
//! the whole patch domain (`wheel_bounds.rs`): a chord between profile
//! parameters `Δ` apart deviates from its arc by at most
//! `Δ²/8 · max |∂²P/∂u²|`.
use super::wheel::{PIECES, Wheel};
use super::worm::Worm;
use super::*;
pub use crate::bevel_gear::clearance::{ResultKind, Witness};
use std::f64::consts::TAU;

const STATIONS: usize = 128;
const NODES: usize = 96;
const MAX_RECTANGLES: usize = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub origin_mm: Point,
    /// World images of the local x, y and z axes.
    pub axes: [Point; 3],
}

impl Placement {
    fn world(&self, local: Point) -> Point {
        std::array::from_fn(|i| {
            self.origin_mm[i]
                + self.axes[0][i] * local[0]
                + self.axes[1][i] * local[1]
                + self.axes[2][i] * local[2]
        })
    }

    fn local(&self, world: Point) -> Point {
        let d = sub(world, self.origin_mm);
        self.axes.map(|axis| dot(axis, d))
    }

    fn valid(&self) -> bool {
        let [x, y, z] = self.axes;
        self.origin_mm.iter().all(|v| v.is_finite())
            && self.axes.iter().all(|a| (norm(*a) - 1.).abs() <= 1e-9)
            && dot(x, y).abs() <= 1e-9
            && dot(y, z).abs() <= 1e-9
            && dot(x, z).abs() <= 1e-9
            && norm(sub(cross(x, y), z)) <= 1e-9
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Limit,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Certificate {
    pub result: ResultKind,
    pub nearest: Option<Witness>,
}

/// Exact space-boundary nodes of sector 0 at axial stations.
struct Field<'a> {
    wheel: &'a Wheel,
    stations: Vec<f64>,
    nodes: Vec<Vec<[f64; 2]>>,
    /// Section boundary motion per unit height and chord sagitta, module units.
    lipschitz: f64,
    chord: f64,
}

enum Probe {
    /// Lower bound on the distance to the wheel in mm, with the nearest
    /// space-boundary node when the point lies in a tooth space.
    Clear(f64, Option<(usize, usize, u32)>),
    Material,
}

impl<'a> Field<'a> {
    fn new(wheel: &'a Wheel) -> Result<Self, Error> {
        let h = wheel.half_width();
        let stations: Vec<f64> = (0..=STATIONS)
            .map(|j| -h + 2. * h * j as f64 / STATIONS as f64)
            .collect();
        let chain = |v: f64, n: usize| -> Vec<[f64; 2]> {
            let section = wheel.section(v);
            let mut points = vec![];
            for piece in PIECES {
                for i in 0..n {
                    points.push(wheel.piece_point(&section, piece, i as f64 / n as f64));
                }
            }
            points.push(wheel.piece_point(&section, PIECES[4], 1.));
            points
        };
        let nodes: Vec<_> = stations.iter().map(|&v| chain(v, NODES)).collect();
        // Proven derivative bounds: every node moves at most `motion` per
        // unit height, and a chord of profile parameter length `1/NODES`
        // stays within `bend / (8 NODES²)` of its arc.
        let [motion, bend, _] = wheel.section_bounds().ok_or(Error::Limit)?;
        Ok(Self {
            wheel,
            stations,
            nodes,
            lipschitz: motion,
            chord: bend / (8. * (NODES * NODES) as f64) + 1e-12,
        })
    }

    /// Classify a local wheel point given in mm. Outside the convex blank
    /// `B`, any wheel point `q` satisfies `|p-q|² ≥ |p-p*|² + |p*-q|²` for
    /// the projection `p*` of `p` onto `B`, so both gaps combine.
    fn probe(&self, [x, y, z]: Point) -> Probe {
        let wheel = self.wheel;
        let m = wheel.module_mm();
        let (x, y, v) = (x / m, y / m, z / m);
        let h = wheel.half_width();
        let radius = x.hypot(y);
        let blank = wheel.blank();
        let outside = (radius - blank).max(0.).hypot((v.abs() - h).max(0.)) * m;
        let scale = if radius > blank { blank / radius } else { 1. };
        let (x, y, v, radius) = (x * scale, y * scale, v.clamp(-h, h), radius * scale);
        let combine = |inner: f64, node| {
            if inner <= 0. && outside <= 0. {
                Probe::Material
            } else {
                Probe::Clear(outside.hypot(inner.max(0.)), node)
            }
        };
        if let Some(bore) = wheel.bore_radius_mm()
            && radius < bore / m
        {
            return combine(bore - radius * m, None);
        }
        let pitch = 2. * wheel.span();
        let theta = y.atan2(x);
        let index = (theta / pitch).round();
        let local = rotate_z([x, y, 0.], -index * pitch);
        let sector = (index as i64).rem_euclid(i64::from(wheel.teeth())) as u32;
        let j = (((v + h) / (2. * h)) * STATIONS as f64)
            .round()
            .clamp(0., STATIONS as f64) as usize;
        let chain = &self.nodes[j];
        let angle = local[1].atan2(local[0]);
        let angles = |p: &[f64; 2]| p[1].atan2(p[0]);
        if angle <= angles(&chain[0]) || angle >= angles(chain.last().expect("chain")) {
            return combine(0., None);
        }
        let k = chain
            .partition_point(|p| angles(p) < angle)
            .clamp(1, chain.len() - 1);
        let [a, b] = [chain[k - 1], chain[k]];
        // Radius of the chord along the ray at `angle`.
        let (s, c) = angle.sin_cos();
        let d = [b[0] - a[0], b[1] - a[1]];
        let along = (a[0] * d[1] - a[1] * d[0]) / (c * d[1] - s * d[0]);
        let mut nearest = (f64::INFINITY, 0);
        for (i, pair) in chain.windows(2).enumerate() {
            let e = [pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]];
            let w = [local[0] - pair[0][0], local[1] - pair[0][1]];
            let t = ((w[0] * e[0] + w[1] * e[1]) / (e[0] * e[0] + e[1] * e[1])).clamp(0., 1.);
            let gap = (w[0] - t * e[0]).hypot(w[1] - t * e[1]);
            if gap < nearest.0 {
                nearest = (gap, if t < 0.5 { i } else { i + 1 });
            }
        }
        let shift = self.lipschitz * (v - self.stations[j]).abs();
        let gap = nearest.0 - self.chord - shift;
        if radius <= along || gap <= 0. {
            return combine(0., None);
        }
        combine(
            gap / self.lipschitz.hypot(1.) * m,
            Some((j, nearest.1, sector)),
        )
    }

    /// Exact boundary point of a node in mm, and its surface parameters.
    fn node_point(&self, j: usize, node: usize, sector: u32) -> Point {
        let m = self.wheel.module_mm();
        let [x, y] = self.nodes[j][node];
        rotate_z(
            [x * m, y * m, self.stations[j] * m],
            self.wheel.sector_angle(sector),
        )
    }

    /// Local refinement of the nearest exact flank point to `target` (mm).
    fn refine(&self, target: Point, j: usize, node: usize, sector: u32) -> Point {
        let wheel = self.wheel;
        let m = wheel.module_mm();
        let h = wheel.half_width();
        let piece = (node / NODES).min(4);
        let u0 = (node - piece * NODES) as f64 / NODES as f64;
        let point = |u: f64, v: f64| {
            let section = wheel.section(v);
            let [x, y] = wheel.piece_point(&section, PIECES[piece], u.clamp(0., 1.));
            rotate_z([x * m, y * m, v * m], wheel.sector_angle(sector))
        };
        let distance = |u: f64, v: f64| norm(sub(point(u, v), target));
        let (mut u, mut v) = (u0, self.stations[j]);
        let du = 2. / NODES as f64;
        let dv = 4. * h / STATIONS as f64;
        for _ in 0..3 {
            u = golden(|t| distance(t, v), (u - du).max(0.), (u + du).min(1.));
            v = golden(|t| distance(u, t), (v - dv).max(-h), (v + dv).min(h));
        }
        point(u, v)
    }
}

fn golden(f: impl Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..40 {
        let a = lo + (hi - lo) * 0.381_966_011_250_105_1;
        let b = hi - (hi - lo) * 0.381_966_011_250_105_1;
        if f(a) < f(b) {
            hi = b;
        } else {
            lo = a;
        }
    }
    (lo + hi) / 2.
}

/// Worm boundary face parametrization in the thread frame (mm).
#[derive(Clone, Copy)]
enum Patch {
    /// Profile band between two phases over the threaded length.
    Band {
        face: u32,
    },
    Cap(usize),
    Bore,
}

struct Rectangle {
    patch: Patch,
    a: [f64; 2],
    b: [f64; 2],
}

struct WormSide<'a> {
    worm: &'a Worm,
    placement: Placement,
    /// Partial derivative norm bounds per patch kind: band, cap, bore.
    rates: [[f64; 2]; 3],
}

impl WormSide<'_> {
    fn thread_point(&self, patch: Patch, [p, q]: [f64; 2]) -> Point {
        let worm = self.worm;
        match patch {
            Patch::Band { .. } => worm.wall_point(p, q),
            Patch::Cap(cap) => {
                let z = cap as f64 * worm.length_mm();
                let outer = worm.thread().radius(q, z);
                let inner = worm.bore_radius_mm().unwrap_or(0.);
                let r = inner + (outer - inner) * p;
                [r * q.cos(), r * q.sin(), z]
            }
            Patch::Bore => {
                let r = worm.bore_radius_mm().unwrap_or(0.);
                [r * p.cos(), r * p.sin(), q]
            }
        }
    }

    fn world(&self, thread: Point) -> Point {
        self.placement.world(self.worm.to_local(thread))
    }

    fn radius(&self, rectangle: &Rectangle) -> f64 {
        let kind = match rectangle.patch {
            Patch::Band { .. } => 0,
            Patch::Cap(_) => 1,
            Patch::Bore => 2,
        };
        let [rp, rq] = self.rates[kind];
        0.5 * (rp * (rectangle.b[0] - rectangle.a[0]) + rq * (rectangle.b[1] - rectangle.a[1]))
    }

    /// A thread-frame point moved strictly into the worm by `delta`.
    fn inward(&self, patch: Patch, thread: Point, delta: f64) -> Point {
        let [x, y, z] = thread;
        match patch {
            Patch::Band { .. } => {
                let r = x.hypot(y);
                let k = (r - delta) / r;
                [x * k, y * k, z]
            }
            Patch::Cap(cap) => [x, y, if cap == 0 { z + delta } else { z - delta }],
            Patch::Bore => {
                let r = x.hypot(y);
                let k = (r + delta) / r;
                [x * k, y * k, z]
            }
        }
    }
}

/// Decide positive clearance or a strict common interior point for a placed
/// worm and wheel. The nearest witness uses actual flank points within
/// `reach_mm`.
pub fn certify(
    worm: &Worm,
    worm_at: Placement,
    wheel: &Wheel,
    wheel_at: Placement,
    tolerance_mm: f64,
    reach_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Certificate, Error> {
    certify_within(
        (worm, wheel),
        [worm_at, wheel_at],
        [tolerance_mm, reach_mm],
        interrupt,
        MAX_RECTANGLES,
    )
}

fn certify_within(
    (worm, wheel): (&Worm, &Wheel),
    [worm_at, wheel_at]: [Placement; 2],
    [tolerance_mm, reach_mm]: [f64; 2],
    interrupt: Option<&dyn Fn() -> bool>,
    budget: usize,
) -> Result<Certificate, Error> {
    if !worm_at.valid()
        || !wheel_at.valid()
        || tolerance_mm.is_nan()
        || tolerance_mm <= 0.
        || reach_mm.is_nan()
        || reach_mm < 0.
    {
        return Err(Error::Invalid);
    }
    let field = Field::new(wheel)?;
    let spec = worm.thread().spec();
    let rate = worm.thread().helical_rate();
    let major = spec.major_radius_mm;
    let slope = worm.thread().maximum_profile_slope();
    let bore = worm.bore_radius_mm().unwrap_or(0.);
    let side = WormSide {
        worm,
        placement: worm_at,
        rates: [
            [slope.hypot(major / rate), (major / rate).hypot(1.)],
            [major - bore, major + rate * slope],
            [bore, 1.],
        ],
    };
    let phases = worm.rail_phases();
    let length = worm.length_mm();
    let mut stack = vec![];
    for (i, face) in [2, 5, 4, 3].into_iter().enumerate() {
        stack.push(Rectangle {
            patch: Patch::Band { face },
            a: [phases[i + 1], 0.],
            b: [phases[i], length],
        });
    }
    for cap in 0..2 {
        stack.push(Rectangle {
            patch: Patch::Cap(cap),
            a: [0., 0.],
            b: [1., TAU],
        });
    }
    if bore > 0. {
        stack.push(Rectangle {
            patch: Patch::Bore,
            a: [0., 0.],
            b: [TAU, length],
        });
    }
    // Gaps below about this radius stay unresolved. After the first
    // unresolved rectangle only coarse cells continue the witness search.
    let mut minimum = (1e-4 * wheel.module_mm()).max(tolerance_mm);
    let mut unresolved = false;
    let mut count = 0;
    let mut nearest: Option<(f64, Point, (usize, usize, u32))> = None;
    while let Some(rectangle) = stack.pop() {
        count += 1;
        if count > budget {
            return Err(Error::Limit);
        }
        if count.is_multiple_of(4096) && interrupt.is_some_and(|stop| stop()) {
            return Err(Error::Cancelled);
        }
        let middle = [0, 1].map(|i| (rectangle.a[i] + rectangle.b[i]) / 2.);
        let thread = side.thread_point(rectangle.patch, middle);
        let world = side.world(thread);
        let local = wheel_at.local(world);
        let radius = side.radius(&rectangle);
        match field.probe(local) {
            Probe::Clear(bound, node) => {
                if let (Some(node), Patch::Band { face: 2 | 4, .. }) = (node, rectangle.patch) {
                    let [j, i, sector] = [node.0, node.1, node.2 as usize];
                    let gap = norm(sub(field.node_point(j, i, sector as u32), local));
                    if nearest.is_none_or(|(best, ..)| gap < best) {
                        nearest = Some((gap, local, node));
                    }
                }
                if bound > radius {
                    continue;
                }
            }
            Probe::Material => {
                let margin = wheel.margin(local);
                let depth = match rectangle.patch {
                    Patch::Band { .. } => thread[0].hypot(thread[1]) - bore,
                    Patch::Cap(_) => length,
                    Patch::Bore => major,
                };
                let inside = side.inward(rectangle.patch, thread, (margin / 2.).min(depth / 2.));
                let witness = wheel_at.local(side.world(inside));
                if margin > 1e-9 * wheel.module_mm()
                    && worm.margin(worm.to_local(inside)) > 1e-12
                    && wheel.margin(witness) > 1e-12
                {
                    return Ok(Certificate {
                        result: ResultKind::CommonInterior,
                        nearest: None,
                    });
                }
            }
        }
        if radius <= minimum {
            unresolved = true;
            minimum = minimum.max(1e-2 * wheel.module_mm());
            continue;
        }
        let kind = match rectangle.patch {
            Patch::Band { .. } => 0,
            Patch::Cap(_) => 1,
            Patch::Bore => 2,
        };
        let rates = side.rates[kind];
        let axis = usize::from(
            rates[1] * (rectangle.b[1] - rectangle.a[1])
                > rates[0] * (rectangle.b[0] - rectangle.a[0]),
        );
        let split = middle[axis];
        let mut low = Rectangle {
            patch: rectangle.patch,
            a: rectangle.a,
            b: rectangle.b,
        };
        let mut high = Rectangle {
            patch: rectangle.patch,
            a: rectangle.a,
            b: rectangle.b,
        };
        low.b[axis] = split;
        high.a[axis] = split;
        stack.extend([low, high]);
    }
    let witness = nearest.map(|(_, local, (j, i, sector))| {
        let point = field.refine(local, j, i, sector);
        Witness {
            distance_mm: norm(sub(point, local)),
            points_mm: [wheel_at.world(local), wheel_at.world(point)],
        }
    });
    let witness = witness.filter(|w| w.distance_mm <= reach_mm);
    if unresolved {
        return Ok(Certificate {
            result: ResultKind::Unresolved,
            nearest: None,
        });
    }
    // A cleared worm boundary leaves the wheel either disjoint or enclosed.
    let section = wheel.section(0.);
    let tooth = wheel.joint(&section, 0);
    let m = wheel.module_mm();
    let probe = [tooth[0] * m * 0.999, tooth[1] * m * 0.999, 0.];
    let enclosed = worm.margin(worm_at.local(wheel_at.world(probe))) >= 0.;
    Ok(Certificate {
        result: [ResultKind::Clear, ResultKind::Unresolved][usize::from(enclosed)],
        nearest: witness,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worm_gear::{wheel::tests::spec as wheel_spec, worm::tests::spec as worm_spec};

    /// The canonical mating pose: wheel axis +Z at the origin, worm axis
    /// along wheel +Y at center distance `(teeth/2 + 5) m`, both at drive.
    fn pose(teeth: u32, m: f64, drive: f64, extra: f64) -> [Placement; 2] {
        let center = (f64::from(teeth) / 2. + 5.) * m;
        let (sw, cw) = drive.sin_cos();
        // Worm local +Z is wheel +Y and local X starts on wheel +X, so the
        // worm's -X crest faces the wheel axis. Drive turns it about +Y.
        let worm_axes = [[cw, 0., -sw], [-sw, 0., -cw], [0., 1., 0.]];
        let turn = -drive / f64::from(teeth) + extra;
        let (s, c) = turn.sin_cos();
        [
            Placement {
                origin_mm: [center, 0., 0.],
                axes: worm_axes,
            },
            Placement {
                origin_mm: [0.; 3],
                axes: [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]],
            },
        ]
    }

    fn members(teeth: u32, bore: [f64; 2], thinning: f64) -> (Worm, Wheel) {
        let worm = Worm::new(worm_spec(1., 20., bore[0], thinning), 1e-3).unwrap();
        let wheel = Wheel::new(wheel_spec(1., teeth, 1.5, bore[1], thinning), 1e-3, 1e7).unwrap();
        (worm, wheel)
    }

    fn run(teeth: u32, drive: f64, extra: f64) -> Certificate {
        let (worm, wheel) = members(teeth, [0., 0.], 0.04);
        let [a, b] = pose(teeth, 1., drive, extra);
        certify(&worm, a, &wheel, b, 1e-7, 0.1, None).unwrap()
    }

    #[test]
    fn mating_poses_clear_with_witnesses_and_wrong_poses_overlap() {
        for (teeth, drive) in [(40, 0.), (40, 1.3), (32, 2.5), (64, -0.7)] {
            let result = run(teeth, drive, 0.);
            assert_eq!(result.result, ResultKind::Clear, "{teeth} {drive}");
            let witness = result.nearest.unwrap();
            assert!(
                witness.distance_mm < 0.1 && witness.distance_mm > 0.02,
                "{witness:?}"
            );
            let measured = norm(sub(witness.points_mm[0], witness.points_mm[1]));
            assert!((measured - witness.distance_mm).abs() < 1e-12);
            let wrong = run(teeth, drive, std::f64::consts::PI / f64::from(teeth));
            assert_eq!(wrong.result, ResultKind::CommonInterior);
        }
        // A quarter-pitch phase error overlaps only along one flank.
        assert_eq!(
            run(40, 0., std::f64::consts::PI / 80.).result,
            ResultKind::CommonInterior
        );
        // Bored members and a moved center distance.
        let (worm, wheel) = members(40, [2., 6.], 0.04);
        let [mut a, b] = pose(40, 1., 0.4, 0.);
        assert_eq!(
            certify(&worm, a, &wheel, b, 1e-7, 0.1, None)
                .unwrap()
                .result,
            ResultKind::Clear
        );
        a.origin_mm[0] -= 0.5;
        assert_eq!(
            certify(&worm, a, &wheel, b, 1e-7, 0.1, None)
                .unwrap()
                .result,
            ResultKind::CommonInterior
        );
        // Far apart: clear without an engagement witness.
        a.origin_mm[0] += 40.;
        let far = certify(&worm, a, &wheel, b, 1e-7, 0.1, None).unwrap();
        assert_eq!((far.result, far.nearest), (ResultKind::Clear, None));
    }

    #[test]
    fn touching_enclosed_and_invalid_inputs_stay_conservative() {
        // Zero thinning: conjugate flanks touch without a strict overlap.
        let (worm, wheel) = members(40, [0., 0.], 0.);
        let [a, b] = pose(40, 1., 0., 0.);
        let touching = certify(&worm, a, &wheel, b, 1e-7, 0.1, None).unwrap();
        assert_eq!(touching.result, ResultKind::Unresolved);
        assert_eq!(touching.nearest, None);
        // A small wheel buried in a large worm overlaps its core.
        let small = Wheel::new(wheel_spec(0.5, 32, 0.5, 0., 0.), 1e-3, 1e7).unwrap();
        let big = Worm::new(worm_spec(2., 20., 0., 0.), 1e-3).unwrap();
        let centered = Placement {
            origin_mm: [0.; 3],
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        };
        let buried = certify(&big, centered, &small, centered, 1e-7, 0.1, None).unwrap();
        assert_eq!(buried.result, ResultKind::CommonInterior);
        // The bore wall of a hollow worm crosses the wheel material.
        let hollow = Worm::new(worm_spec(2., 20., 5., 0.), 1e-3).unwrap();
        let crossing = certify(&hollow, centered, &small, centered, 1e-7, 0.1, None).unwrap();
        assert_eq!(crossing.result, ResultKind::CommonInterior);
        // A slim worm threads coaxially through a large wheel bore.
        let slim = Worm::new(worm_spec(0.5, 20., 0., 0.), 1e-3).unwrap();
        let ring = Wheel::new(wheel_spec(2., 64, 1., 40., 0.), 1e-3, 1e7).unwrap();
        let through = certify(&slim, centered, &ring, centered, 1e-7, 0.1, None).unwrap();
        assert_eq!((through.result, through.nearest), (ResultKind::Clear, None));
        assert_eq!(
            certify_within((&worm, &wheel), [a, b], [1e-7, 0.1], None, 10),
            Err(Error::Limit)
        );
        let mut bad = a;
        bad.axes[2] = [1., 0., 0.];
        assert_eq!(
            certify(&worm, bad, &wheel, b, 1e-7, 0.1, None),
            Err(Error::Invalid)
        );
        assert_eq!(
            certify(&worm, a, &wheel, b, 0., 0.1, None),
            Err(Error::Invalid)
        );
        assert_eq!(
            certify(&worm, a, &wheel, b, 1e-7, -1., None),
            Err(Error::Invalid)
        );
        let stop = || true;
        assert_eq!(
            certify(&worm, a, &wheel, b, 1e-7, 0.1, Some(&stop)),
            Err(Error::Cancelled)
        );
        let far = certify(
            &members(40, [0., 0.], 0.04).0,
            a,
            &members(40, [0., 0.], 0.04).1,
            b,
            1e-7,
            0.,
            None,
        )
        .unwrap();
        assert_eq!(far.nearest, None);
    }
}
