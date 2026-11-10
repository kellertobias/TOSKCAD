use super::*;
use crate::{
    display::{DisplayMesh, MeshError, Tessellation, triangulate_region, wall},
    revolution::length,
    spur_gear::{clearance::segment_tubes_clear, scale, sub},
};
use spade::Point2;

const MAX_COLLISION_CHECKS: usize = 2_000_000;

struct Boundary {
    points: Vec<Point>,
    edges: Vec<[usize; 2]>,
    faces: Vec<u32>,
    holes: usize,
}

impl Boundary {
    fn new(gear: &Herringbone, options: Tessellation) -> Result<Self, MeshError> {
        let profile = gear.profile();
        let bore = profile.bore_radius_mm();
        let samples = profile.outline_samples(options, bore)?;
        let bore_count = if let Some(radius) = bore {
            options.arc_segments(radius, TAU)?.max(3)
        } else {
            0
        };
        let count = samples.len() + bore_count;
        let holes = usize::from(bore.is_some());
        // Minimum of one layer in each band plus the two end caps.
        if 6 * count + 4 * holes - 4 > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        let mut result = Self {
            points: Vec::with_capacity(count),
            edges: Vec::with_capacity(count),
            faces: Vec::with_capacity(count),
            holes,
        };
        result.ring(samples);
        if let Some(radius) = bore {
            let count = bore_count;
            let face = (2 + 2 * profile.outline().len()) as u32;
            result.ring((0..count).map(|index| {
                (
                    rotate([radius, 0.], -TAU * index as f64 / count as f64),
                    face,
                )
            }));
        }
        Ok(result)
    }

    fn ring(&mut self, samples: impl IntoIterator<Item = (Point, u32)>) {
        let start = self.points.len();
        for (point, face) in samples {
            let index = self.points.len();
            self.points.push(point);
            self.edges.push([index, index + 1]);
            self.faces.push(face);
        }
        self.edges.last_mut().expect("nonempty ring")[1] = start;
    }
}

struct Segment {
    indices: [usize; 2],
    ends: [Point; 2],
    length: f64,
    x: [f64; 2],
}

impl Segment {
    fn new(indices: [usize; 2], points: &[Point]) -> Result<Self, MeshError> {
        let ends = indices.map(|index| points[index]);
        let length = length(sub(ends[1], ends[0]));
        if length == 0. {
            return Err(MeshError::UnresolvedBoundary);
        }
        let x = ends.map(|p| p[0]);
        Ok(Self {
            indices,
            ends,
            length,
            x: [x[0].min(x[1]), x[0].max(x[1])],
        })
    }

    fn adjacent_at(&self, other: &Self) -> Option<Point> {
        self.indices
            .iter()
            .position(|i| other.indices.contains(i))
            .map(|i| self.ends[i])
    }

    fn away_from(&self, joint: Point) -> Point {
        let direction = if self.ends[0] == joint {
            sub(self.ends[1], joint)
        } else {
            sub(self.ends[0], joint)
        };
        scale(direction, self.length.recip())
    }
}

