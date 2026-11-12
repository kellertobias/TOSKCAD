//! Bounded support of the actual radial intersection, not of its display mesh.
use super::{interval::I, *};
use std::{cmp::Ordering, collections::BinaryHeap};
#[derive(Clone, Copy)]
struct Cell {
    q: [f64; 2],
    z: [f64; 2],
    band: usize,
    period: u32,
    entry: bool,
    upper: f64,
    split_q: bool,
}
impl PartialEq for Cell {
    fn eq(&self, o: &Self) -> bool {
        self.upper == o.upper
    }
}
impl Eq for Cell {}
impl PartialOrd for Cell {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cell {
    fn cmp(&self, o: &Self) -> Ordering {
        self.upper.total_cmp(&o.upper)
    }
}
#[derive(Clone, Copy)]
struct Jet {
    value: I,
    first: [I; 2],
    second: [I; 3],
}
struct Query<'a> {
    body: &'a Chamfer,
    d: [I; 3],
    rate: I,
    radial: I,
    slope: I,
}
impl Chamfer {
    /// Support enclosure in the caller's objective units. `tolerance` bounds
    /// the returned upper-minus-lower gap, including all arithmetic error.
    pub fn support(&self, direction: Point, options: QueryOptions) -> Result<Enclosure, Error> {
        self.support_with_interrupt(direction, options, None)
    }
    pub fn support_with_interrupt(
        &self,
        direction: Point,
        options: QueryOptions,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Enclosure, Error> {
        let mut work = Work::new(options, interrupt)?;
        work.step()?;
        if direction.into_iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid);
        }
        let scale = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if scale == 0. {
            return Ok(Enclosure {
                lower: 0.,
                upper: 0.,
            });
        }
        let normalized = direction.map(|v| {
            if v == 0. {
                I::point(0.)
            } else if v == scale {
                I::point(1.)
            } else if v == -scale {
                I::point(-1.)
            } else {
                I::point(v) / I::point(scale)
            }
        });
        let radial = (normalized[0].square() + normalized[1].square()).sqrt();
        let finish = |value: I| -> Result<Enclosure, Error> {
            let value = (value * I::point(scale)).checked()?;
            if value.width() > options.tolerance {
                return Err(Error::Unresolved);
            }
            Ok(Enclosure {
                lower: value.lo,
                upper: value.hi,
            })
        };
        if direction[0] == 0. && direction[1] == 0. {
            if direction[2] < 0. {
                return Ok(Enclosure {
                    lower: 0.,
                    upper: 0.,
                });
            }
            if scale == 1. {
                return Ok(Enclosure {
                    lower: self.full_length_mm(),
                    upper: self.full_length_mm(),
                });
            }
            return finish(I::point(self.full_length_mm()));
        }
        let spec = self.tool.spec();
        let breaks = self.tool.profile_breaks();
        let central = I::point(self.material_length_mm()) - I::point(self.entry_length_mm);
        let crest_gap = I::point(breaks[4]) - I::point(breaks[1]);
        if direction[2] == 0. && central.lo >= crest_gap.hi {
            if scale == 1. && (direction[0] == 0. || direction[1] == 0.) {
                return Ok(Enclosure {
                    lower: spec.major_radius_mm,
                    upper: spec.major_radius_mm,
                });
            }
            return finish(radial * I::point(spec.major_radius_mm));
        }
        let tolerance = (I::point(options.tolerance) / I::point(scale)).lo;
        if tolerance <= 0. {
            return Err(Error::Unresolved);
        }
        let rate = self.rate_interval()?;
        let query = Query {
            body: self,
            d: normalized,
            rate: rate.checked()?,
            radial,
            slope: self.cone_slope()?,
        };
        let mut minimum = f64::INFINITY;
        for (band, span) in breaks.windows(2).enumerate() {
            work.step()?;
            minimum = minimum.min(self.profile(band, I::new(span[0], span[1]))?.radius.lo);
        }
        let full_cone = ((I::point(minimum) - I::point(self.entry_radius_mm)).positive()
            / query.slope)
            .lo
            .max(0.)
            .min(self.entry_length_mm);
        // The axis is always material. A zero cone interval does not prove
        // the entire entry disk when the source valleys lie inside that disk.
        let axis = normalized[2] * I::point(self.full_length_mm());
        let mut fixed = I::new(axis.lo.max(0.), axis.hi.max(0.));
        let union = |a: I, b: I| I::new(a.lo.max(b.lo), a.hi.max(b.hi));
        if minimum >= self.entry_radius_mm {
            fixed = union(fixed, query.cone_disk_support([0., full_cone]));
        }
        if let Some(tail) = self.terminal {
            let [a, b] = [tail.material_length_mm(), tail.full_length_mm()]
                .map(|z| normalized[2] * I::point(z));
            let axial = I::new(a.lo.max(b.lo), a.hi.max(b.hi));
            fixed = union(fixed, radial * I::point(tail.neck_radius_mm()) + axial);
        }
        let mut best = fixed.lo;
        let frozen = fixed.hi;
        let mut heap = BinaryHeap::new();
        let central = if direction[2] >= 0. {
            [
                self.entry_length_mm
                    .max((I::point(self.material_length_mm()) - I::point(spec.pitch_mm)).lo),
                self.material_length_mm(),
            ]
        } else {
            [
                self.entry_length_mm,
                self.material_length_mm()
                    .min((I::point(self.entry_length_mm) + I::point(spec.pitch_mm)).hi),
            ]
        };
        for (z, entry) in [([full_cone, self.entry_length_mm], true), (central, false)] {
            // Each initial cell spans at most one quarter pitch in Z. The
            // total source length/pitch is already bounded by Thread admission.
            let count = ((z[1] - z[0]) / spec.pitch_mm * 4.).ceil().max(1.) as usize;
            for iz in 0..count {
                let a = if iz == 0 {
                    z[0]
                } else {
                    (z[1] - z[0]).mul_add(iz as f64 / count as f64, z[0])
                };
                let b = if iz + 1 == count {
                    z[1]
                } else {
                    (z[1] - z[0]).mul_add((iz + 1) as f64 / count as f64, z[0])
                };
                for period in 0..spec.starts {
                    for (band, q) in breaks.windows(2).enumerate() {
                        let (cell, lower) =
                            query.cell([q[0], q[1]], [a, b], band, period, entry, &mut work)?;
                        best = best.max(lower);
                        heap.push(cell);
                    }
                }
            }
        }
        finish(query.refine(heap, best, frozen, tolerance, &mut work)?)
    }
}

