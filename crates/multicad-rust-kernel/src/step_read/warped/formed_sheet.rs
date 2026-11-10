//! Recover an inset sheet from its planar web and line/circle flange sweeps.
mod openings;

use super::{
    axial_cuts::{Incidence, verify},
    *,
};
use crate::{
    normal_sweep::Edge,
    warped_union::{SheetFlange, SheetFlangeOpening, SheetWebEdge, formed_sheet_with_openings},
};

const MAX_BEND_WALLS: usize = 4;

struct Wire<'a, 'reader, 'input> {
    reader: &'a Shell<'reader, 'input>,
    boundary: &'a Boundary,
}

impl Wire<'_, '_, '_> {
    fn points(&self, (edge, forward): (u32, bool)) -> [Point; 2] {
        let (mut ends, _) = self.boundary.curves[&edge];
        if !forward {
            ends.reverse();
        }
        ends.map(|id| self.reader.points[&id])
    }

    fn line(&self, usage: (u32, bool)) -> Result<[Point; 2]> {
        if !matches!(self.boundary.curves[&usage.0].1, TrimCurve::Line) {
            return Err(ReadError::Unsupported);
        }
        Ok(self.points(usage))
    }

    fn outline(
        &self,
        usage: (u32, bool),
        plane: [Point; 3],
        snap: impl Fn([f64; 2]) -> [f64; 2],
    ) -> Result<Edge> {
        let [origin, u, v] = plane;
        let normal = cross(u, v);
        let tolerance = self.reader.tolerance;
        let point = |point| {
            let delta = sub(point, origin);
            if dot(delta, normal).abs() > tolerance {
                return Err(ReadError::Invalid);
            }
            Ok([dot(delta, u), dot(delta, v)])
        };
        let [a, b] = self.points(usage).map(point);
        let (a, b) = (snap(a?), snap(b?));
        match self.boundary.curves[&usage.0].1 {
            TrimCurve::Line => Ok(Edge::Line { a, b }),
            TrimCurve::Arc(arc) => {
                let ellipse = arc.ellipse();
                let [major, minor] = ellipse.semiaxes_mm();
                let facing = dot(normal, ellipse.normal());
                if (major - minor).abs() > tolerance || facing.abs() < 1. - 1e-10 {
                    return Err(ReadError::Unsupported);
                }
                let clockwise = (facing * arc.sweep_rad() < 0.) == usage.1;
                Ok(Edge::Arc {
                    center: point(ellipse.center_mm())?,
                    a,
                    b,
                    clockwise,
                })
            }
        }
    }

    fn normal(&self, face: usize) -> Result<Point> {
        match self.boundary.faces[face].surface {
            Surface::Plane { normal, .. } => Ok(normal),
            _ => Err(ReadError::Unsupported),
        }
    }

    fn bend(&self, face: usize, incidence: &Incidence) -> Result<Option<Bend>> {
        let boundary = &self.boundary.faces[face];
        let Surface::Cylinder { axis, radius, .. } = boundary.surface else {
            return Ok(None);
        };
        if boundary.loop_.len() != 4 || !boundary.holes.is_empty() {
            return Ok(None);
        }
        let (mut rails, mut caps) = (Vec::with_capacity(2), Vec::with_capacity(2));
        for &(id, forward) in &boundary.loop_ {
            let other = incidence.adjacent(id, face)?;
            if other.hole.is_some() || self.normal(other.face).is_err() {
                return Ok(None);
            }
            match self.boundary.curves[&id].1 {
                TrimCurve::Line => {
                    let [a, b] = self.points((id, forward));
                    if length(cross(unit(sub(b, a))?, axis)) > 1e-10 {
                        return Ok(None);
                    }
                    rails.push((id, other.face));
                }
                TrimCurve::Arc(arc) => {
                    if (arc.sweep_rad().abs() - FRAC_PI_2).abs() * radius > self.reader.tolerance
                        || arc
                            .ellipse()
                            .semiaxes_mm()
                            .iter()
                            .any(|r| (r - radius).abs() > self.reader.tolerance)
                        || length(cross(self.normal(other.face)?, axis)) > 1e-10
                    {
                        return Ok(None);
                    }
                    caps.push(other.face);
                }
            }
        }
        let (Ok(rails), Ok(caps)) = (rails.try_into(), caps.try_into()) else {
            return Ok(None);
        };
        Ok(Some(Bend { rails, caps }))
    }
}

