//! Whole-curve support and exact iso-parametric boundary correspondence.
use super::*;
#[cfg(test)]
mod tests;
const D: usize = 9;
#[derive(Clone)]
pub struct TrimmedBoundary {
    pub(super) ownership: BoundaryOwnership,
    pub(super) tensor_edges: Vec<Option<[usize; 4]>>,
    pub(super) tensors: Vec<BoundTensor>,
    pub(super) error_mm: f64,
}
#[derive(Clone)]
pub(super) struct BoundTensor {
    pub face: usize,
    pub surface: RetainedTensor,
    /// The four actual iso-edges, oriented in the tensor's natural UV wire.
    pub edges: [(usize, bool); 4],
}
impl TrimmedBoundary {
    pub fn input(&self) -> &BoundaryInput {
        self.ownership.input()
    }
    pub fn boundary_error_mm(&self) -> f64 {
        self.error_mm
    }
    pub fn tensor_edges(&self) -> &[Option<[usize; 4]>] {
        &self.tensor_edges
    }
}
impl BoundaryOwnership {
    pub fn bind_surfaces(
        self,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<TrimmedBoundary, Error> {
        if !tolerance.is_finite() || tolerance <= 0. {
            return Err(Error::Invalid);
        }
        let mut work = Work {
            remaining: 2_000_000,
            interrupt,
        };
        work.step()?;
        if self.sewing_error_mm > tolerance / 16. {
            return Err(Error::Unresolved);
        }
        let mut error = self.sewing_error_mm;
        let mut tensor_edges = Vec::with_capacity(self.input.faces.len());
        let mut tensors = Vec::new();
        for (face_index, face) in self.input.faces.iter().enumerate() {
            work.step()?;
            let edges = match &self.input.surfaces[face.surface] {
                Surface::Tensor(t) => {
                    let edges = tensor_wire(&self.input, face, t, &mut work)?;
                    tensors.push(BoundTensor {
                        face: face_index,
                        surface: t.clone(),
                        edges,
                    });
                    Some(edges.map(|edge| edge.0))
                }
                Surface::Plane(axis) => {
                    for &(e, _) in &face.outer {
                        error = error.max(plane_curve(
                            &self.input.curves[self.input.edges[e].curve],
                            *axis,
                            &mut work,
                        )?);
                    }
                    None
                }
                Surface::Cone {
                    axis,
                    radius_mm,
                    semi_angle_rad,
                } => {
                    for &(e, _) in &face.outer {
                        error = error.max(cone_curve(
                            &self.input.curves[self.input.edges[e].curve],
                            *axis,
                            *radius_mm,
                            *semi_angle_rad,
                            tolerance / 16.,
                            &mut work,
                        )?);
                    }
                    None
                }
            };
            if error > tolerance / 16. {
                return Err(Error::Unresolved);
            }
            tensor_edges.push(edges);
        }
        Ok(TrimmedBoundary {
            ownership: self,
            tensor_edges,
            tensors,
            error_mm: error,
        })
    }
}
fn tensor_wire(
    input: &BoundaryInput,
    face: &Face,
    t: &RetainedTensor,
    work: &mut Work<'_>,
) -> Result<[(usize, bool); 4], Error> {
    if face.outer.len() != 4 {
        return Err(Error::Invalid);
    }
    let [w, h] = t.dimensions();
    let p = t.poles();
    let natural = [
        (0..w).map(|i| p[i * h]).collect::<Vec<_>>(),
        p[(w - 1) * h..].to_vec(),
        (0..w).rev().map(|i| p[i * h + h - 1]).collect(),
        p[..h].iter().rev().copied().collect(),
    ];
    let wire = if face.forward {
        face.outer.clone()
    } else {
        face.outer.iter().rev().map(|&(e, f)| (e, !f)).collect()
    };
    let mut candidates = Vec::new();
    for offset in 0..4 {
        let mut matched = true;
        for (side, expected) in natural.iter().enumerate() {
            work.step()?;
            let (e, forward) = wire[(offset + side) % 4];
            let edge = input.edges[e];
            let Curve::Spline { poles, .. } = &input.curves[edge.curve] else {
                matched = false;
                break;
            };
            if poles.len() != expected.len() {
                matched = false;
                break;
            }
            for (chunk, points) in poles.chunks(128).zip(0..) {
                work.step()?;
                let base = points * 128;
                for (i, &point) in chunk.iter().enumerate() {
                    let index = base + i;
                    let target = if edge.forward == forward {
                        index
                    } else {
                        poles.len() - 1 - index
                    };
                    if point != expected[target] {
                        matched = false;
                        break;
                    }
                }
                if !matched {
                    break;
                }
            }
            if !matched {
                break;
            }
        }
        if matched {
            candidates.push(std::array::from_fn(|i| wire[(offset + i) % 4]));
        }
    }
    let [edges] = candidates[..] else {
        return Err(Error::Unresolved);
    };
    Ok(edges)
}
fn magnitude(v: [I; 3]) -> Result<f64, Error> {
    let bound = v
        .into_iter()
        .fold(I::point(0.), |s, x| s + I::point(x.magnitude()))
        .checked()?;
    Ok(bound.hi)
}
fn plane_curve(curve: &Curve, axis: Axis, work: &mut Work<'_>) -> Result<f64, Error> {
    let n = checked_axis(axis)?[2];
    let mut error = 0f64;
    match curve {
        Curve::Spline { poles, .. } => {
            for chunk in poles.chunks(128) {
                work.step()?;
                for &p in chunk {
                    let d = std::array::from_fn(|i| I::point(p[i]) - I::point(axis.origin[i]));
                    error = error.max(dot(d, n).magnitude());
                }
            }
        }
        Curve::Circle {
            axis: circle,
            radius_mm,
        } => {
            let d = std::array::from_fn(|i| I::point(circle.origin[i]) - I::point(axis.origin[i]));
            let sin = magnitude(cross(n, checked_axis(*circle)?[2]))?;
            error = (I::point(dot(d, n).magnitude()) + I::point(*radius_mm) * I::point(sin)).hi;
        }
    }
    if error.is_finite() {
        Ok(error)
    } else {
        Err(Error::Unresolved)
    }
}
fn cone_curve(
    curve: &Curve,
    axis: Axis,
    radius: f64,
    angle: f64,
    budget: f64,
    work: &mut Work<'_>,
) -> Result<f64, Error> {
    let n = checked_axis(axis)?[2];
    let slope = if angle == 0. {
        I::point(0.)
    } else {
        I::point(angle).sin() / I::point(angle).cos()
    };
    slope.checked()?;
    match curve {
        Curve::Circle {
            axis: circle,
            radius_mm,
        } => {
            let d = std::array::from_fn(|i| I::point(circle.origin[i]) - I::point(axis.origin[i]));
            let z = dot(d, n);
            let offset = magnitude(std::array::from_fn(|i| d[i] - n[i] * z))?;
            let sin = magnitude(cross(n, checked_axis(*circle)?[2]))?;
            let r = I::point(radius) + slope * z;
            r.positive()?;
            let error = I::point(offset)
                + I::point((I::point(*radius_mm) - r).magnitude())
                + I::point(*radius_mm)
                    * I::point(sin)
                    * (I::point(1.) + I::point(slope.magnitude()));
            error.checked()?;
            if error.hi <= budget {
                Ok(error.hi)
            } else {
                Err(Error::Unresolved)
            }
        }
        Curve::Spline { spans, poles } => {
            let mut maximum = 0f64;
            for i in 0..*spans {
                work.step()?;
                let p = poles[i * D..=i * D + D]
                    .iter()
                    .map(|p| std::array::from_fn(|j| I::point(p[j]) - I::point(axis.origin[j])))
                    .collect::<Vec<_>>();
                let mut pending = vec![(p, 0u32)];
                while let Some((p, depth)) = pending.pop() {
                    work.step()?;
                    let z = p.iter().map(|&p| dot(p, n)).collect::<Vec<_>>();
                    let r = z
                        .iter()
                        .map(|&z| I::point(radius) + slope * z)
                        .collect::<Vec<_>>();
                    let minimum = r.iter().map(|r| r.lo).fold(f64::INFINITY, f64::min);
                    if minimum <= 0. {
                        return Err(Error::Unresolved);
                    }
                    let mut g = vec![I::point(0.); 2 * D + 1];
                    for a in 0..=D {
                        work.step()?;
                        for b in 0..=D {
                            let weight = I::point(binomial(D, a) * binomial(D, b))
                                / I::point(binomial(2 * D, a + b));
                            g[a + b] =
                                g[a + b] + weight * (dot(p[a], p[b]) - z[a] * z[b] - r[a] * r[b]);
                        }
                    }
                    let bound = g.iter().map(|g| g.magnitude()).fold(0., f64::max);
                    let error = (I::point(bound) / I::point(minimum)).checked()?;
                    if error.hi <= budget {
                        maximum = maximum.max(error.hi);
                        continue;
                    }
                    if depth == 32 {
                        return Err(Error::Unresolved);
                    }
                    let (a, b) = split(&p);
                    pending.push((b, depth + 1));
                    pending.push((a, depth + 1));
                }
            }
            Ok(maximum)
        }
    }
}
pub(super) fn split(p: &[[I; 3]]) -> (Vec<[I; 3]>, Vec<[I; 3]>) {
    let mut p = p.to_vec();
    let mut a = vec![p[0]];
    let mut b = vec![p[p.len() - 1]];
    for n in (1..p.len()).rev() {
        for i in 0..n {
            p[i] = std::array::from_fn(|j| (p[i][j] + p[i + 1][j]) * I::point(0.5));
        }
        a.push(p[0]);
        b.push(p[n - 1]);
    }
    b.reverse();
    (a, b)
}
pub(super) fn binomial(n: usize, k: usize) -> f64 {
    let mut value = 1.;
    for i in 0..k {
        value = value * (n - i) as f64 / (i + 1) as f64;
    }
    value
}
