//! Queries over retained finite Bernstein nets, without an authored recipe.
use super::*;
use std::{cmp::Ordering, collections::BinaryHeap, sync::Arc};
const D: usize = MANUFACTURING_DEGREE;
use super::hermite::{MAX_POLES, certify_radial_control_chart};
type Point = [f64; 3];
type BoxPoint = [I; 3];
pub(super) fn checked_grid(spans: [usize; 2], actual: usize) -> Result<[usize; 2], Error> {
    if spans.contains(&0) {
        return Err(Error::Invalid);
    }
    let dimensions = spans.map(|n| {
        D.checked_mul(n)
            .and_then(|v| v.checked_add(1))
            .ok_or(Error::Limit)
    });
    let [width, height] = dimensions;
    let [width, height] = [width?, height?];
    let count = width.checked_mul(height).ok_or(Error::Limit)?;
    if count > MAX_POLES {
        return Err(Error::Limit);
    }
    if count != actual {
        return Err(Error::Invalid);
    }
    Ok([width, height])
}
pub(crate) struct RadialChart<'a> {
    pub spans: [usize; 2],
    pub controls: &'a [[I; 3]],
    pub outward: bool,
}
pub(crate) fn certify_radial_charts(
    charts: &[RadialChart<'_>],
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), Error> {
    let count = charts
        .iter()
        .try_fold(0usize, |s, c| s.checked_add(c.controls.len()))
        .ok_or(Error::Limit)?;
    if count > MAX_POLES {
        return Err(Error::Limit);
    }
    let mut work = Work {
        remaining: 1_000_000,
        interrupt,
    };
    for chart in charts {
        work.step()?;
        certify_radial_control_chart(chart.spans, chart.controls, chart.outward, &mut work)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct RetainedTensor {
    spans: [usize; 2],
    poles: Arc<[Point]>,
}
/// A retained parameter cell. Derivatives use its own unit square, so
/// subdivision carries the correct area scaling without an external factor.
#[derive(Clone, Debug)]
pub struct TensorCell {
    controls: Vec<Vec<BoxPoint>>,
    domain: [[f64; 2]; 2],
    depth: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TensorDerivatives {
    pub position: [[f64; 2]; 3],
    pub du: [[f64; 2]; 3],
    pub dv: [[f64; 2]; 3],
    pub duu: [[f64; 2]; 3],
    pub duv: [[f64; 2]; 3],
    pub dvv: [[f64; 2]; 3],
}
impl RetainedTensor {
    /// Only the actual finite net is retained. No thread dimensions are inferred.
    pub fn new(spans: [usize; 2], poles: Vec<Point>) -> Result<Self, Error> {
        checked_grid(spans, poles.len())?;
        if poles
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e7)
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            spans,
            poles: poles.into(),
        })
    }
    /// Prove regular outward radial projection in the stored axial chart.
    /// This does not by itself certify a closed solid or a thread recipe.
    pub fn certify_radial_chart(
        &self,
        outward: bool,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(), Error> {
        let controls = self
            .poles
            .iter()
            .map(|p| p.map(I::point))
            .collect::<Vec<_>>();
        certify_radial_charts(
            &[RadialChart {
                spans: self.spans,
                controls: &controls,
                outward,
            }],
            interrupt,
        )
    }
    pub fn spans(&self) -> [usize; 2] {
        self.spans
    }
    pub fn dimensions(&self) -> [usize; 2] {
        self.spans.map(|n| D * n + 1)
    }
    pub fn poles(&self) -> &[Point] {
        &self.poles
    }
    fn patch(&self, cell: [usize; 2]) -> Result<Vec<Vec<BoxPoint>>, Error> {
        if cell.into_iter().zip(self.spans).any(|(i, n)| i >= n) {
            return Err(Error::Invalid);
        }
        let height = self.dimensions()[1];
        Ok((0..=D)
            .map(|i| {
                (0..=D)
                    .map(|j| self.poles[(cell[0] * D + i) * height + cell[1] * D + j].map(I::point))
                    .collect()
            })
            .collect())
    }
    pub fn cell(&self, index: [usize; 2]) -> Result<TensorCell, Error> {
        Ok(TensorCell {
            controls: self.patch(index)?,
            domain: [[0., 1.]; 2],
            depth: 0,
        })
    }
    /// Complete image enclosure of a local Bernstein parameter rectangle.
    pub fn point_bounds(
        &self,
        cell: [usize; 2],
        uv: [[f64; 2]; 2],
    ) -> Result<[[f64; 2]; 3], Error> {
        image_bounds(self.patch(cell)?, uv)
    }
    pub fn derivative_bounds(
        &self,
        cell: [usize; 2],
        uv: [[f64; 2]; 2],
    ) -> Result<TensorDerivatives, Error> {
        let p = self.patch(cell)?;
        let du = derivative(&p, 0);
        let dv = derivative(&p, 1);
        Ok(TensorDerivatives {
            position: image_bounds(p, uv)?,
            du: image_bounds(du.clone(), uv)?,
            dv: image_bounds(dv.clone(), uv)?,
            duu: image_bounds(derivative(&du, 0), uv)?,
            duv: image_bounds(derivative(&du, 1), uv)?,
            dvv: image_bounds(derivative(&dv, 1), uv)?,
        })
    }
    pub fn support_bounds(
        &self,
        direction: Point,
        accuracy: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<[f64; 2], Error> {
        if direction.into_iter().any(|x| !x.is_finite()) || !accuracy.is_finite() || accuracy <= 0.
        {
            return Err(Error::Invalid);
        }
        let mut work = Work {
            remaining: 1_000_000,
            interrupt,
        };
        work.step()?;
        if direction == [0.; 3] {
            return Ok([0.; 2]);
        }
        let mut heap = BinaryHeap::new();
        let mut lower = f64::NEG_INFINITY;
        for i in 0..self.spans[0] {
            for j in 0..self.spans[1] {
                work.step()?;
                let points = self.patch([i, j])?;
                let values = points
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|p| {
                                p.into_iter()
                                    .zip(direction)
                                    .fold(I::point(0.), |s, (p, d)| s + p * I::point(d))
                            })
                            .collect()
                    })
                    .collect();
                let cell = SupportCell::new(values, 0)?;
                lower = lower.max(cell.lower);
                heap.push(cell);
            }
        }
        loop {
            work.step()?;
            let old = heap.pop().ok_or(Error::Unresolved)?;
            if (I::point(old.upper) - I::point(lower)).hi <= accuracy {
                return Ok([lower, old.upper.max(lower)]);
            }
            if old.depth >= 64 || heap.len() >= 131072 {
                return Err(Error::Limit);
            }
            let axis = old.depth as usize % 2;
            for child in old.split(axis)? {
                work.step()?;
                lower = lower.max(child.lower);
                if child.upper >= lower {
                    heap.push(child);
                }
            }
        }
    }
}
impl TensorCell {
    pub fn domain(&self) -> [[f64; 2]; 2] {
        self.domain
    }
    pub fn bounds(&self) -> Result<[[f64; 2]; 3], Error> {
        hull(self.controls.iter().flatten().copied())
    }
    pub fn derivative_bounds(&self) -> Result<TensorDerivatives, Error> {
        self.derivative_bounds_at([[0., 1.]; 2])
    }
    /// Derivatives use this leaf's unit-square parameter, including the
    /// chain-rule factors already retained by its subdivided controls.
    pub fn derivative_bounds_at(&self, uv: [[f64; 2]; 2]) -> Result<TensorDerivatives, Error> {
        let du = derivative(&self.controls, 0);
        let dv = derivative(&self.controls, 1);
        Ok(TensorDerivatives {
            position: self.point_bounds(uv)?,
            du: image_bounds(du.clone(), uv)?,
            dv: image_bounds(dv.clone(), uv)?,
            duu: image_bounds(derivative(&du, 0), uv)?,
            duv: image_bounds(derivative(&du, 1), uv)?,
            dvv: image_bounds(derivative(&dv, 1), uv)?,
        })
    }
    pub fn point_bounds(&self, uv: [[f64; 2]; 2]) -> Result<[[f64; 2]; 3], Error> {
        image_bounds(self.controls.clone(), uv)
    }
    pub fn subdivide(&self, axis: usize) -> Result<[Self; 2], Error> {
        if axis > 1 {
            return Err(Error::Invalid);
        }
        if self.depth >= 64 {
            return Err(Error::Limit);
        }
        let [lo, hi] = self.domain[axis];
        let middle = lo + (hi - lo) * 0.5;
        if middle <= lo || middle >= hi {
            return Err(Error::Unresolved);
        }
        let mut source = self.controls.clone();
        if axis == 0 {
            source = transpose(source);
        }
        let mut left = Vec::with_capacity(source.len());
        let mut right = Vec::with_capacity(source.len());
        for row in &source {
            let (a, b) = split(row, I::point(0.5));
            left.push(a);
            right.push(b);
        }
        if axis == 0 {
            left = transpose(left);
            right = transpose(right);
        }
        let mut a = self.domain;
        let mut b = self.domain;
        a[axis][1] = middle;
        b[axis][0] = middle;
        Ok([
            Self {
                controls: left,
                domain: a,
                depth: self.depth + 1,
            },
            Self {
                controls: right,
                domain: b,
                depth: self.depth + 1,
            },
        ])
    }
}
/// Private interval calculation cell over a checked source patch. The
/// coefficients enclose its original poles in a centered orthonormal chart;
/// this does not construct replacement manufacturing geometry.
pub(crate) fn calculation_cell(controls: [[I; 3]; 100]) -> TensorCell {
    TensorCell {
        controls: controls
            .as_chunks::<10>()
            .0
            .iter()
            .map(|row| row.to_vec())
            .collect(),
        domain: [[0., 1.]; 2],
        depth: 0,
    }
}
pub(crate) fn curve_taylor9(points: &[BoxPoint]) -> [[I; 3]; 10] {
    let mut controls = points.iter().map(|&p| vec![p]).collect::<Vec<_>>();
    let mut factorial = 1.;
    std::array::from_fn(|order| {
        let value = coefficient_hull(&controls).map(|v| v / I::point(factorial));
        if order < 9 {
            controls = derivative(&controls, 0);
            factorial *= (order + 1) as f64;
        }
        value
    })
}
pub(crate) fn curve_first_bounds(points: &[BoxPoint], t: I) -> [[I; 3]; 2] {
    let derivatives = points
        .windows(2)
        .map(|p| std::array::from_fn(|k| (p[1][k] - p[0][k]) * I::point((points.len() - 1) as f64)))
        .collect::<Vec<_>>();
    [points, &derivatives].map(|p| {
        let weights = bernstein(p.len() - 1, t);
        let mut value = p[0];
        for (point, weight) in p.iter().zip(weights).skip(1) {
            for k in 0..3 {
                value[k] = value[k] + weight * (point[k] - p[0][k]);
            }
        }
        value
    })
}

