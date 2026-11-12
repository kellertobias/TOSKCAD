//! Complete actual tensor-image/trim coupling. No source tensor is substituted.
use super::polynomial::Curve;
use super::{polynomial::*, *};

pub(super) struct Surface {
    axes: [Curve<1>; 2],
    rows: Vec<Vec<[I; 3]>>,
    gradient: [f64; 2],
    cached: std::cell::RefCell<Option<((usize, usize), tensor::Cell)>>,
}
fn axis(degree: &Value<'_>, mults: &Value<'_>, knots: &Value<'_>) -> Result<Curve<1>> {
    let Value::Number(degree) = *degree else {
        return Err(ReadError::Invalid);
    };
    require(degree.fract() == 0. && (1. ..=11.).contains(&degree))?;
    let list = |value: &Value<'_>| -> Result<Vec<f64>> {
        let Value::List(items) = value else {
            return Err(ReadError::Invalid);
        };
        items
            .iter()
            .map(|v| match *v {
                Value::Number(n) => Ok(n),
                _ => Err(ReadError::Invalid),
            })
            .collect()
    };
    let degree = degree as usize;
    let knots = list(knots)?;
    let mults = list(mults)?
        .into_iter()
        .map(|n| {
            require(n.fract() == 0. && (1. ..=12.).contains(&n))?;
            Ok(n as usize)
        })
        .collect::<Result<Vec<_>>>()?;
    if !(2..=1025).contains(&knots.len()) {
        return Err(ReadError::Limit);
    }
    let count = degree * (knots.len() - 1) + 1;
    Curve::new(degree, knots, mults, vec![[0.]; count])
}
impl Surface {
    pub fn read(shell: &Shell<'_, '_>, id: u32, work: &mut Work) -> Result<Self> {
        let [
            Value::String(_),
            a,
            b,
            Value::List(rows),
            Value::Enum("UNSPECIFIED"),
            Value::Enum("F"),
            Value::Enum("F"),
            Value::Enum("F"),
            am,
            bm,
            ak,
            bk,
            Value::Enum("UNSPECIFIED" | "PIECEWISE_BEZIER_KNOTS"),
        ] = shell.args(id, "B_SPLINE_SURFACE_WITH_KNOTS")?
        else {
            return Err(ReadError::Invalid);
        };
        let axes = [axis(a, am, ak)?, axis(b, bm, bk)?];
        require(rows.len() == axes[0].poles.len())?;
        let mut controls = Vec::with_capacity(rows.len());
        for row in rows {
            let ids = refs(row)?;
            require(ids.len() == axes[1].poles.len())?;
            work.charge(ids.len() * 3)?;
            controls.push(
                ids.into_iter()
                    .map(|id| shell.point(id).map(|p| p.map(I::point)))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        Self::new(axes, controls, work)
    }
    fn new(axes: [Curve<1>; 2], rows: Vec<Vec<[I; 3]>>, work: &mut Work) -> Result<Self> {
        require(
            rows.len() == axes[0].poles.len()
                && rows.iter().all(|r| r.len() == axes[1].poles.len()),
        )?;
        let mut result = Self {
            axes,
            rows,
            gradient: [0.; 2],
            cached: std::cell::RefCell::new(None),
        };
        // The tensor is C0 along each shared Bernstein row. These derivative
        // hulls therefore bound global physical-parameter displacement even
        // when a small reference collar crosses an actual internal knot.
        for i in 0..result.axes[0].knots.len() - 1 {
            for j in 0..result.axes[1].knots.len() - 1 {
                let points = result.points(i, j);
                for axis in 0..2 {
                    let n = result.axes[axis].degree;
                    let span = I::point(result.axes[axis].knots[if axis == 0 { i } else { j } + 1])
                        - I::point(result.axes[axis].knots[if axis == 0 { i } else { j }]);
                    for u in 0..=result.axes[0].degree - usize::from(axis == 0) {
                        for v in 0..=result.axes[1].degree - usize::from(axis == 1) {
                            work.charge(3)?;
                            let a = points[u * (result.axes[1].degree + 1) + v];
                            let b = points[(u + usize::from(axis == 0))
                                * (result.axes[1].degree + 1)
                                + v
                                + usize::from(axis == 1)];
                            let norm = (0..3).fold(I::point(0.), |sum, k| {
                                sum + ((b[k] - a[k]) * I::point(n as f64) / span).abs()
                            });
                            result.gradient[axis] = result.gradient[axis].max(finite(norm)?.hi);
                        }
                    }
                }
            }
        }
        Ok(result)
    }
    fn points(&self, i: usize, j: usize) -> Vec<[I; 3]> {
        let [a, b] = self.axes.each_ref().map(|a| a.degree);
        (0..=a)
            .flat_map(|u| (0..=b).map(move |v| self.rows[i * a + u][j * b + v]))
            .collect()
    }
    fn error(&self, curve: &[[I; 3]], uv: &[[I; 2]], work: &mut Work) -> Result<f64> {
        let middle = evaluate(uv, I::point(0.5), work)?;
        let mut spans = [0; 2];
        let mut normalized = uv.to_vec();
        let mut collar = I::point(0.);
        for axis in 0..2 {
            let [global_a, global_b] = self.axes[axis].domain();
            let parameter = middle[axis]
                .lo
                .midpoint(middle[axis].hi)
                .clamp(global_a, global_b);
            let span = self.axes[axis].span(parameter)?;
            spans[axis] = span;
            let [a, b] = [self.axes[axis].knots[span], self.axes[axis].knots[span + 1]];
            let mut movement = 0_f64;
            for (target, original) in normalized.iter_mut().zip(uv) {
                let clamped = I::new(original[axis].lo.clamp(a, b), original[axis].hi.clamp(a, b));
                movement = movement.max(finite(original[axis] - clamped)?.magnitude());
                let t = finite((clamped - I::point(a)) / (I::point(b) - I::point(a)))?;
                // Intersect an outward arithmetic enclosure with a known
                // exact closed-domain fact, never extend the actual surface.
                target[axis] = I::new(t.lo.max(0.), t.hi.min(1.));
            }
            collar = collar + I::point(movement) * I::point(self.gradient[axis]);
        }
        let key = (spans[0], spans[1]);
        let mut cache = self.cached.borrow_mut();
        if cache.as_ref().is_none_or(|(previous, _)| *previous != key) {
            *cache = Some((
                key,
                tensor::Cell::new(
                    self.axes[0].degree + 1,
                    self.axes[1].degree + 1,
                    self.points(spans[0], spans[1]),
                    work,
                )?,
            ));
        }
        let cell = &cache.as_ref().expect("inserted actual cell").1;
        finite(collar + I::point(cell.curve_error(curve, &normalized, work)?)).map(|e| e.hi)
    }
    fn constant_u(&self, u: f64, work: &mut Work) -> Result<Curve<3>> {
        let span = self.axes[0].span(u)?;
        let a = self.axes[0].knots[span];
        let b = self.axes[0].knots[span + 1];
        let t = if u == a {
            I::point(0.)
        } else if u == b {
            I::point(1.)
        } else {
            finite((I::point(u) - I::point(a)) / (I::point(b) - I::point(a)))?
        };
        let t = I::new(t.lo.max(0.), t.hi.min(1.));
        let degree = self.axes[0].degree;
        let mut poles = Vec::with_capacity(self.axes[1].poles.len());
        for v in 0..self.axes[1].poles.len() {
            let controls = (0..=degree)
                .map(|i| self.rows[span * degree + i][v])
                .collect::<Vec<_>>();
            poles.push(evaluate(&controls, t, work)?);
        }
        Ok(Curve {
            degree: self.axes[1].degree,
            knots: self.axes[1].knots.clone(),
            poles,
        })
    }
    fn constant_v(&self, v: f64, work: &mut Work) -> Result<Curve<3>> {
        let span = self.axes[1].span(v)?;
        let a = self.axes[1].knots[span];
        let b = self.axes[1].knots[span + 1];
        let t = if v == a {
            I::point(0.)
        } else if v == b {
            I::point(1.)
        } else {
            finite((I::point(v) - I::point(a)) / (I::point(b) - I::point(a)))?
        };
        let t = I::new(t.lo.max(0.), t.hi.min(1.));
        let d = self.axes[1].degree;
        let poles = self
            .rows
            .iter()
            .map(|row| evaluate(&row[span * d..=span * d + d], t, work))
            .collect::<Result<Vec<_>>>()?;
        Ok(Curve {
            degree: self.axes[0].degree,
            knots: self.axes[0].knots.clone(),
            poles,
        })
    }
    pub fn verify(
        &self,
        curve: &Curve<3>,
        uv: &Curve<2>,
        tolerance: f64,
        work: &mut Work,
    ) -> Result<f64> {
        require(curve.domain() == uv.domain() && tolerance.is_finite() && tolerance > 0.)?;
        for point in &uv.poles {
            for (axis, value) in point.iter().enumerate() {
                let [a, b] = self.axes[axis].domain();
                require(value.lo >= a && value.hi <= b)?;
            }
        }
        if uv.degree == 1
            && uv.knots.len() == 2
            && uv.poles[0][1] == uv.poles[1][1]
            && uv.poles[0][0] == I::point(uv.knots[0])
            && uv.poles[1][0] == I::point(uv.knots[1])
        {
            let isoline = self.constant_v(uv.poles[0][1].lo, work)?;
            let error = identity_error(curve, &isoline, work)?;
            require(error <= tolerance)?;
            return Ok(error);
        }
        if uv.degree == 1
            && uv.knots.len() == 2
            && uv.poles[0][0] == uv.poles[1][0]
            && uv.poles[0][1] == I::point(uv.knots[0])
            && uv.poles[1][1] == I::point(uv.knots[1])
        {
            let isoline = self.constant_u(uv.poles[0][0].lo, work)?;
            let error = identity_error(curve, &isoline, work)?;
            require(error <= tolerance)?;
            return Ok(error);
        }
        let mut knots = curve.knots.clone();
        knots.extend_from_slice(&uv.knots);
        knots.sort_by(f64::total_cmp);
        knots.dedup();
        let mut pending = Vec::new();
        let mut maximum = 0_f64;
        for pair in knots.windows(2) {
            pending.push((
                curve.restrict([pair[0], pair[1]], work)?,
                uv.restrict([pair[0], pair[1]], work)?,
                0u8,
            ));
            while let Some((c, p, depth)) = pending.pop() {
                work.charge(1)?;
                let error = self.error(&c, &p, work)?;
                if error <= tolerance {
                    maximum = maximum.max(error);
                    continue;
                }
                if depth >= 40 {
                    return Err(ReadError::Unsupported);
                }
                let [ca, cb] = split(&c, I::point(0.5), work)?;
                let [pa, pb] = split(&p, I::point(0.5), work)?;
                pending.push((cb, pb, depth + 1));
                pending.push((ca, pa, depth + 1));
            }
        }
        Ok(maximum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emitted_actual_wall_trims_bind_to_their_actual_parameterized_surfaces() {
        let source = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let body = Chamfer::new(source, 0.4, None).unwrap();
        let text = crate::step::write_thread_entry_chamfer(
            &body,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7,
        )
        .unwrap();
        compound::read_members_by(&text, 1e-7, true, true, |shell, id, _| {
            let mut work = Work {
                remaining: 500_000_000,
                interrupt: None,
                cancelled: false,
            };
            let mut count = 0;
            for face_id in shell.member_faces(id)?.into_iter().take(4) {
                let (surface, _, bounds) = face(shell, face_id)?;
                let actual = Surface::read(shell, surface, &mut work)?;
                for (edge, _) in ring(shell, bounds[0], true)? {
                    let [_, _, _, geometry, _] =
                        <&[Value<'_>; 5]>::try_from(shell.args(edge, "EDGE_CURVE")?)
                            .expect("fixture edge");
                    if shell.simple(reference(geometry)?)?.name != "SURFACE_CURVE" {
                        continue;
                    }
                    let [_, curve, pcurves, _] = <&[Value<'_>; 4]>::try_from(
                        shell.args(reference(geometry)?, "SURFACE_CURVE")?,
                    )
                    .expect("fixture surface curve");
                    let c = Curve::<3>::read(shell, reference(curve)?)?;
                    for pc in refs(pcurves)? {
                        let (support, uv) = shell.pcurve_definition(pc)?;
                        if support == surface {
                            let uv = Curve::<2>::read(shell, uv)?;
                            actual.verify(&c, &uv, 1e-7, &mut work)?;
                            count += 1;
                        }
                    }
                }
            }
            assert!(count >= 16);
            eprintln!(
                "bound {count} actual wall edges, remaining work {}",
                work.remaining
            );
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    fn work() -> Work<'static> {
        Work {
            remaining: 5_000_000,
            interrupt: None,
            cancelled: false,
        }
    }
    fn chart() -> Curve<1> {
        Curve::new(1, vec![0., 0.5, 1.], vec![2, 1, 2], vec![[0.]; 3]).unwrap()
    }
    fn surface() -> Surface {
        let rows = (0..3)
            .map(|i| {
                (0..3)
                    .map(|j| [i as f64 / 2., j as f64 / 2., 0.].map(I::point))
                    .collect()
            })
            .collect();
        Surface::new([chart(), chart()], rows, &mut work()).unwrap()
    }
    #[test]
    fn actual_tensor_record_admission_and_interior_isolines_keep_finite_domain_authority() {
        let degree = Value::Number(1.);
        let m = Value::List(vec![Value::Number(2.), Value::Number(2.)]);
        let k = Value::List(vec![Value::Number(0.), Value::Number(1.)]);
        assert!(axis(&Value::Enum("F"), &m, &k).is_err());
        assert!(axis(&degree, &Value::Enum("F"), &k).is_err());
        assert!(
            axis(
                &degree,
                &m,
                &Value::List(vec![Value::Enum("F"), Value::Number(1.)])
            )
            .is_err()
        );
        let many = Value::List((0..1026).map(|i| Value::Number(i as f64)).collect());
        assert!(matches!(axis(&degree, &m, &many), Err(ReadError::Limit)));
        with_shell("#1=B_SPLINE_SURFACE_WITH_KNOTS('');", |s| {
            assert!(Surface::read(s, 1, &mut work()).is_err())
        });
        assert!(Surface::new([chart(), chart()], vec![], &mut work()).is_err());
        let uncached = surface();
        let curve = [
            [I::point(0.); 3],
            [I::point(1.), I::point(1.), I::point(0.)],
        ];
        let uv = [[I::point(0.); 2], [I::point(1.); 2]];
        assert!(matches!(
            uncached.error(
                &curve,
                &uv,
                &mut Work {
                    remaining: 8,
                    interrupt: None,
                    cancelled: false
                }
            ),
            Err(ReadError::Limit)
        ));
        let actual = surface();
        for (p, uv) in [
            (
                [[0.375, 0., 0.], [0.375, 1., 0.]],
                [[0.375, 0.], [0.375, 1.]],
            ),
            (
                [[0., 0.625, 0.], [1., 0.625, 0.]],
                [[0., 0.625], [1., 0.625]],
            ),
            ([[0., 0., 0.], [1., 1., 0.]], [[0., 0.], [1., 1.]]),
        ] {
            let c = Curve::new(1, vec![0., 1.], vec![2, 2], p.to_vec()).unwrap();
            let uv = Curve::new(1, vec![0., 1.], vec![2, 2], uv.to_vec()).unwrap();
            assert!(actual.verify(&c, &uv, 1e-9, &mut work()).unwrap() < 1e-9);
        }
        let c = Curve::new(
            1,
            vec![0., 1.],
            vec![2, 2],
            vec![[0., 0., 0.1], [1., 1., 0.1]],
        )
        .unwrap();
        let uv = Curve::new(1, vec![0., 1.], vec![2, 2], vec![[0., 0.], [1., 1.]]).unwrap();
        assert!(matches!(
            actual.verify(&c, &uv, 1e-9, &mut work()),
            Err(ReadError::Unsupported)
        ));
    }
}