struct Bend {
    rails: [(u32, usize); 2],
    caps: [usize; 2],
}

fn reverse(wire: &[(u32, bool)]) -> impl Iterator<Item = (u32, bool)> + Clone + '_ {
    wire.iter().rev().map(|&(edge, forward)| (edge, !forward))
}

pub(super) fn reconstruct(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
) -> Result<Option<ImportedWarped>> {
    let wire = Wire { reader, boundary };
    let incidence = Incidence::new(boundary);
    let bends = (0..boundary.faces.len())
        .filter_map(|i| wire.bend(i, &incidence).transpose())
        .take(MAX_BEND_WALLS + 1)
        .collect::<Result<Vec<_>>>()?;
    if !matches!(bends.len(), 2 | 4) {
        return Ok(None);
    }
    let candidates = bends
        .iter()
        .flat_map(|bend| bend.rails.map(|(_, host)| host))
        .collect::<BTreeSet<_>>();
    for &lower in &candidates {
        let bottom = &boundary.faces[lower];
        for &upper in &candidates {
            if lower == upper || bottom.loop_.len() != boundary.faces[upper].loop_.len() {
                continue;
            }
            let result = replay(&wire, &incidence, &bends, lower, upper);
            if matches!(result, Ok(_) | Err(ReadError::Limit)) {
                return result.map(Some);
            }
        }
    }
    Ok(None)
}

