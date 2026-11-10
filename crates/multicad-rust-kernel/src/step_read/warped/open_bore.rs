//! Restore the one straight boundary shared by an opened blind bore's two
//! planar mouths, then verify the complete STEP shell against a fresh cut.
use super::*;
use crate::warped_union::{AxialCut, OpenBoreSide};
use axial_cuts::{Incidence, verify};

type EdgeUse = (u32, bool);

struct RoundEnd {
    center: Point,
    edges: BTreeSet<u32>,
}

struct Bore {
    entry: usize,
    floor: usize,
    roof: usize,
    side: OpenBoreSide,
    entry_rim: RoundEnd,
    floor_rim: RoundEnd,
    chord: u32,
    rails: BTreeSet<u32>,
    walls: BTreeSet<usize>,
    radius: f64,
}

pub(super) fn reconstruct(
    reader: &Shell<'_, '_>,
    source: &Boundary,
) -> Result<Option<ImportedWarped>> {
    let tol = reader.tolerance;
    let Some(bore) = Bore::read(source, tol)? else {
        return Ok(None);
    };
    let rings = bore.restore(reader, source)?;
    let union = Union::verify_boundary(&rings, tol).map_err(boundary_error)?;
    let stock = warped_union::from_planar_union(&union, tol).map_err(axial_cuts::replay_error)?;
    let mouth = bore.entry_rim.center;
    let floor = bore.floor_rim.center;
    let axis = unit(sub(floor, mouth))?;
    let start_mm = std::array::from_fn(|i| mouth[i] - axis[i] * 4. * tol);
    let cut = AxialCut {
        start_mm,
        end_mm: floor,
        radius_mm: bore.radius,
        source: 0,
        wall_source_face: 0,
        cap_source_faces: [1, 2],
        recess: None,
    };
    let shell = stock
        .cut_open_axial(&cut, bore.side, tol)
        .map_err(axial_cuts::replay_error)?;
    let origin_mm = [0.; 3];
    let u = [1., 0., 0.];
    let v = [0., 1., 0.];
    // Only rigid placement participates in the replay comparison.
    let chart = Chart {
        origin: origin_mm,
        u,
        v,
        up: [0., 0., 1.],
        band: Band {
            center_z_mm: 0.,
            radius_mm: 1.,
        },
    };
    verify::replayed(reader, source, chart, &shell)?;
    Ok(Some(ImportedWarped {
        shell,
        origin_mm,
        u,
        v,
    }))
}

fn boundary_error(cause: planar_boolean::Error) -> ReadError {
    match cause {
        planar_boolean::Error::Limit => ReadError::Limit,
        _ => ReadError::Invalid,
    }
}

impl Bore {
    fn read(source: &Boundary, tol: f64) -> Result<Option<Self>> {
        if source.faces.iter().any(|face| !face.holes.is_empty()) {
            return Ok(None);
        }
        let walls: BTreeSet<_> = source
            .faces
            .iter()
            .enumerate()
            .filter_map(|(i, face)| matches!(face.surface, Surface::Cylinder { .. }).then_some(i))
            .collect();
        if !(1..=4).contains(&walls.len()) {
            return Ok(None);
        }
        let &first = walls.first().ok_or(ReadError::Invalid)?;
        let Surface::Cylinder {
            origin,
            axis,
            radius,
            outward: false,
        } = source.faces[first].surface
        else {
            return Ok(None);
        };
        let incidence = Incidence::new(source);
        let mut ends = BTreeMap::<usize, RoundEnd>::new();
        let mut exposed_rails = BTreeMap::<usize, BTreeSet<_>>::new();
        for &index in &walls {
            let wall = &source.faces[index];
            let Surface::Cylinder {
                origin: axis_point,
                axis: direction,
                radius: r,
                outward: false,
            } = wall.surface
            else {
                return Ok(None);
            };
            if wall.loop_.len() != 4
                || length(cross(axis, direction)) > 1e-10
                || length(cross(sub(axis_point, origin), axis)) > tol
                || (radius - r).abs() > tol
            {
                return Ok(None);
            }
            let mut arcs = 0;
            for &(edge, _) in &wall.loop_ {
                let neighbor = incidence.adjacent(edge, index)?.face;
                match source.curves[&edge].1 {
                    TrimCurve::Line if !walls.contains(&neighbor) => {
                        exposed_rails.entry(neighbor).or_default().insert(edge);
                    }
                    TrimCurve::Line => {}
                    TrimCurve::Arc(arc) => {
                        arcs += 1;
                        let circle = arc.ellipse();
                        let center = circle.center_mm();
                        if walls.contains(&neighbor)
                            || circle
                                .semiaxes_mm()
                                .into_iter()
                                .any(|r| (radius - r).abs() > tol)
                            || length(cross(axis, circle.normal())) > 1e-10
                            || length(cross(sub(center, origin), axis)) > tol
                        {
                            return Ok(None);
                        }
                        let end = ends.entry(neighbor).or_insert_with(|| RoundEnd {
                            center,
                            edges: BTreeSet::new(),
                        });
                        if length(sub(end.center, center)) > tol || !end.edges.insert(edge) {
                            return Ok(None);
                        }
                    }
                }
            }
            if arcs != 2 {
                return Ok(None);
            }
        }
        let Some((roof, side, rails)) = single_roof(source, exposed_rails) else {
            return Ok(None);
        };
        let mut end_faces = ends.into_iter();
        let (Some(a), Some(b), None) = (end_faces.next(), end_faces.next(), end_faces.next())
        else {
            return Ok(None);
        };
        let chord = |(index, end): &(usize, RoundEnd)| -> Option<u32> {
            let loop_ = &source.faces[*index].loop_;
            if loop_.len() != end.edges.len() + 1 {
                return None;
            }
            let &(edge, _) = loop_.iter().find(|(edge, _)| !end.edges.contains(edge))?;
            matches!(source.curves[&edge].1, TrimCurve::Line).then_some(edge)
        };
        let ((floor, floor_rim), (entry, entry_rim), chord) = match (chord(&a), chord(&b)) {
            (Some(chord), None) => (a, b, chord),
            (None, Some(chord)) => (b, a, chord),
            _ => return Ok(None),
        };
        if incidence.adjacent(chord, floor)?.face != roof
            || [entry, floor].contains(&roof)
            || [entry_rim.edges.len(), floor_rim.edges.len()] != [walls.len(); 2]
            || length(sub(floor_rim.center, entry_rim.center)) <= tol
        {
            return Ok(None);
        }
        Ok(Some(Self {
            entry,
            floor,
            roof,
            side,
            entry_rim,
            floor_rim,
            chord,
            rails,
            walls,
            radius,
        }))
    }

