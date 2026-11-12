//! One-dimensional Gauss2 with a full fourth-derivative enclosure.
use super::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
struct Cell {
    ends: [I; 2],
    value: Values,
    score: f64,
}
impl PartialEq for Cell {
    fn eq(&self, o: &Self) -> bool {
        self.score == o.score
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
        self.score.total_cmp(&o.score)
    }
}
fn cell(
    source: Source<'_>,
    ends: [I; 2],
    scales: Values,
    work: &mut Work<'_>,
) -> Result<Cell, Error> {
    work.step()?;
    let width = ends[1] - ends[0];
    if width.lo <= 0. {
        return Err(Error::Unresolved);
    }
    let midpoint = (ends[0] + ends[1]) / I::point(2.);
    let offset = width / (I::point(12.).sqrt());
    let a = source.evaluate(Jet::variable(midpoint - offset))?;
    let b = source.evaluate(Jet::variable(midpoint + offset))?;
    let derivative = source.evaluate(Jet::variable(I::new(ends[0].lo, ends[1].hi)))?;
    let mut value = zero();
    let mut score = 0f64;
    for i in 0..COUNT {
        let error = I::point(derivative[i].0[4].magnitude()) * width.square().square() * width
            / I::point(180.);
        value[i] = ((a[i].0[0] + b[i].0[0]) * width / I::point(2.) + I::new(-error.hi, error.hi))
            .checked()?;
        score = score.max(value[i].width() / scales[i].lo);
    }
    Ok(Cell { ends, value, score })
}
fn ready(
    source: Source<'_>,
    ends: [I; 2],
    scales: Values,
    work: &mut Work<'_>,
) -> Result<Vec<Cell>, Error> {
    let mut pending = vec![ends];
    let mut output = Vec::new();
    while let Some(ends) = pending.pop() {
        match cell(source, ends, scales, work) {
            Ok(cell) => output.push(cell),
            Err(Error::Unresolved) => {
                let mid = (ends[0] + ends[1]) / I::point(2.);
                if mid.lo <= ends[0].hi || mid.hi >= ends[1].lo {
                    return Err(Error::Unresolved);
                }
                pending.push([ends[0], mid]);
                pending.push([mid, ends[1]]);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(output)
}
pub(super) fn integrate(
    source: Source<'_>,
    scales: Values,
    epsilon: f64,
    work: &mut Work<'_>,
) -> Result<Values, Error> {
    let mut heap = BinaryHeap::new();
    let mut total = zero();
    for c in ready(source, source.segment.ends.map(|k| k.phase), scales, work)? {
        add(&mut total, c.value);
        heap.push(c);
    }
    let mut iteration = 0usize;
    loop {
        work.step()?;
        if total
            .iter()
            .zip(scales)
            .all(|(v, s)| v.width() <= (I::point(epsilon) * s).lo)
        {
            return Ok(total);
        }
        let old = heap.pop().ok_or(Error::Unresolved)?;
        let mid = (old.ends[0] + old.ends[1]) / I::point(2.);
        if mid.lo <= old.ends[0].hi || mid.hi >= old.ends[1].lo {
            return Err(Error::Unresolved);
        }
        for (i, v) in total.iter_mut().enumerate() {
            *v = I::new(
                (I::point(v.lo) - I::point(old.value[i].lo)).lo,
                (I::point(v.hi) - I::point(old.value[i].hi)).hi,
            );
        }
        for ends in [[old.ends[0], mid], [mid, old.ends[1]]] {
            for c in ready(source, ends, scales, work)? {
                add(&mut total, c.value);
                heap.push(c);
            }
        }
        iteration += 1;
        if iteration.is_multiple_of(32) {
            total = zero();
            for cell in &heap {
                work.step()?;
                add(&mut total, cell.value);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_phase_intervals_priority_and_finite_resolution_refuse_without_a_partial_integral() {
        let body =
            Chamfer::new(super::super::super::tests::tool(false, 1, false), 0.3, None).unwrap();
        let mut source = Source {
            body: &body,
            start: 0,
            segment: body.phase_segments().unwrap()[0],
            kind: Kind::Wall,
        };
        let scales = [I::point(1.); COUNT];
        let mut work = Work::new(QueryOptions::default(), None).unwrap();
        let a = cell(source, [I::point(0.), I::point(0.01)], scales, &mut work).unwrap();
        let b = cell(source, [I::point(0.02), I::point(0.03)], scales, &mut work).unwrap();
        assert!(a == a);
        assert_eq!(a.partial_cmp(&b), Some(a.cmp(&b)));
        assert!(matches!(
            ready(source, [I::point(0.01); 2], scales, &mut work),
            Err(Error::Unresolved)
        ));
        let lo = 0.02_f64;
        for (end, value) in source.segment.ends.iter_mut().zip([lo, lo.next_up()]) {
            end.phase = I::point(value);
            end.nominal = value;
        }
        assert!(matches!(
            integrate(source, scales, 0., &mut work),
            Err(Error::Unresolved)
        ));
        assert!(
            integrate(
                Source {
                    segment: body.phase_segments().unwrap()[0],
                    ..source
                },
                scales,
                1e-5,
                &mut work
            )
            .is_ok()
        );
    }
}
