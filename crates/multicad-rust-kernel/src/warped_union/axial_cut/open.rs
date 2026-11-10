//! Retain a blind circular segment where a planar crop opens a bore sideways.
use super::*;
use crate::warped_union::boss::{append_edge, oriented};
mod patch;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpenBoreSide {
    pub point_mm: Point,
    pub normal: Point,
}

struct Segment {
    normal: Point,
    tangent: Point,
    radius: f64,
    offset: f64,
    half_chord: f64,
    start_rad: f64,
    sweep_rad: f64,
}

impl Segment {
    fn new(tube: Tube, side: OpenBoreSide, tolerance: f64) -> Result<Self, Error> {
        let axis = tube.axes[2];
        let tilt = dot(axis, side.normal);
        if tilt.abs() > 64. * f64::EPSILON || tilt.abs() * tube.length > tolerance {
            return Err(Error::Unsupported);
        }
        let normal = std::array::from_fn(|i| side.normal[i] - tilt * axis[i]);
        let length = length3(normal);
        let normal = normal.map(|x| x / length);
        let offset = dot(sub(side.point_mm, tube.start), normal);
        let radius = tube.radius;
        if offset.abs() >= radius - tolerance {
            return Err(Error::Unsupported);
        }
        let start_rad = (offset / radius).acos();
        Ok(Self {
            normal,
            tangent: cross(axis, normal),
            radius,
            offset,
            half_chord: ((radius - offset) * (radius + offset)).sqrt(),
            start_rad,
            sweep_rad: 2. * (-offset / radius).acos(),
        })
    }

    fn chord(&self, center: Point) -> [Point; 2] {
        [1., -1.].map(|sign| {
            std::array::from_fn(|i| {
                center[i] + self.normal[i] * self.offset + sign * self.tangent[i] * self.half_chord
            })
        })
    }

    fn support(&self, direction: Point) -> f64 {
        let u = dot(direction, self.normal);
        let v = dot(direction, self.tangent);
        let length = u.hypot(v);
        if self.radius * u <= self.offset * length {
            self.radius * length
        } else {
            self.offset * u + self.half_chord * v.abs()
        }
    }

    fn area(&self) -> f64 {
        let angle = self.sweep_rad;
        // Evaluate the small segment without subtracting nearly equal
        // terms. The short-angle series is rounded at machine precision.
        let segment = if angle < 1e-3 {
            let squared = angle * angle;
            angle * squared * (1. / 6. + squared * (-1. / 120. + squared / 5040.))
        } else {
            angle - angle.sin()
        };
        0.5 * self.radius * self.radius * segment
    }
}

struct Opening {
    tube: Tube,
    entry: usize,
    roof: usize,
    entry_level: f64,
    common_edge: usize,
    entry_ends: [usize; 2],
    segment: Segment,
    floor_source_face: usize,
}

impl Shell {
    /// Reconstruct one open-sided blind bore on an otherwise planar chart.
    /// The side and entrance must share one straight edge across the notch.
    pub fn cut_open_axial(
        &self,
        cut: &AxialCut,
        mut side: OpenBoreSide,
        tolerance_mm: f64,
    ) -> Result<Self, Error> {
        if !(1e-10..=1e-3).contains(&tolerance_mm)
            || side
                .point_mm
                .into_iter()
                .any(|x| !x.is_finite() || x.abs() > 1e8)
            || side.normal.into_iter().any(|x| !x.is_finite())
        {
            return Err(Error::Invalid);
        }
        let normal_length = length3(side.normal);
        if !normal_length.is_finite() || normal_length == 0. {
            return Err(Error::Invalid);
        }
        if self.band.is_some() || cut.recess.is_some() {
            return Err(Error::Unsupported);
        }
        side.normal = side.normal.map(|x| x / normal_length);
        let tube = Tube::new(cut.start_mm, cut.end_mm, cut.radius_mm, tolerance_mm)?;
        let original = Opening::prove(self, tube, side, cut.cap_source_faces[1], tolerance_mm);
        let opening = original.or_else(|_| {
            Opening::prove(
                self,
                tube.reversed(),
                side,
                cut.cap_source_faces[0],
                tolerance_mm,
            )
        })?;
        let mut shell = self.clone();
        opening.append(&mut shell, cut)?;
        Ok(shell)
    }
}