    fn restore(&self, reader: &Shell<'_, '_>, source: &Boundary) -> Result<Vec<Vec<Point>>> {
        let entry_wire = &source.faces[self.entry].loop_;
        let [first, last] = span(entry_wire, &self.entry_rim.edges)?;
        let [left, right] = [
            entry_wire[(first + entry_wire.len() - 1) % entry_wire.len()],
            entry_wire[(last + 1) % entry_wire.len()],
        ];
        let ends = [left, right].map(|(edge, forward)| {
            let (vertices, curve) = source.curves[&edge];
            (vertices, curve, forward)
        });
        let [
            (a, TrimCurve::Line, forward_a),
            (b, TrimCurve::Line, forward_b),
        ] = ends
        else {
            return Err(ReadError::Invalid);
        };
        let shared = [a[usize::from(!forward_a)], b[usize::from(forward_b)]];
        let roof_notch = self
            .rails
            .iter()
            .copied()
            .chain([self.chord, left.0, right.0])
            .collect();
        let entry_notch = self
            .entry_rim
            .edges
            .iter()
            .copied()
            .chain([left.0, right.0])
            .collect();
        let make_wire = |face: usize, notch, forward| {
            let mut result = vec![(left.0, forward)];
            result.extend(omit_span(&source.faces[face].loop_, notch)?);
            Ok::<_, ReadError>(result)
        };
        let entry = make_wire(self.entry, &entry_notch, true)?;
        let roof = make_wire(self.roof, &roof_notch, false)?;
        let mut rings = Vec::new();
        for (index, face) in source.faces.iter().enumerate() {
            if index == self.floor || self.walls.contains(&index) {
                continue;
            }
            let wire = if index == self.entry {
                &entry
            } else if index == self.roof {
                &roof
            } else {
                &face.loop_
            };
            let vertices = wire
                .iter()
                .map(|&(edge, forward)| {
                    let (ends, curve) = source.curves[&edge];
                    if !matches!(curve, TrimCurve::Line) {
                        return Err(ReadError::Unsupported);
                    }
                    let ends = if edge == left.0 { shared } else { ends };
                    Ok(if forward { ends } else { [ends[1], ends[0]] })
                })
                .collect::<Result<Vec<_>>>()?;
            if vertices
                .iter()
                .zip(vertices.iter().cycle().skip(1))
                .any(|(a, b)| a[1] != b[0])
            {
                return Err(ReadError::Invalid);
            }
            rings.push(
                vertices
                    .into_iter()
                    .map(|[start, _]| reader.points[&start])
                    .collect(),
            );
        }
        Ok(rings)
    }
}

fn single_roof(
    source: &Boundary,
    exposed: BTreeMap<usize, BTreeSet<u32>>,
) -> Option<(usize, OpenBoreSide, BTreeSet<u32>)> {
    let mut faces = exposed.into_iter();
    let (roof, rails) = faces.next()?;
    let Surface::Plane {
        origin: point_mm,
        normal,
    } = source.faces[roof].surface
    else {
        return None;
    };
    (faces.next().is_none() && rails.len() == 2).then_some((
        roof,
        OpenBoreSide { point_mm, normal },
        rails,
    ))
}

fn span(wire: &[EdgeUse], notch: &BTreeSet<u32>) -> Result<[usize; 2]> {
    if notch.is_empty() || notch.len() >= wire.len() {
        return Err(ReadError::Invalid);
    }
    let start = (0..wire.len())
        .find(|&i| {
            notch.contains(&wire[i].0)
                && !notch.contains(&wire[(i + wire.len() - 1) % wire.len()].0)
        })
        .ok_or(ReadError::Invalid)?;
    if !wire
        .iter()
        .cycle()
        .skip(start)
        .take(notch.len())
        .all(|(edge, _)| notch.contains(edge))
    {
        return Err(ReadError::Invalid);
    }
    Ok([start, (start + notch.len() - 1) % wire.len()])
}

fn omit_span(wire: &[EdgeUse], notch: &BTreeSet<u32>) -> Result<impl Iterator<Item = EdgeUse>> {
    let [_, last] = span(wire, notch)?;
    Ok(wire
        .iter()
        .cycle()
        .skip(last + 1)
        .take(wire.len() - notch.len())
        .copied())
}

#[cfg(test)]
mod tests;
