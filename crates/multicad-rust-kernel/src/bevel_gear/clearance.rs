//! Interference of two bevels sharing one pitch apex, decided on the unit
//! sphere. Each solid is a cone over its spherical tooth domain, so disjoint
//! domains prove positive clearance. Chords carry analytic error tubes.
use super::*;
use std::collections::HashMap;

const MAX_SEGMENTS: usize = 400_000;
const TARGETS: [f64; 2] = [1e-6, 1e-8];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub apex_mm: Point,
    /// World images of the local x, y and z axes.
    pub axes: [Point; 3],
}

impl Placement {
    fn world(&self, local: Point) -> Point {
        std::array::from_fn(|i| {
            self.axes[0][i] * local[0] + self.axes[1][i] * local[1] + self.axes[2][i] * local[2]
        })
    }

    fn local(&self, world: Point) -> Point {
        self.axes.map(|axis| dot(axis, world))
    }

    fn valid(&self) -> bool {
        let [x, y, z] = self.axes;
        self.apex_mm.iter().all(|v| v.is_finite())
            && self.axes.iter().all(|a| (norm(*a) - 1.).abs() <= 1e-9)
            && dot(x, y).abs() <= 1e-9
            && dot(y, z).abs() <= 1e-9
            && dot(x, z).abs() <= 1e-9
            && norm(sub(cross(x, y), z)) <= 1e-9
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResultKind {
    Clear,
    CommonInterior,
    Unresolved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Limit,
    Cancelled,
}

/// Actual points on both solids; their distance bounds the true gap above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Witness {
    pub distance_mm: f64,
    pub points_mm: [Point; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Certificate {
    pub result: ResultKind,
    pub nearest: Option<Witness>,
}

struct Chain {
    sector: u32,
    piece: Piece,
    points: Vec<Point>,
}

struct Side<'a> {
    gear: &'a Bevel,
    placement: Placement,
    chains: Vec<Chain>,
    error: f64,
}

fn angle(a: Point, b: Point) -> f64 {
    norm(cross(a, b)).atan2(dot(a, b))
}

/// Closest points between segments `p0 p1` and `q0 q1` as parameters.
fn closest(p0: Point, p1: Point, q0: Point, q1: Point) -> (f64, f64, f64) {
    let d1 = sub(p1, p0);
    let d2 = sub(q1, q0);
    let r = sub(p0, q0);
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);
    let c = dot(d1, r);
    let b = dot(d1, d2);
    let denominator = a * e - b * b;
    let mut s = if denominator > 0. {
        ((b * f - c * e) / denominator).clamp(0., 1.)
    } else {
        0.
    };
    let mut t = (b * s + f) / e;
    if t < 0. {
        t = 0.;
        s = (-c / a).clamp(0., 1.);
    } else if t > 1. {
        t = 1.;
        s = ((b - c) / a).clamp(0., 1.);
    }
    let pa = std::array::from_fn(|i| p0[i] + d1[i] * s);
    let pb = std::array::from_fn(|i| q0[i] + d2[i] * t);
    (norm(sub(pa, pb)), s, t)
}

impl<'a> Side<'a> {
    fn new(
        gear: &'a Bevel,
        placement: Placement,
        other: (&Bevel, Placement),
        target: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        let reach = other.0.tip_gamma();
        let axis = other.1.axes[2];
        let spec = gear.spec;
        let mut chains = vec![];
        let mut error = 0_f64;
        let mut total = 0;
        for sector in 0..spec.teeth {
            if interrupt.is_some_and(|stop| stop()) {
                return Err(Error::Cancelled);
            }
            let center = placement.world(gear.pitch_direction(sector));
            let start = gear.sector_start(sector);
            let span = TAU / f64::from(spec.teeth);
            let radius = [gear.root_gamma(), gear.tip_gamma()]
                .into_iter()
                .flat_map(|g| [start, start + span].map(|p| placement.world(polar(g, p))))
                .map(|corner| angle(corner, center))
                .fold(0., f64::max);
            if angle(center, axis) > reach + radius + 0.05 {
                continue;
            }
            for &piece in gear.pieces() {
                let bound = gear.curvature_bound(piece);
                let count = (bound / (8. * target)).sqrt().ceil().max(1.) as usize;
                total += count;
                if total > MAX_SEGMENTS {
                    return Err(Error::Limit);
                }
                let h = 1. / count as f64;
                error = error.max(h * h * bound / 8.);
                chains.push(Chain {
                    sector,
                    piece,
                    points: (0..=count)
                        .map(|i| {
                            let sample = gear.sample(piece, i as f64 * h);
                            placement.world(gear.rotate(sector, sample.direction))
                        })
                        .collect(),
                });
            }
        }
        Ok(Self {
            gear,
            placement,
            chains,
            error,
        })
    }