/// The exact polynomial normal P_u cross P_v of one calculation patch.
/// Product coefficients are formed before interval hulls, preserving the
/// cancellations lost by crossing two independent derivative boxes.
pub(crate) struct NormalPolynomial {
    controls: Vec<Vec<BoxPoint>>,
}
pub(crate) fn normal_polynomial(cell: &TensorCell) -> NormalPolynomial {
    let du = derivative(&cell.controls, 0);
    let dv = derivative(&cell.controls, 1);
    let choose = |n: usize, k: usize| {
        let mut value = I::point(1.);
        for i in 0..k {
            value = value * I::point((n - i) as f64) / I::point((i + 1) as f64);
        }
        value
    };
    let weights = std::array::from_fn::<_, 9, _>(|i| {
        std::array::from_fn::<_, 10, _>(|j| choose(8, i) * choose(9, j) / choose(17, i + j))
    });
    let mut controls = vec![vec![[I::point(0.); 3]; 18]; 18];
    for (i, row) in du.iter().enumerate() {
        for (j, &u) in row.iter().enumerate() {
            for (a, row) in dv.iter().enumerate() {
                for (b, &v) in row.iter().enumerate() {
                    let cross = [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ];
                    let weight = weights[i][a] * weights[b][j];
                    for k in 0..3 {
                        controls[i + a][j + b][k] = controls[i + a][j + b][k] + weight * cross[k];
                    }
                }
            }
        }
    }
    NormalPolynomial { controls }
}
impl NormalPolynomial {
    pub(crate) fn taylor8(
        &self,
        domain: [[f64; 2]; 2],
        along_v: bool,
    ) -> Result<[[I; 3]; 9], Error> {
        let restricted = restricted_grid(self.controls.clone(), domain)?;
        let mut controls = self.controls.clone();
        let widths = domain.map(|d| I::point(d[1]) - I::point(d[0]));
        let mut scale = widths[0] * widths[1];
        let mut factorial = 1.;
        Ok(std::array::from_fn(|order| {
            // Differentiate the fixed root coefficients before multiplying
            // the child chain factor. Differencing independently enclosed
            // child coefficients would lose their shared source correlation
            // and create a non-shrinking high-derivative uncertainty floor.
            let hull = if order == 0 {
                coefficient_hull(&restricted)
            } else {
                coefficient_hull(&controls)
            };
            let value = hull.map(|v| v * scale / I::point(factorial));
            if order < 8 {
                controls = derivative(&controls, usize::from(along_v));
                factorial *= (order + 1) as f64;
                scale = scale * widths[usize::from(along_v)];
            }
            value
        }))
    }
}

