//! Reconstruct a full meridian from the verified rim identities of a STEP shell.
use super::*;
use crate::{polygon::MAX_VERTICES, revolution::Revolution};
use std::f64::consts::{PI, TAU};

type MeridianPoint = [f64; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Rim(u32),
    Axis(usize),
}

impl Shell<'_, '_> {
    pub(super) fn cylindrical_revolution(
        &self,
        faces: &[Face],
        readback: &Readback,
    ) -> Option<ImportSolid> {
        if faces.len() > MAX_VERTICES {
            return None;
        }
        let first = readback.circles.first()?;
        let axis_sign = if preferred_axis(first.normal) {
            1.
        } else {
            -1.
        };
        let axis = first.normal.map(|v| zero(axis_sign * v));
        let origin =
            readback
                .circles
                .iter()
                .map(|circle| circle.center_mm)
                .fold(first.center_mm, |a, b| {
                    let order = dot(sub(a, first.center_mm), axis)
                        .total_cmp(&dot(sub(b, first.center_mm), axis))
                        .then_with(|| compare(a, b));
                    if order.is_gt() { b } else { a }
                });
        let mut points = BTreeMap::new();
        for (&id, edge) in &self.edges {
            if let Curve::Circle(circle) = edge.curve {
                let delta = sub(circle.center_mm, origin);
                if length(cross(delta, axis)) > self.tolerance
                    || circle.radius_mm * length(cross(circle.normal, axis)) > self.tolerance
                {
                    return None;
                }
                points.insert(Node::Rim(id), [circle.radius_mm, zero(dot(delta, axis))]);
            }
        }
        let face_edges = self.indexed_edges(faces.len());
        let mut links = BTreeMap::new();
        let mut axis_nodes = Vec::new();
        for (i, (face, trims)) in faces.iter().zip(face_edges).enumerate() {
            let rims = trims.into_iter().map(Node::Rim);
            let rim_point = |rim| points.get(&rim).copied();
            let [from, to] = match face {
                Face::Cylindrical { seam, flux } => {
                    let rims: Vec<_> = rims.filter(|node| *node != Node::Rim(*seam)).collect();
                    let [a, b] = *rims.as_slice() else {
                        return None;
                    };
                    let [[ra, za], [rb, zb]] = [rim_point(a)?, rim_point(b)?];
                    if ra != rb || (za - zb).abs() <= self.tolerance {
                        return None;
                    }
                    if (za < zb) == (*flux > 0.) {
                        [a, b]
                    } else {
                        [b, a]
                    }
                }
                Face::Planar { normal, .. } => {
                    let rings: Vec<_> = rims.collect();
                    let (outer, inner) = match *rings.as_slice() {
                        [outer] => {
                            let [_, level] = rim_point(outer)?;
                            let inner = Node::Axis(i);
                            points.insert(inner, [0., level]);
                            axis_nodes.push(inner);
                            (outer, inner)
                        }
                        [a, b] => {
                            let [[ra, za], [rb, zb]] = [rim_point(a)?, rim_point(b)?];
                            if za != zb || (ra - rb).abs() <= self.tolerance {
                                return None;
                            }
                            if ra > rb { (a, b) } else { (b, a) }
                        }
                        _ => return None,
                    };
                    let radius = points[&outer][0];
                    if radius * length(cross(*normal, axis)) > self.tolerance {
                        return None;
                    }
                    if dot(*normal, axis) > 0. {
                        [outer, inner]
                    } else {
                        [inner, outer]
                    }
                }
                _ => return None,
            };
            if links.insert(from, to).is_some() {
                return None;
            }
        }
        close_axis(&axis_nodes, &points, &mut links, self.tolerance)?;
        let meridian = cycle(&points, &links)?;
        let revolution = Revolution::new(vec![meridian.clone()], TAU).ok()?;
        let [u, v] = frame_axes(axis, reference_axis(axis));
        if !matches_readback(
            &revolution,
            &meridian,
            origin,
            [u, v, axis],
            readback,
            self.tolerance,
        ) {
            return None;
        }
        let height_mm = meridian.iter().map(|p| p[1]).fold(0., f64::max);
        Some(ImportSolid {
            origin_mm: origin.map(zero),
            u,
            v,
            height_mm,
            profile: ImportProfile::CylindricalRevolution { meridian },
        })
    }
}

// An axial interval forms no STEP face. A single shell can either stay clear
// of the axis or close along it between two disk faces.
fn close_axis(
    nodes: &[Node],
    points: &BTreeMap<Node, MeridianPoint>,
    links: &mut BTreeMap<Node, Node>,
    tolerance: f64,
) -> Option<()> {
    match *nodes {
        [] => Some(()),
        [a, b] => {
            let za = points[&a][1];
            let zb = points[&b][1];
            let [upper, lower] = if za > zb { [a, b] } else { [b, a] };
            ((za - zb).abs() > tolerance && links.insert(upper, lower).is_none()).then_some(())
        }
        _ => None,
    }
}

fn cycle(
    points: &BTreeMap<Node, MeridianPoint>,
    links: &BTreeMap<Node, Node>,
) -> Option<Vec<MeridianPoint>> {
    if points.len() != links.len() || points.len() > MAX_VERTICES {
        return None;
    }
    let (&start, _) = points.iter().min_by(|a, b| {
        let [ra, za] = *a.1;
        let [rb, zb] = *b.1;
        za.total_cmp(&zb).then(ra.total_cmp(&rb))
    })?;
    let mut ring = Vec::with_capacity(links.len());
    let mut current = start;
    for _ in 0..links.len() {
        ring.push(*points.get(&current)?);
        current = *links.get(&current)?;
        if current == start {
            return (ring.len() == links.len()).then_some(ring);
        }
    }
    None
}

fn matches_readback(
    revolution: &Revolution,
    meridian: &[MeridianPoint],
    origin: Point,
    axes: [Point; 3],
    readback: &Readback,
    tolerance: f64,
) -> bool {
    let surface_area: f64 = meridian
        .iter()
        .zip(meridian.iter().cycle().skip(1))
        .map(|(&[a, z0], &[b, z1])| PI * (a + b) * (a - b).hypot(z1 - z0))
        .sum();
    let volume = revolution.volume_mm3();
    if (volume - readback.volume_mm3).abs()
        > 2. * tolerance * surface_area + 64. * f64::EPSILON * volume
    {
        return false;
    }
    [false, true].into_iter().enumerate().all(|(bound, upper)| {
        let sign = if upper { 1. } else { -1. };
        origin.into_iter().enumerate().all(|(i, datum)| {
            let offset = revolution.support(axes.map(|axis| sign * axis[i]));
            (datum + sign * offset - readback.bounds_mm[bound][i]).abs() <= 2. * tolerance
        })
    })
}

#[cfg(test)]
mod tests;