impl Query<'_> {
    fn refine(
        &self,
        mut heap: BinaryHeap<Cell>,
        mut best: f64,
        mut frozen: f64,
        tolerance: f64,
        work: &mut Work<'_>,
    ) -> Result<I, Error> {
        loop {
            let upper = heap.peek().map_or(frozen, |cell| frozen.max(cell.upper));
            if (I::point(upper) - I::point(best)).hi <= tolerance {
                return Ok(I::new(best, upper));
            }
            let cell = heap.pop().ok_or(Error::Unresolved)?;
            if cell.upper <= best {
                continue;
            }
            let span = if cell.split_q { cell.q } else { cell.z };
            let mid = span[0].midpoint(span[1]);
            if mid == span[0] || mid == span[1] {
                frozen = frozen.max(cell.upper);
                continue;
            }
            for part in [[span[0], mid], [mid, span[1]]] {
                let (q, z) = if cell.split_q {
                    (part, cell.z)
                } else {
                    (cell.q, part)
                };
                let (child, lower) = self.cell(q, z, cell.band, cell.period, cell.entry, work)?;
                best = best.max(lower);
                if child.upper > best {
                    heap.push(child);
                }
            }
        }
    }

    fn trigonometry(&self, q: I, z: I, period: u32) -> (I, I) {
        let phase = q + I::point(f64::from(period)) * I::point(self.body.tool.spec().pitch_mm);
        let theta = (z - phase) / self.rate;
        let cos = theta.cos();
        let sin = theta.sin();
        (
            self.d[0] * cos + self.d[1] * sin,
            -self.d[0] * sin + self.d[1] * cos,
        )
    }
    fn jet(&self, band: usize, q: I, z: I, period: u32, cone: bool) -> Result<Jet, Error> {
        let (c, s) = self.trigonometry(q, z, period);
        let inv = I::point(1.) / self.rate;
        let inv2 = inv.square();
        let two = I::point(2.);
        let jet = if cone {
            let r = I::point(self.body.entry_radius_mm) + self.slope * z;
            Jet {
                value: r * c + self.d[2] * z,
                first: [-r * s * inv, self.slope * c + r * s * inv + self.d[2]],
                second: [
                    -r * c * inv2,
                    -self.slope * s * inv + r * c * inv2,
                    two * self.slope * s * inv - r * c * inv2,
                ],
            }
        } else {
            let p = self.body.profile(band, q)?;
            let r = p.radius;
            let d = p.first;
            let dd = p.second;
            Jet {
                value: r * c + self.d[2] * z,
                first: [d * c - r * s * inv, r * s * inv + self.d[2]],
                second: [
                    dd * c - two * d * s * inv - r * c * inv2,
                    d * s * inv + r * c * inv2,
                    -r * c * inv2,
                ],
            }
        };
        jet.value.checked()?;
        for x in jet.first.into_iter().chain(jet.second) {
            x.checked()?;
        }
        Ok(jet)
    }
    fn point(&self, band: usize, q: f64, z: f64, period: u32, entry: bool) -> Result<I, Error> {
        let q = I::point(q);
        let z = I::point(z);
        let (c, _) = self.trigonometry(q, z, period);
        let mut r = self.body.profile(band, q)?.radius;
        if entry {
            let cone = I::point(self.body.entry_radius_mm) + self.slope * z;
            r = I::new(r.lo.min(cone.lo), r.hi.min(cone.hi));
        }
        ((r * c).positive() + self.d[2] * z).checked()
    }
    fn cone_disk_support(&self, z: [f64; 2]) -> I {
        let start = self.radial * I::point(self.body.entry_radius_mm);
        let slope = self.radial * self.slope + self.d[2];
        let [a, b] = z.map(|z| start + slope * I::point(z));
        I::new(a.lo.max(b.lo), a.hi.max(b.hi))
    }
    fn cell(
        &self,
        q: [f64; 2],
        z: [f64; 2],
        band: usize,
        period: u32,
        entry: bool,
        work: &mut Work<'_>,
    ) -> Result<(Cell, f64), Error> {
        work.step()?;
        let qspan = I::new(q[0], q[1]);
        let zspan = I::new(z[0], z[1]);
        let qm = (I::point(q[0]) + I::point(q[1])) / I::point(2.);
        let zm = (I::point(z[0]) + I::point(z[1])) / I::point(2.);
        let half = [
            (I::point(q[1]) - I::point(q[0])) / I::point(2.),
            (I::point(z[1]) - I::point(z[0])) / I::point(2.),
        ];
        let axis = (self.d[2] * zspan).hi;
        let taylor = |cone| -> Result<(f64, [f64; 2]), Error> {
            let center = self.jet(band, qm, zm, period, cone)?;
            let whole = self.jet(band, qspan, zspan, period, cone)?;
            let linear = center.first.map(I::magnitude);
            let second = whole.second.map(I::magnitude);
            let cross = I::point(second[1]) * half[0] * half[1];
            let q_error = I::point(linear[0]) * half[0]
                + I::point(second[0]) * half[0].square() / I::point(2.)
                + cross / I::point(2.);
            let z_error = I::point(linear[1]) * half[1]
                + I::point(second[2]) * half[1].square() / I::point(2.)
                + cross / I::point(2.);
            let upper = (center.value + q_error + z_error)
                .hi
                .min(whole.value.hi)
                .max(axis);
            Ok((upper, [q_error.hi, z_error.hi]))
        };
        let (mut upper, demand) = taylor(false)?;
        if entry {
            upper = upper.min(taylor(true)?.0).min(self.cone_disk_support(z).hi);
        }
        let mut lower = f64::NEG_INFINITY;
        for [qa, za] in [
            [q[0].midpoint(q[1]), z[0].midpoint(z[1])],
            [q[0], z[0]],
            [q[0], z[1]],
            [q[1], z[0]],
            [q[1], z[1]],
        ] {
            lower = lower.max(self.point(band, qa, za, period, entry)?.lo);
        }
        I::new(lower, upper).checked()?;
        Ok((
            Cell {
                q,
                z,
                band,
                period,
                entry,
                upper,
                split_q: demand[0] >= demand[1],
            },
            lower,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn priority_and_precision_floor_keep_real_patch_support_unresolved() {
        let body = Chamfer::new(super::super::tests::tool(false, 1, false), 0.3, None).unwrap();
        let query = Query {
            body: &body,
            d: [I::point(0.), I::point(0.), I::point(1.)],
            rate: body.rate_interval().unwrap(),
            radial: I::point(0.),
            slope: body.cone_slope().unwrap(),
        };
        let mut work = Work {
            remaining: 1000,
            interrupt: None,
        };
        let (a, lower) = query
            .cell(
                [0.01, 0.01f64.next_up()],
                [1., 1_f64.next_up()],
                0,
                0,
                false,
                &mut work,
            )
            .unwrap();
        assert!(a == a);
        assert_eq!(a.partial_cmp(&a), Some(Ordering::Equal));
        let mut heap = BinaryHeap::new();
        heap.push(a);
        assert_eq!(
            query.refine(heap, lower, 0., 1e-300, &mut work),
            Err(Error::Unresolved)
        );
        let (b, _) = query
            .cell([0., 0.01], [0.5, 1.], 0, 0, false, &mut work)
            .unwrap();
        let mut heap = BinaryHeap::new();
        heap.push(b);
        assert_eq!(
            query.refine(
                heap,
                body.full_length_mm(),
                body.full_length_mm().next_up(),
                1e-300,
                &mut work
            ),
            Err(Error::Unresolved)
        );
    }
}