/// A transverse slice of one triangulated wall strip is a broken chord.
/// In the common affine frame interpolating the two ring rotations, its
/// diagonal moves by at most half the chord length times tan(angle/2).
/// Disjoint tubes prove separation for all nonadjacent chords. Adjacent
/// chords stay on opposite sides of their shared rotating bisector.
fn embedded_strip(
    segments: &[Segment],
    angle: f64,
    maximum_length: f64,
    guard: f64,
    remaining: &mut usize,
) -> Result<bool, MeshError> {
    let tube = 0.5 * (angle * 0.5).tan();
    let maximum_margin = 2. * tube * maximum_length + guard;
    let bisector_limit = 2. * angle.sin() + 1024. * f64::EPSILON;
    for (i, a) in segments.iter().enumerate() {
        for b in &segments[i + 1..] {
            if b.x[0] - a.x[1] > maximum_margin {
                break;
            }
            *remaining = remaining.checked_sub(1).ok_or(MeshError::BudgetExceeded)?;
            let clear = if let Some(joint) = a.adjacent_at(b) {
                length(sub(a.away_from(joint), b.away_from(joint))) > bisector_limit
            } else {
                segment_tubes_clear(a.ends, b.ends, tube * (a.length + b.length) + guard)
            };
            if !clear {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn certified_layers(
    segments: &[Segment],
    sweep: f64,
    maximum_length: f64,
    guard: f64,
    mut layers: usize,
    budget: usize,
) -> Result<usize, MeshError> {
    let mut remaining = MAX_COLLISION_CHECKS;
    loop {
        let angle = sweep / layers as f64;
        if embedded_strip(segments, angle, maximum_length, guard, &mut remaining)? {
            return Ok(layers);
        }
        layers *= 2;
        if layers > budget {
            return Err(MeshError::BudgetExceeded);
        }
    }
}

impl Herringbone {
    /// An indexed closed mesh with both end caps, one shared apex ring and
    /// the analytic face ordinals. Profile chord error, axial sagitta and
    /// the twist diagonal each consume at most a third of the deflection.
    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let section = Tessellation {
            deflection: options.deflection / 3.,
            angle: options.angle / 2.,
            ..options
        };
        let boundary = Boundary::new(self, section)?;
        let count = boundary.points.len();
        let caps_count = count + 2 * boundary.holes - 2;
        let mut segments = boundary
            .edges
            .iter()
            .map(|&indices| Segment::new(indices, &boundary.points))
            .collect::<Result<Vec<_>, _>>()?;
        segments.sort_unstable_by(|a, b| a.x[0].total_cmp(&b.x[0]));
        let max_length = segments.iter().map(|s| s.length).fold(0., f64::max);
        let height = self.profile.height_mm();
        let half = height * 0.5;
        let sweep = self.phase_at(half).abs();
        let max_angle = (8. * section.deflection / self.tip_radius_mm)
            .sqrt()
            .min(section.angle)
            .min(4. * section.deflection / max_length);
        let required = (sweep / max_angle).ceil().max(1.);
        let layer_budget = options.triangle_limit.saturating_sub(2 * caps_count) / (4 * count);
        if required > layer_budget as f64 {
            return Err(MeshError::BudgetExceeded);
        }
        let guard = 1024. * f64::EPSILON * self.tip_radius_mm.max(1.);
        let refinement = certified_layers(
            &segments,
            sweep,
            max_length,
            guard,
            required as usize,
            layer_budget,
        );
        let layers = refinement?;
        let points = boundary
            .points
            .iter()
            .map(|&[x, y]| Point2::new(x, y))
            .collect();
        let region = triangulate_region(points, &boundary.edges, boundary.holes)?;
        let cap_top = 2 * layers * count;
        let triangles = 2 * caps_count + 4 * layers * count;
        let mut mesh = DisplayMesh {
            vertices: Vec::with_capacity((2 * layers + 1) * count * 3),
            indices: Vec::with_capacity(triangles * 3),
            face_indices: Vec::with_capacity(triangles),
        };
        for layer in 0..=2 * layers {
            let z = height * (layer as f64 / (2 * layers) as f64);
            for point in &boundary.points {
                mesh.vertices.extend(self.section_point(*point, z));
            }
        }
        for [a, b, c] in region.triangles {
            mesh.triangle([c, b, a], 0);
            mesh.triangle([cap_top + a, cap_top + b, cap_top + c], 1);
        }
        let band_faces = self.profile.outline().len() as u32;
        let bore_face = 2 + 2 * band_faces;
        for layer in 0..2 * layers {
            for (&[a, b], &face) in boundary.edges.iter().zip(&boundary.faces) {
                let start = layer * count;
                let end = start + count;
                let face = if layer >= layers && face != bore_face {
                    face + band_faces
                } else {
                    face
                };
                wall(&mut mesh, [start + a, start + b], [end + a, end + b], face);
            }
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_requires_separated_tubes_and_budget_for_refinement() {
        let points = [[0., 0.], [1., 0.], [0., 0.03], [1., 0.03]];
        assert_eq!(
            Segment::new([0, 0], &points).err(),
            Some(MeshError::UnresolvedBoundary)
        );
        let segments = [[0, 1], [2, 3]].map(|edge| Segment::new(edge, &points).unwrap());
        let guard = 1e-12;
        assert_eq!(
            embedded_strip(&segments, 0.4, 1., guard, &mut 0),
            Err(MeshError::BudgetExceeded)
        );
        assert_eq!(
            certified_layers(&segments, 0.4, 1., guard, 1, 1),
            Err(MeshError::BudgetExceeded)
        );
        assert_eq!(certified_layers(&segments, 0.4, 1., guard, 1, 16), Ok(8));
    }
}
