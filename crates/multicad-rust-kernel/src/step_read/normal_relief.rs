//! Independent finite-graph proof and inference for analytic cap relief.
use super::*;
use crate::normal_relief::{Boundary, Curve as ReliefCurve, Relief, Surface};
use crate::normal_sweep::Edge as ProfileEdge;
mod proof;
mod recognize;
#[cfg(test)]
mod tests;

const MAX_WORK: usize = 24_000_000;
struct Work(usize);
impl Work {
    fn charge(&mut self, amount: usize) -> Result<()> {
        self.0 = self.0.checked_sub(amount).ok_or(ReadError::Limit)?;
        Ok(())
    }
}
fn require(value: bool) -> Result<()> {
    value.then_some(()).ok_or(ReadError::Invalid)
}
#[derive(Clone, Copy)]
struct Frame {
    origin: Point,
    axes: [Point; 3],
}
impl Frame {
    fn new(origin: Point, u: Point, v: Point) -> Result<Self> {
        require(
            origin.into_iter().chain(u).chain(v).all(f64::is_finite)
                && (dot(u, u) - 1.).abs() <= 1e-10
                && (dot(v, v) - 1.).abs() <= 1e-10
                && dot(u, v).abs() <= 1e-10,
        )?;
        Ok(Self {
            origin,
            axes: [u, v, cross(u, v)],
        })
    }
    fn vector(self, [x, y, z]: Point) -> Point {
        std::array::from_fn(|i| self.axes[0][i] * x + self.axes[1][i] * y + self.axes[2][i] * z)
    }
    fn world(self, [x, y, z]: Point) -> Point {
        std::array::from_fn(|i| {
            self.origin[i] + self.axes[0][i] * x + self.axes[1][i] * y + self.axes[2][i] * z
        })
    }
    fn local(self, point: Point) -> Point {
        let delta = sub(point, self.origin);
        let [u, v, n] = self.axes;
        let determinant = dot(u, cross(v, n));
        [cross(v, n), cross(n, u), cross(u, v)].map(|row| dot(row, delta) / determinant)
    }
}
/// Verify every physical vertex, analytic curve, support and shared oriented
/// wire of a known relief. No source metadata or tessellation is trusted.
pub fn verify_normal_relief(
    input: &str,
    relief: &Relief,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let frame = Frame::new(origin, u, v)?;
    compound::read_members_by(input, tolerance, true, true, |shell, solid, _| {
        let ids = shell.member_faces(solid)?;
        let faces = shell.validate_faces(ids.clone())?;
        let readback = shell.readback(&faces)?;
        proof::verify(
            shell,
            &ids,
            &faces,
            &readback,
            relief,
            frame,
            &mut Work(MAX_WORK),
        )?;
        Ok(readback.volume_mm3)
    })?
    .pop()
    .ok_or(ReadError::Invalid)
}

type Wire = Vec<(u32, bool)>;
fn cyclic(mut wire: Wire) -> Wire {
    if let Some((first, _)) = wire.iter().enumerate().min_by_key(|(_, edge)| **edge) {
        wire.rotate_left(first);
    }
    wire
}
fn loops(shell: &Shell<'_, '_>, id: u32, work: &mut Work) -> Result<Vec<Wire>> {
    let [Value::String(_), bounds, _, _] = shell.args(id, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    let bounds = refs(bounds)?;
    work.charge(bounds.len())?;
    bounds
        .into_iter()
        .map(|bound| {
            let (_, wire, forward) = shell.bound(bound, false)?;
            let [Value::String(_), edges] = shell.args(wire, "EDGE_LOOP")? else {
                return Err(ReadError::Invalid);
            };
            let edges = refs(edges)?;
            work.charge(edges.len())?;
            let mut result = edges
                .into_iter()
                .map(|edge| {
                    let [
                        Value::String(_),
                        Value::Derived,
                        Value::Derived,
                        id,
                        orientation,
                    ] = shell.args(edge, "ORIENTED_EDGE")?
                    else {
                        return Err(ReadError::Invalid);
                    };
                    Ok((reference(id)?, sense(orientation)? == forward))
                })
                .collect::<Result<Wire>>()?;
            if !forward {
                result.reverse();
            }
            Ok(result)
        })
        .collect()
}
pub(super) fn recognize(
    shell: &Shell<'_, '_>,
    ids: &[u32],
    faces: &[Face],
    readback: &Readback,
) -> Result<Option<ImportSolid>> {
    recognize::recognize(shell, ids, faces, readback)
}