/// Interval Taylor coefficients through order eight, for P, P_u and P_v.
/// All coordinates and derivatives use this retained leaf's unit square.
/// A source leaf remains degree nine after subdivision; the ninth source
/// derivative required here therefore never produces an empty control net.
pub(crate) fn tensor_taylor8(cell: &TensorCell, along_v: bool) -> [[[I; 3]; 9]; 3] {
    let base = [
        cell.controls.clone(),
        derivative(&cell.controls, 0),
        derivative(&cell.controls, 1),
    ];
    base.map(|mut controls| {
        let mut factorial = 1.;
        std::array::from_fn(|order| {
            let value = coefficient_hull(&controls).map(|v| v / I::point(factorial));
            if order < 8 {
                controls = derivative(&controls, usize::from(along_v));
                factorial *= (order + 1) as f64;
            }
            value
        })
    })
}
fn coefficient_hull(controls: &[Vec<BoxPoint>]) -> BoxPoint {
    let mut bounds = [I::new(f64::INFINITY, f64::NEG_INFINITY); 3];
    for point in controls.iter().flatten() {
        for i in 0..3 {
            bounds[i].lo = bounds[i].lo.min(point[i].lo);
            bounds[i].hi = bounds[i].hi.max(point[i].hi);
        }
    }
    bounds
}
/// Factored Bernstein evaluation encloses P, P_u and P_v at the complete
/// requested parameter intervals. The caller supplies unit-square nodes.
/// Partition of unity is applied before rounding, retaining a large anchor
/// once rather than repeatedly multiplying it by uncertain basis weights.
pub(crate) fn tensor_first_bounds(cell: &TensorCell, uv: [I; 2]) -> [[I; 3]; 3] {
    [
        cell.controls.clone(),
        derivative(&cell.controls, 0),
        derivative(&cell.controls, 1),
    ]
    .map(|controls| {
        let bu = bernstein(controls.len() - 1, uv[0]);
        let bv = bernstein(controls[0].len() - 1, uv[1]);
        let anchor = controls[0][0];
        let mut delta = [I::point(0.); 3];
        for (i, row) in controls.iter().enumerate() {
            for (j, p) in row.iter().enumerate() {
                if i == 0 && j == 0 {
                    continue;
                }
                let weight = bu[i] * bv[j];
                for k in 0..3 {
                    delta[k] = delta[k] + weight * (p[k] - anchor[k]);
                }
            }
        }
        std::array::from_fn(|k| anchor[k] + delta[k])
    })
}
fn bernstein(degree: usize, t: I) -> Vec<I> {
    let one = I::point(1.) - t;
    (0..=degree)
        .map(|i| {
            let mut c = I::point(1.);
            for j in 0..i {
                c = c * I::point((degree - j) as f64) / I::point((j + 1) as f64);
            }
            for _ in 0..i {
                c = c * t;
            }
            for _ in i..degree {
                c = c * one;
            }
            c
        })
        .collect()
}