    fn direction(&self, chain: &Chain, s: f64) -> Point {
        let sample = self.gear.sample(chain.piece, s);
        self.placement
            .world(self.gear.rotate(chain.sector, sample.direction))
    }

    /// Back-cone distance interval along one world direction.
    fn interval(&self, direction: Point) -> [f64; 2] {
        let local = self.placement.local(direction);
        let gamma = local[2].clamp(-1., 1.).acos();
        [End::Toe, End::Heel].map(|end| self.gear.end_distance_along(end, gamma))
    }

    fn contains(&self, direction: Point, margin: f64) -> bool {
        self.gear.direction_margin(self.placement.local(direction)) > margin
    }

    /// Strict interior direction of this gear inside the other's material.
    fn interior_in(&self, other: &Side) -> bool {
        let gear = self.gear;
        let mut seen = std::collections::BTreeSet::new();
        for chain in &self.chains {
            if !seen.insert(chain.sector) {
                continue;
            }
            for i in 1..10 {
                let gamma =
                    gear.root_gamma() + (gear.tip_gamma() - gear.root_gamma()) * f64::from(i) / 10.;
                let width = gear.half_width(gamma);
                for j in -6..=6 {
                    let local = polar(gamma, width * f64::from(j) / 7.);
                    let direction = self.placement.world(gear.rotate(chain.sector, local));
                    let [a0, a1] = self.interval(direction);
                    let [b0, b1] = other.interval(direction);
                    if other.contains(direction, other.error + 1e-9)
                        && a1.min(b1) - a0.max(b0) > 1e-9 * a1
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Decide positive clearance or a strict common-interior direction for two
/// bevels whose placed pitch apexes coincide within `tolerance_mm`. The
/// nearest witness uses actual surface points within `reach_mm`.
pub fn certify(
    gears: [&Bevel; 2],
    placements: [Placement; 2],
    tolerance_mm: f64,
    reach_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Certificate, Error> {
    if !placements.iter().all(Placement::valid)
        || tolerance_mm.is_nan()
        || tolerance_mm <= 0.
        || reach_mm.is_nan()
        || reach_mm < 0.
        || norm(sub(placements[0].apex_mm, placements[1].apex_mm)) > tolerance_mm
    {
        return Err(Error::Invalid);
    }
    let apex = placements[0].apex_mm;
    let near = gears
        .iter()
        .map(|g| g.end_distance_mm(End::Toe))
        .fold(f64::INFINITY, f64::min);
    let reach = reach_mm / near;
    for target in TARGETS {
        let a = Side::new(
            gears[0],
            placements[0],
            (gears[1], placements[1]),
            target,
            interrupt,
        )?;
        let b = Side::new(
            gears[1],
            placements[1],
            (gears[0], placements[0]),
            target,
            interrupt,
        )?;
        let length = |side: &Side| {
            side.chains
                .iter()
                .flat_map(|c| c.points.windows(2).map(|w| norm(sub(w[1], w[0]))))
                .fold(0., f64::max)
        };
        let cell = 2. * length(&a).max(length(&b)) + reach + a.error + b.error + 1e-9;
        let key = |p: Point| p.map(|v| (v / cell).floor() as i64);
        let mut grid: HashMap<[i64; 3], Vec<(usize, usize)>> = HashMap::new();
        for (c, chain) in b.chains.iter().enumerate() {
            for i in 0..chain.points.len() - 1 {
                grid.entry(key(chain.points[i])).or_default().push((c, i));
            }
        }
        let mut best = (f64::INFINITY, 0, 0, 0., 0, 0, 0.);
        let mut tests = 0_usize;
        for (ca, chain) in a.chains.iter().enumerate() {
            for i in 0..chain.points.len() - 1 {
                let [x, y, z] = key(chain.points[i]);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let Some(list) = grid.get(&[x + dx, y + dy, z + dz]) else {
                                continue;
                            };
                            for &(cb, j) in list {
                                tests += 1;
                                if tests.is_multiple_of(4096)
                                    && interrupt.is_some_and(|stop| stop())
                                {
                                    return Err(Error::Cancelled);
                                }
                                let q = &b.chains[cb].points;
                                let (d, s, t) =
                                    closest(chain.points[i], chain.points[i + 1], q[j], q[j + 1]);
                                if d < best.0 {
                                    best = (d, ca, i, s, cb, j, t);
                                }
                            }
                        }
                    }
                }
            }
        }
        let (distance, ca, i, s, cb, j, t) = best;
        let nearest = distance.is_finite().then(|| {
            let param =
                |chain: &Chain, i: usize, s: f64| (i as f64 + s) / (chain.points.len() - 1) as f64;
            let ua = a.direction(&a.chains[ca], param(&a.chains[ca], i, s));
            let ub = b.direction(&b.chains[cb], param(&b.chains[cb], j, t));
            let [a0, a1] = a.interval(ua);
            let [b0, b1] = b.interval(ub);
            let r = a0.max(b0);
            (r <= a1.min(b1)).then(|| {
                let points = [ua, ub].map(|u| std::array::from_fn(|k| apex[k] + r * u[k]));
                Witness {
                    distance_mm: norm(sub(points[0], points[1])),
                    points_mm: points,
                }
            })
        });
        let nearest = nearest.flatten().filter(|w| w.distance_mm <= reach_mm);
        if distance > a.error + b.error + 1e-12 {
            let crossed =
                a.contains(placements[1].axes[2], 0.) || b.contains(placements[0].axes[2], 0.);
            return Ok(Certificate {
                result: if crossed {
                    ResultKind::Unresolved
                } else {
                    ResultKind::Clear
                },
                nearest,
            });
        }
        if a.interior_in(&b) || b.interior_in(&a) {
            return Ok(Certificate {
                result: ResultKind::CommonInterior,
                nearest,
            });
        }
    }
    Ok(Certificate {
        result: ResultKind::Unresolved,
        nearest: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bevel_gear::tests::spec;

    fn pair(counts: [u32; 2], drive: f64, extra: f64) -> ([Bevel; 2], [Placement; 2]) {
        let gears = [0, 1].map(|i| {
            Bevel::new(spec(1., counts[i], counts[1 - i], 2., 0., 0.2), 1e-3, 1e7).unwrap()
        });
        let [za, zb] = counts.map(f64::from);
        let phase = PI - PI / zb - drive * za / zb + extra;
        let (s0, c0) = drive.sin_cos();
        let (s1, c1) = phase.sin_cos();
        let placements = [
            Placement {
                apex_mm: [0.; 3],
                axes: [[c0, s0, 0.], [-s0, c0, 0.], [0., 0., 1.]],
            },
            Placement {
                apex_mm: [0.; 3],
                axes: [[0., s1, -c1], [0., c1, s1], [1., 0., 0.]],
            },
        ];
        (gears, placements)
    }

    fn run(counts: [u32; 2], drive: f64, extra: f64) -> Certificate {
        let (gears, placements) = pair(counts, drive, extra);
        certify([&gears[0], &gears[1]], placements, 1e-6, 0.2, None).unwrap()
    }

    #[test]
    fn meshing_pairs_clear_with_a_flank_witness_and_wrong_phases_overlap() {
        for counts in [[24, 32], [32, 24], [24, 64]] {
            for drive in [0., 0.05, -0.11] {
                let result = run(counts, drive, 0.);
                assert_eq!(result.result, ResultKind::Clear, "{counts:?} {drive}");
                let witness = result.nearest.unwrap();
                assert!(witness.distance_mm < 0.1 && witness.distance_mm > 0.);
                let measured = norm(sub(witness.points_mm[0], witness.points_mm[1]));
                assert!((measured - witness.distance_mm).abs() < 1e-12);
            }
            let wrong = run(counts, 0., PI / f64::from(counts[1]));
            assert_eq!(wrong.result, ResultKind::CommonInterior);
        }
        // Quarter-pitch errors overlap only along the flanks.
        assert_eq!(
            run([24, 32], 0., PI / 64.).result,
            ResultKind::CommonInterior
        );
    }

    #[test]
    fn contact_nested_domains_and_invalid_inputs_stay_conservative() {
        let (mut gears, placements) = pair([24, 32], 0., 0.);
        for gear in &mut gears {
            let mut spec = gear.spec();
            // Zero backlash: conjugate flanks touch without a strict overlap.
            spec.half_base_tooth_angle_rad += 0.05 / f64::from(spec.teeth);
            *gear = Bevel::new(spec, 1e-3, 1e7).unwrap();
        }
        let touching = certify([&gears[0], &gears[1]], placements, 1e-6, 0.2, None).unwrap();
        assert_eq!(touching.result, ResultKind::Unresolved);
        assert_eq!(touching.nearest, None);
        let small = Bevel::new(spec(1., 24, 64, 2., 0., 0.2), 1e-3, 1e7).unwrap();
        let large = Bevel::new(spec(1., 64, 24, 2., 0., 0.2), 1e-3, 1e7).unwrap();
        let coaxial = [placements[0]; 2];
        let nested = certify([&small, &large], coaxial, 1e-6, 0.2, None).unwrap();
        assert_eq!(nested.result, ResultKind::Unresolved);
        assert_eq!(nested.nearest, None);
        let same = certify([&small, &small], coaxial, 1e-6, 0.2, None).unwrap();
        assert_eq!(same.result, ResultKind::CommonInterior);
        let mut moved = placements;
        moved[1].apex_mm = [1., 0., 0.];
        let refs = [&gears[0], &gears[1]];
        assert_eq!(certify(refs, moved, 1e-6, 0.2, None), Err(Error::Invalid));
        moved[1] = placements[1];
        moved[1].axes[2] = [0., 1., 0.];
        assert_eq!(certify(refs, moved, 1e-6, 0.2, None), Err(Error::Invalid));
        assert_eq!(
            certify(refs, placements, 0., 0.2, None),
            Err(Error::Invalid)
        );
        assert_eq!(
            certify(refs, placements, 1e-6, -1., None),
            Err(Error::Invalid)
        );
        let stop = || true;
        assert_eq!(
            certify(refs, placements, 1e-6, 0.2, Some(&stop)),
            Err(Error::Cancelled)
        );
        let calls = std::cell::Cell::new(0);
        let late = || {
            calls.set(calls.get() + 1);
            calls.get() > 2 * 24 + 2 * 32
        };
        assert_eq!(
            certify(refs, placements, 1e-6, 0.2, Some(&late)),
            Err(Error::Cancelled)
        );
        // Cancel after the first gear's sectors, while the second side builds.
        let calls = std::cell::Cell::new(0);
        let second = || {
            calls.set(calls.get() + 1);
            calls.get() > 24
        };
        assert_eq!(
            certify(refs, placements, 1e-6, 0.2, Some(&second)),
            Err(Error::Cancelled)
        );
        let side = Side::new(
            &gears[0],
            placements[0],
            (&gears[1], placements[1]),
            1e-16,
            None,
        );
        assert!(matches!(side, Err(Error::Limit)));
        // A witness beyond the requested reach is withheld.
        let far = certify(refs, placements, 1e-6, 0., None).unwrap();
        assert_eq!(far.nearest, None);
    }

    #[test]
    fn segment_distances_cover_parallel_and_clamped_cases() {
        let (d, s, t) = closest([0.; 3], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]);
        assert!((d - 1.).abs() < 1e-15 && s == 0. && t == 0.);
        let (d, _, t) = closest([0.; 3], [1., 0., 0.], [-2., 1., 0.], [-1., 1., 0.]);
        assert!((d - 2_f64.sqrt()).abs() < 1e-15 && t == 1.);
        let (d, s, t) = closest([0.; 3], [1., 0., 0.], [2., 1., 0.], [3., 1., 0.]);
        assert!((d - 2_f64.sqrt()).abs() < 1e-15 && s == 1. && t == 0.);
    }
}