impl Opening {
    fn prove(
        shell: &Shell,
        tube: Tube,
        side: OpenBoreSide,
        floor_source_face: usize,
        tolerance: f64,
    ) -> Result<Self, Error> {
        let segment = Segment::new(tube, side, tolerance)?;
        let mut candidate = None;
        for (entry, face) in shell.faces.iter().enumerate() {
            let Some((entry_level, true, entry_normal)) = tube.crossing(face, tolerance) else {
                continue;
            };
            if !straight_plane(shell, face) {
                continue;
            }
            let center = tube.point(entry_level);
            let chord = segment.chord(center);
            let floor = segment.chord(tube.point(tube.length));
            for (roof, roof_face) in shell.faces.iter().enumerate() {
                let Support::Plane { normal } = roof_face.support else {
                    continue;
                };
                if length3(sub(normal, side.normal)) > 64. * f64::EPSILON
                    || dot(sub(roof_face.point_mm, side.point_mm), side.normal).abs() > tolerance
                    || !straight_plane(shell, roof_face)
                {
                    continue;
                }
                let Some((common_edge, entry_ends)) =
                    shared_chord(shell, face, roof_face, chord, tolerance)
                else {
                    continue;
                };
                if !inside_edges(shell, face, entry_normal, common_edge, |a, inward| {
                    dot(sub(center, a), inward) - segment.support(inward.map(|x| -x)) > tolerance
                }) || !inside_edges(shell, roof_face, normal, common_edge, |a, inward| {
                    chord
                        .into_iter()
                        .chain(floor)
                        .all(|p| dot(sub(p, a), inward) > tolerance)
                }) {
                    continue;
                }
                if candidate
                    .replace((entry, roof, entry_level, common_edge, entry_ends))
                    .is_some()
                {
                    return Err(Error::Unsupported);
                }
            }
        }
        let (entry, roof, entry_level, common_edge, entry_ends) =
            candidate.ok_or(Error::Unsupported)?;
        if shell.faces.iter().enumerate().any(|(index, face)| {
            index != entry && index != roof && !tube.clears_face(shell, face, tolerance)
        }) {
            return Err(Error::Unsupported);
        }
        Ok(Self {
            tube,
            entry,
            roof,
            entry_level,
            common_edge,
            entry_ends,
            segment,
            floor_source_face,
        })
    }
}

fn straight_plane(shell: &Shell, face: &Face) -> bool {
    face.holes.is_empty()
        && face
            .boundary
            .iter()
            .all(|edge| shell.edges[edge.edge].curve == Curve::Line)
}

fn shared_chord(
    shell: &Shell,
    entry: &Face,
    roof: &Face,
    chord: [Point; 2],
    tolerance: f64,
) -> Option<(usize, [usize; 2])> {
    entry.boundary.iter().find_map(|&use_| {
        if !roof
            .boundary
            .iter()
            .any(|roof_use| roof_use.edge == use_.edge && roof_use.forward != use_.forward)
        {
            return None;
        }
        let ends = shell.oriented_vertices(use_);
        let [a, b] = ends.map(|i| shell.vertices[i]);
        let direction = sub(b, a);
        let length = length3(direction);
        let direction = direction.map(|x| x / length);
        let [start, end] = chord.map(|p| dot(sub(p, a), direction));
        (start > tolerance
            && end - start > tolerance
            && length - end > tolerance
            && chord
                .into_iter()
                .all(|p| length3(cross(sub(p, a), direction)) <= tolerance))
        .then_some((use_.edge, ends))
    })
}

fn inside_edges(
    shell: &Shell,
    face: &Face,
    normal: Point,
    exempt: usize,
    test: impl Fn(Point, Point) -> bool,
) -> bool {
    face.boundary.iter().all(|&use_| {
        if use_.edge == exempt {
            return true;
        }
        let [a, b] = shell.oriented_vertices(use_).map(|i| shell.vertices[i]);
        let inward = cross(normal, sub(b, a));
        let length = length3(inward);
        length > 0. && test(a, inward.map(|x| x / length))
    })
}