fn transpose<T: Copy>(rows: Vec<Vec<T>>) -> Vec<Vec<T>> {
    (0..rows[0].len())
        .map(|j| rows.iter().map(|row| row[j]).collect())
        .collect()
}
fn derivative(p: &[Vec<BoxPoint>], axis: usize) -> Vec<Vec<BoxPoint>> {
    let degree = if axis == 0 {
        p.len() - 1
    } else {
        p[0].len() - 1
    };
    (0..p.len() - usize::from(axis == 0))
        .map(|i| {
            (0..p[0].len() - usize::from(axis == 1))
                .map(|j| {
                    let [a, b] = if axis == 0 {
                        [p[i][j], p[i + 1][j]]
                    } else {
                        [p[i][j], p[i][j + 1]]
                    };
                    std::array::from_fn(|k| (b[k] - a[k]) * I::point(degree as f64))
                })
                .collect()
        })
        .collect()
}
fn image_bounds(rows: Vec<Vec<BoxPoint>>, uv: [[f64; 2]; 2]) -> Result<[[f64; 2]; 3], Error> {
    hull(restricted_grid(rows, uv)?.into_iter().flatten())
}
fn restricted_grid(
    mut rows: Vec<Vec<BoxPoint>>,
    uv: [[f64; 2]; 2],
) -> Result<Vec<Vec<BoxPoint>>, Error> {
    for [lo, hi] in uv {
        if !lo.is_finite() || !hi.is_finite() || lo < 0. || hi > 1. || lo > hi {
            return Err(Error::Invalid);
        }
    }
    if uv == [[0., 1.]; 2] {
        return Ok(rows);
    }
    for row in &mut rows {
        *row = restrict(row, I::new(uv[1][0], uv[1][1]))?;
    }
    let mut columns = (0..rows[0].len())
        .map(|j| rows.iter().map(|row| row[j]).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    for column in &mut columns {
        *column = restrict(column, I::new(uv[0][0], uv[0][1]))?;
    }
    Ok(transpose(columns))
}
fn hull(points: impl Iterator<Item = BoxPoint>) -> Result<[[f64; 2]; 3], Error> {
    let mut bounds = [I::new(f64::INFINITY, f64::NEG_INFINITY); 3];
    for point in points {
        for axis in 0..3 {
            bounds[axis].lo = bounds[axis].lo.min(point[axis].lo);
            bounds[axis].hi = bounds[axis].hi.max(point[axis].hi);
        }
    }
    for bound in bounds {
        bound.checked()?;
    }
    Ok(bounds.map(|p| [p.lo, p.hi]))
}
fn split(points: &[BoxPoint], t: I) -> (Vec<BoxPoint>, Vec<BoxPoint>) {
    let mut current = points.to_vec();
    let mut left = vec![current[0]];
    let mut right = vec![current[current.len() - 1]];
    let complement = I::point(1.) - t;
    for n in (1..points.len()).rev() {
        for i in 0..n {
            current[i] =
                std::array::from_fn(|a| complement * current[i][a] + t * current[i + 1][a]);
        }
        left.push(current[0]);
        right.push(current[n - 1]);
    }
    right.reverse();
    (left, right)
}
fn restrict(points: &[BoxPoint], range: I) -> Result<Vec<BoxPoint>, Error> {
    if range.lo == 0. && range.hi == 1. {
        return Ok(points.to_vec());
    }
    let upper = if range.hi == 1. {
        points.to_vec()
    } else {
        split(points, I::point(range.hi)).0
    };
    let restricted = if range.lo == 0. {
        upper
    } else {
        split(&upper, I::point(range.lo) / I::point(range.hi)).1
    };
    for p in &restricted {
        for coordinate in p {
            coordinate.checked()?;
        }
    }
    Ok(restricted)
}
struct SupportCell {
    values: Vec<Vec<I>>,
    lower: f64,
    upper: f64,
    depth: u32,
}
impl SupportCell {
    fn new(values: Vec<Vec<I>>, depth: u32) -> Result<Self, Error> {
        let upper = values
            .iter()
            .flatten()
            .map(|v| v.hi)
            .fold(f64::NEG_INFINITY, f64::max);
        let mut column = Vec::new();
        for row in &values {
            let points = row
                .iter()
                .map(|&v| [v, I::point(0.), I::point(0.)])
                .collect::<Vec<_>>();
            column.push(
                split(&points, I::point(0.5))
                    .0
                    .last()
                    .copied()
                    .ok_or(Error::Invalid)?,
            );
        }
        let lower = split(&column, I::point(0.5))
            .0
            .last()
            .ok_or(Error::Invalid)?[0]
            .lo;
        if !lower.is_finite() || !upper.is_finite() || lower > upper {
            return Err(Error::Unresolved);
        }
        Ok(Self {
            values,
            lower,
            upper,
            depth,
        })
    }
    fn split(self, axis: usize) -> Result<[Self; 2], Error> {
        let mut source = self.values;
        if axis == 0 {
            source = (0..source[0].len())
                .map(|j| source.iter().map(|row| row[j]).collect())
                .collect();
        }
        let mut a = Vec::new();
        let mut b = Vec::new();
        for row in source {
            let points = row
                .into_iter()
                .map(|v| [v, I::point(0.), I::point(0.)])
                .collect::<Vec<_>>();
            let (l, r) = split(&points, I::point(0.5));
            a.push(l.into_iter().map(|p| p[0]).collect::<Vec<_>>());
            b.push(r.into_iter().map(|p| p[0]).collect::<Vec<_>>());
        }
        if axis == 0 {
            let transpose = |p: Vec<Vec<I>>| {
                (0..p[0].len())
                    .map(|j| p.iter().map(|row| row[j]).collect())
                    .collect()
            };
            a = transpose(a);
            b = transpose(b);
        }
        Ok([Self::new(a, self.depth + 1)?, Self::new(b, self.depth + 1)?])
    }
}
impl PartialEq for SupportCell {
    fn eq(&self, r: &Self) -> bool {
        self.cmp(r) == Ordering::Equal
    }
}
impl Eq for SupportCell {}
impl PartialOrd for SupportCell {
    fn partial_cmp(&self, r: &Self) -> Option<Ordering> {
        Some(self.cmp(r))
    }
}
impl Ord for SupportCell {
    fn cmp(&self, r: &Self) -> Ordering {
        self.upper.total_cmp(&r.upper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph() -> RetainedTensor {
        let points = (0..=D)
            .flat_map(|i| (0..=D).map(move |j| [i as f64, j as f64, (i * j) as f64]))
            .collect();
        RetainedTensor::new([1, 1], points).unwrap()
    }
    #[test]
    fn retained_control_queries_enclose_the_actual_finite_graph_and_its_derivatives() {
        let t = graph();
        assert_eq!(t.spans(), [1, 1]);
        assert_eq!(t.dimensions(), [10, 10]);
        assert_eq!(t.poles().len(), 100);
        for uv in [
            [[0., 0.], [0., 0.]],
            [[1., 1.], [1., 1.]],
            [[0.2, 0.4], [0.3, 0.6]],
            [[1e-17, 0.2], [0.25, 0.75]],
        ] {
            let p = t.point_bounds([0, 0], uv).unwrap();
            assert!(p[0][0] <= 9. * uv[0][0] + 1e-14 && p[0][1] >= 9. * uv[0][1] - 1e-14);
            assert!(p[1][0] <= 9. * uv[1][0] + 1e-14 && p[1][1] >= 9. * uv[1][1] - 1e-14);
            assert!(
                p[2][0] <= 81. * uv[0][0] * uv[1][0] + 1e-13
                    && p[2][1] >= 81. * uv[0][1] * uv[1][1] - 1e-13
            );
            let d = t.derivative_bounds([0, 0], uv).unwrap();
            assert_eq!(d.position, p);
            assert!(d.du[0][0] <= 9. && d.du[0][1] >= 9.);
            assert!(d.dv[1][0] <= 9. && d.dv[1][1] >= 9.);
            assert!(d.duv[2][0] <= 81. && d.duv[2][1] >= 81.);
            for a in [d.duu, d.dvv] {
                for [lo, hi] in a {
                    assert!(lo <= 0. && hi >= 0.);
                }
            }
        }
        for (direction, value) in [([1., 0., 0.], 9.), ([-1., -1., 3.], 225.), ([0.; 3], 0.)] {
            let [lo, hi] = t.support_bounds(direction, 1e-6, None).unwrap();
            assert!(lo <= value + 1e-14 && hi >= value - 1e-14 && hi - lo <= 1e-6);
        }
    }
    #[test]
    fn retained_queries_refuse_invalid_limits_and_cancellation_without_losing_source() {
        let t = graph();
        let source = t.clone();
        assert_eq!(RetainedTensor::new([0, 1], vec![]), Err(Error::Invalid));
        assert_eq!(
            RetainedTensor::new([usize::MAX, 1], vec![]),
            Err(Error::Limit)
        );
        assert_eq!(RetainedTensor::new([1000, 1000], vec![]), Err(Error::Limit));
        assert_eq!(RetainedTensor::new([1, 1], vec![]), Err(Error::Invalid));
        let mut p = t.poles.to_vec();
        p[0][0] = f64::NAN;
        assert_eq!(RetainedTensor::new([1, 1], p), Err(Error::Invalid));
        assert_eq!(t.point_bounds([1, 0], [[0., 1.]; 2]), Err(Error::Invalid));
        for range in [[-1., 0.], [0., 2.], [1., 0.], [f64::NAN, 0.]] {
            assert_eq!(
                t.point_bounds([0, 0], [range, [0., 1.]]),
                Err(Error::Invalid)
            );
        }
        assert_eq!(
            t.support_bounds([1., 0., 0.], 0., None),
            Err(Error::Invalid)
        );
        assert_eq!(
            t.support_bounds([f64::NAN, 0., 0.], 1e-6, None),
            Err(Error::Invalid)
        );
        assert_eq!(
            t.support_bounds([1., 0., 0.], 1e-6, Some(&|| true)),
            Err(Error::Cancelled)
        );
        assert_eq!(
            t.support_bounds([1., 0., 0.], 1e-300, None),
            Err(Error::Limit)
        );
        assert_eq!(
            t.support_bounds([f64::MAX; 3], 1e-6, None),
            Err(Error::Unresolved)
        );
        assert_eq!(t, source);
        t.support_bounds([1., 0., 0.], 1e-6, None).unwrap();
    }
    #[test]
    fn radial_chart_proof_is_independent_of_thread_recipe_and_preserves_polarity() {
        let poles = (0..=D)
            .flat_map(|i| {
                (0..=D).map(move |j| [2., 2. * i as f64 / D as f64 - 1., 4. * j as f64 / D as f64])
            })
            .collect::<Vec<_>>();
        let surface = RetainedTensor::new([1, 1], poles).unwrap();
        surface.certify_radial_chart(true, None).unwrap();
        assert_eq!(
            surface.certify_radial_chart(false, None),
            Err(Error::Unresolved)
        );
        let reversed = (0..=D)
            .rev()
            .flat_map(|i| {
                surface.poles[i * (D + 1)..(i + 1) * (D + 1)]
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();
        RetainedTensor::new([1, 1], reversed)
            .unwrap()
            .certify_radial_chart(false, None)
            .unwrap();
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            let n = calls.get() + 1;
            calls.set(n);
            n == 10
        };
        assert_eq!(
            surface.certify_radial_chart(true, Some(&cancel)),
            Err(Error::Cancelled)
        );
        assert_eq!(calls.get(), 10);
        surface.certify_radial_chart(true, None).unwrap();
        let mut poles = surface.poles.to_vec();
        for p in &mut poles {
            p[1] = 0.;
        }
        assert_eq!(
            RetainedTensor::new([1, 1], poles)
                .unwrap()
                .certify_radial_chart(true, None),
            Err(Error::Unresolved)
        );
        let large = vec![[I::point(2.), I::point(0.), I::point(1.)]; 10 * (D * 2000 + 1)];
        let charts = (0..3)
            .map(|_| RadialChart {
                spans: [1, 2000],
                controls: &large,
                outward: true,
            })
            .collect::<Vec<_>>();
        assert_eq!(certify_radial_charts(&charts, None), Err(Error::Limit));
    }
    #[test]
    fn multi_cell_support_retains_all_source_spans_and_priority_order() {
        let spans = [2, 3];
        let [w, h] = spans.map(|n| D * n + 1);
        let poles = (0..w)
            .flat_map(|i| (0..h).map(move |j| [i as f64, j as f64, (i * j) as f64]))
            .collect();
        let t = RetainedTensor::new(spans, poles).unwrap();
        let p = t.point_bounds([1, 2], [[0.2; 2], [0.3; 2]]).unwrap();
        for ([lo, hi], expected) in p.into_iter().zip([10.8, 20.7, 10.8 * 20.7]) {
            assert!(lo <= expected && hi >= expected);
        }
        let [lo, hi] = t.support_bounds([0., 1., 0.], 1e-6, None).unwrap();
        assert!(lo <= 27. && hi >= 27. && hi - lo <= 1e-6);
        let a = SupportCell::new(vec![vec![I::point(1.); 2]; 2], 0).unwrap();
        let b = SupportCell::new(vec![vec![I::point(2.); 2]; 2], 0).unwrap();
        assert!(a < b && a != b);
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Less));
        assert!(a == SupportCell::new(vec![vec![I::point(1.); 2]; 2], 0).unwrap());
    }
    #[test]
    fn subdivision_encloses_the_same_geometry_and_carries_its_derivative_scaling() {
        let t = graph();
        let shared = t.clone();
        assert!(Arc::ptr_eq(&shared.poles, &t.poles));
        let cell = t.cell([0, 0]).unwrap();
        let midpoint = cell.derivative_bounds_at([[0.5, 0.5]; 2]).unwrap();
        for (bounds, value) in [
            (midpoint.du[0], 9.),
            (midpoint.dv[1], 9.),
            (midpoint.du[2], 40.5),
            (midpoint.dv[2], 40.5),
            (midpoint.duv[2], 81.),
        ] {
            assert!(bounds[0] <= value && bounds[1] >= value);
        }
        assert_eq!(cell.domain(), [[0., 1.]; 2]);
        assert_eq!(
            cell.bounds().unwrap(),
            t.point_bounds([0, 0], [[0., 1.]; 2]).unwrap()
        );
        let [left, right] = cell.subdivide(0).unwrap();
        assert_eq!(left.domain(), [[0., 0.5], [0., 1.]]);
        assert_eq!(right.domain(), [[0.5, 1.], [0., 1.]]);
        for child in [&left, &right] {
            let d = child.derivative_bounds().unwrap();
            assert!(d.du[0][0] <= 4.5 && d.du[0][1] >= 4.5);
            assert!(d.dv[1][0] <= 9. && d.dv[1][1] >= 9.);
        }
        let [_, quadrant] = left.subdivide(1).unwrap();
        assert_eq!(quadrant.domain(), [[0., 0.5], [0.5, 1.]]);
        let d = quadrant.derivative_bounds().unwrap();
        assert!(d.duv[2][0] <= 20.25 && d.duv[2][1] >= 20.25);
        let point = quadrant.point_bounds([[1., 1.], [0., 0.]]).unwrap();
        for ([lo, hi], value) in point.into_iter().zip([4.5, 4.5, 20.25]) {
            assert!(lo <= value && hi >= value);
        }
        assert!(matches!(cell.subdivide(2), Err(Error::Invalid)));
        let mut fine = cell.clone();
        for _ in 0..64 {
            fine = fine.subdivide(0).unwrap()[0].clone();
        }
        assert!(matches!(fine.subdivide(0), Err(Error::Limit)));
        let mut fine = cell;
        let mut refused = false;
        for _ in 0..64 {
            let next = fine.subdivide(0);
            if matches!(next, Err(Error::Unresolved)) {
                refused = true;
                break;
            }
            fine = next.unwrap()[1].clone();
        }
        assert!(refused);
        assert_eq!(
            t.support_bounds([0.; 3], f64::from_bits(1), None),
            Ok([0.; 2])
        );
    }
}