fn replay(
    wire: &Wire<'_, '_, '_>,
    incidence: &Incidence,
    bends: &[Bend],
    lower: usize,
    upper: usize,
) -> Result<ImportedWarped> {
    let bottom = &wire.boundary.faces[lower];
    let top = &wire.boundary.faces[upper];
    let tol = wire.reader.tolerance;
    let up = wire.normal(upper)?;
    if dot(up, wire.normal(lower)?) > -1. + 1e-10 || bottom.holes.len() != top.holes.len() {
        return Err(ReadError::Unsupported);
    }
    let [origin, end] = wire.line((bottom.loop_[0].0, !bottom.loop_[0].1))?;
    let u = unit(sub(end, origin))?;
    let v = unit(cross(up, u))?;
    let u = cross(v, up);
    let chart = Chart {
        origin,
        u,
        v,
        up,
        // Only the affine placement is used for replay verification.
        band: Band {
            center_z_mm: 0.,
            radius_mm: 1.,
        },
    };
    let height = dot(sub(wire.points(top.loop_[0])[0], origin), up);
    if height <= tol || !paired_outline(wire, &bottom.loop_, &top.loop_, up.map(|n| n * height)) {
        return Err(ReadError::Unsupported);
    }

    let mut web = Vec::with_capacity(bottom.loop_.len());
    let mut sections = Vec::new();
    for (ordinal, usage) in reverse(&bottom.loop_).enumerate() {
        let [a, b] = wire.line(usage)?;
        let hinge = if let Some(bend) = bends
            .iter()
            .find(|bend| bend.rails.contains(&(usage.0, lower)))
        {
            let index = sections.len();
            sections.push(flange_section(wire, incidence, bend, [a, b], up, height)?);
            Some(index)
        } else {
            None
        };
        web.push(SheetWebEdge {
            ends: [a, b].map(|p| {
                let [x, y, _] = chart.local(p);
                [x, y]
            }),
            source: ordinal,
            flange: hinge,
        });
    }
    if sections.len() * 2 != bends.len() {
        return Err(ReadError::Unsupported);
    }
    let flanges = sections
        .iter()
        .map(|(section, ends)| SheetFlange {
            section,
            ends: *ends,
        })
        .collect::<Vec<_>>();
    let holes = bottom
        .holes
        .iter()
        .map(|hole| {
            reverse(hole)
                .map(|usage| {
                    wire.outline(usage, [origin, u, v], std::convert::identity)
                        .map(|edge| (edge, 0))
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let openings = sections
        .iter()
        .map(|(section, _)| openings::read(wire, incidence, chart, section))
        .collect::<Result<Vec<_>>>()?;
    let specs = openings
        .iter()
        .map(|holes| {
            holes
                .iter()
                .map(openings::Opening::spec)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let specs = specs.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let shell = formed_sheet_with_openings(
        &web,
        &holes,
        [lower, upper],
        &flanges,
        &specs,
        height,
        tol,
        2. * tol,
    )
    .map_err(axial_cuts::replay_error)?;
    verify::replayed(wire.reader, wire.boundary, chart, &shell)?;
    Ok(ImportedWarped {
        shell,
        origin_mm: origin,
        u,
        v,
    })
}

fn paired_outline(
    wire: &Wire<'_, '_, '_>,
    a: &[(u32, bool)],
    b: &[(u32, bool)],
    movement: Point,
) -> bool {
    let shift = |p: Point| std::array::from_fn(|i| p[i] + movement[i]);
    let a = reverse(a);
    let first = wire.points(a.clone().next().expect("validated loop"))[0];
    b.iter()
        .position(|&edge| length(sub(wire.points(edge)[0], shift(first))) <= wire.reader.tolerance)
        .is_some_and(|start| {
            a.zip(b.iter().cycle().skip(start)).all(|(a, &b)| {
                wire.points(a)
                    .into_iter()
                    .zip(wire.points(b))
                    .all(|(a, b)| length(sub(shift(a), b)) <= wire.reader.tolerance)
            })
        })
}

type Section = (Vec<(Edge, usize)>, [usize; 2]);

fn flange_section(
    wire: &Wire<'_, '_, '_>,
    incidence: &Incidence,
    bend: &Bend,
    [a, b]: [Point; 2],
    up: Point,
    thickness: f64,
) -> Result<Section> {
    let along = unit(sub(b, a))?;
    let outward = cross(along, up);
    let [left, right] = bend.caps;
    let ends = if dot(wire.normal(left)?, along) < 0. {
        [left, right]
    } else {
        [right, left]
    };
    let cap = &wire.boundary.faces[ends[0]];
    let lifted = std::array::from_fn(|i| a[i] + up[i] * thickness);
    let tol = wire.reader.tolerance;
    if !cap.holes.is_empty() {
        return Err(ReadError::Unsupported);
    }
    let root = cap
        .loop_
        .iter()
        .position(|&usage| {
            matches!(wire.boundary.curves[&usage.0].1, TrimCurve::Line)
                && wire
                    .points(usage)
                    .into_iter()
                    .zip([lifted, a])
                    .all(|(point, expected)| length(sub(point, expected)) <= tol)
        })
        .ok_or(ReadError::Unsupported)?;
    let upper = [0., thickness];
    let lower = [0.; 2];
    let exact = |point| {
        [lower, upper]
            .into_iter()
            .find(|&corner| length2(point, corner) <= tol)
            .unwrap_or(point)
    };
    let section = cap
        .loop_
        .iter()
        .cycle()
        .skip(root + 1)
        .take(cap.loop_.len())
        .map(|&usage| {
            let edge = wire.outline(usage, [a, outward, up], exact)?;
            let host = incidence.adjacent(usage.0, ends[0])?.face;
            Ok((edge, host))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((section, ends))
}

fn length2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[cfg(test)]
mod tests;
