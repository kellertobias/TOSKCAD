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
    fn new(
        gear: &Herringbone,
        options: Tessellation,
        hub_radius: Option<f64>,
    ) -> Result<Self, MeshError> {
        let profile = gear.profile();
        let bore = profile.bore_radius_mm().filter(|_| hub_radius.is_none());
        let samples = profile.outline_samples(options, hub_radius.or(bore))?;
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
        let extensions = self.hub_extensions();
        // The tooth strips must also stay outside the continuous bore.
        // A narrow hub/root annulus bounds their entire construction error.
        let deflection =
            extensions
                .iter()
                .flatten()
                .next()
                .map_or(options.deflection / 3., |extension| {
                    (options.deflection / 3.)
                        .min((self.profile.root_radius_mm - extension.radius_mm) / 6.)
                });
        let section = Tessellation {
            deflection,
            angle: options.angle / 2.,
            ..options
        };
        let hubs = HubEnds::new(self, section)?;
        let boundary = Boundary::new(self, section, hubs.as_ref().map(|ends| ends.hub.radius))?;
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
        let cap_triangles = hubs
            .as_ref()
            .map_or(2 * caps_count, |ends| ends.triangles(count));
        let layer_budget = options.triangle_limit.saturating_sub(cap_triangles) / (4 * count);
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
        let cap_top = 2 * layers * count;
        let triangles = cap_triangles + 4 * layers * count;
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
        if let Some(hubs) = hubs {
            hubs.append(
                self,
                &mut mesh,
                [
                    Rim { start: 0, count },
                    Rim {
                        start: cap_top,
                        count,
                    },
                ],
            )?;
        } else {
            let points = boundary
                .points
                .iter()
                .map(|&[x, y]| Point2::new(x, y))
                .collect();
            let region = triangulate_region(points, &boundary.edges, boundary.holes)?;
            for [a, b, c] in region.triangles {
                mesh.triangle([c, b, a], 0);
                mesh.triangle([cap_top + a, cap_top + b, cap_top + c], 1);
            }
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

#[derive(Clone, Copy)]
struct Rim {
    start: usize,
    count: usize,
}

impl Rim {
    fn edge(self, i: usize) -> [usize; 2] {
        [self.start + i, self.start + (i + 1) % self.count]
    }
}

#[derive(Clone, Copy)]
struct Circle {
    radius: f64,
    count: usize,
}

impl Circle {
    fn new(radius: f64, options: Tessellation) -> Result<Self, MeshError> {
        Ok(Self {
            radius,
            count: options.arc_segments(radius, TAU)?.max(3),
        })
    }

    fn at(self, mesh: &mut DisplayMesh, z: f64) -> Rim {
        let ring = Rim {
            start: mesh.vertices.len() / 3,
            count: self.count,
        };
        for i in 0..self.count {
            let angle = TAU * i as f64 / self.count as f64;
            let (sin, cos) = angle.sin_cos();
            mesh.vertices
                .extend([self.radius * cos, self.radius * sin, z]);
        }
        ring
    }
}

struct HubEnds {
    extensions: [Option<super::super::HubExtension>; 2],
    hub: Circle,
    bore: Option<Circle>,
}

impl HubEnds {
    fn new(gear: &Herringbone, options: Tessellation) -> Result<Option<Self>, MeshError> {
        let extensions = gear.hub_extensions();
        let Some(extension) = extensions.iter().flatten().next() else {
            return Ok(None);
        };
        let bore = gear
            .profile()
            .bore_radius_mm()
            .map(|radius| Circle::new(radius, options))
            .transpose()?;
        let hub_options = Tessellation {
            deflection: bore.map_or(options.deflection, |bore| {
                options
                    .deflection
                    .min((extension.radius_mm - bore.radius) / 2.)
            }),
            ..options
        };
        Ok(Some(Self {
            extensions,
            hub: Circle::new(extension.radius_mm, hub_options)?,
            bore,
        }))
    }

    fn triangles(&self, outer: usize) -> usize {
        let bore = self.bore.map_or(0, |circle| circle.count);
        let mut triangles = 2 * bore;
        for extension in self.extensions {
            let hole = if extension.is_some() {
                self.hub.count
            } else {
                bore
            };
            triangles += outer + hole - if hole == 0 { 2 } else { 0 };
            if extension.is_some() {
                triangles += 3 * self.hub.count + bore - if bore == 0 { 2 } else { 0 };
            }
        }
        triangles
    }

    fn append(
        self,
        gear: &Herringbone,
        mesh: &mut DisplayMesh,
        outer: [Rim; 2],
    ) -> Result<(), MeshError> {
        let limits = gear.profile().axial_bounds();
        let mut bores = [None; 2];
        for (i, extension) in self.extensions.into_iter().enumerate() {
            let upwards = i == 1;
            let bore = self.bore.map(|circle| circle.at(mesh, limits[i]));
            bores[i] = bore;
            let hole = if let Some(extension) = extension {
                let opening = self.hub.at(mesh, extension.gear_z_mm);
                let end = self.hub.at(mesh, extension.end_z_mm);
                let [low, high] = if upwards {
                    [opening, end]
                } else {
                    [end, opening]
                };
                for k in 0..self.hub.count {
                    wall(mesh, low.edge(k), high.edge(k), extension.wall_face);
                }
                cap(mesh, end, bore, upwards, extension.cap_face)?;
                Some(opening)
            } else {
                bore
            };
            cap(mesh, outer[i], hole, upwards, i as u32)?;
        }
        if let [Some(low), Some(high)] = bores {
            let face = (2 + 2 * gear.profile().outline().len()) as u32;
            for k in 0..low.count {
                let [a, b] = low.edge(k);
                let [c, d] = high.edge(k);
                wall(mesh, [b, a], [d, c], face);
            }
        }
        Ok(())
    }
}

fn cap(
    mesh: &mut DisplayMesh,
    outer: Rim,
    hole: Option<Rim>,
    upwards: bool,
    face: u32,
) -> Result<(), MeshError> {
    let indices = |rim: Rim| rim.start..rim.start + rim.count;
    let boundary: Vec<_> = indices(outer)
        .chain(hole.into_iter().flat_map(|rim| indices(rim).rev()))
        .collect();
    let points = boundary
        .iter()
        .map(|&i| Point2::new(mesh.vertices[3 * i], mesh.vertices[3 * i + 1]))
        .collect();
    let mut edges: Vec<_> = (0..outer.count)
        .map(|i| [i, (i + 1) % outer.count])
        .collect();
    if let Some(hole) = hole {
        edges
            .extend((0..hole.count).map(|i| [outer.count + i, outer.count + (i + 1) % hole.count]));
    }
    for [a, b, c] in triangulate_region(points, &edges, usize::from(hole.is_some()))?.triangles {
        let triangle = if upwards { [a, b, c] } else { [c, b, a] };
        mesh.triangle(triangle.map(|i| boundary[i]), face);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inconsistent_hub_boundary_refuses_mesh_publication() {
        let (spec, twist) = super::super::tests::fixture(24);
        let mut gear = Herringbone::new_helical(spec, twist, 1e-5, 1e7).unwrap();
        // Isolate the cap validator from the public constructor's wall
        // guard. An inconsistent interior wire must propagate a typed
        // failure instead of publishing a disconnected hub cap.
        gear.profile.hub = Some(super::super::super::Hub {
            radius_mm: 3.,
            axial_start_mm: -2.,
            axial_end_mm: 6.,
        });
        assert_eq!(
            gear.mesh(Tessellation {
                deflection: 0.005,
                angle: 0.15,
                triangle_limit: 100_000,
            })
            .err(),
            Some(MeshError::UnresolvedBoundary)
        );
    }

    #[test]
    fn narrow_hub_root_clearance_limits_coarse_requested_strip_error() {
        let (mut spec, _) = super::super::tests::fixture(24);
        spec.bore_radius_mm = spec.root_radius_mm - 0.1;
        let gear = Herringbone::new_helical(spec, 0.5, 1e-5, 1e7)
            .unwrap()
            .with_hub(
                super::super::super::Hub {
                    radius_mm: spec.root_radius_mm - 0.05,
                    axial_start_mm: -2.,
                    axial_end_mm: 6.,
                },
                1e-5,
                1e7,
            )
            .unwrap();
        let options = Tessellation {
            deflection: 0.5,
            angle: 0.5,
            triangle_limit: 200_000,
        };
        let mesh = gear.mesh(options).unwrap();
        crate::manufacturing::certify(&mesh).unwrap();
        let tooth_end = 2 + 2 * gear.profile().outline().len() as u32;
        for (triangle, &face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            if !(2..tooth_end).contains(&face) {
                continue;
            }
            let points: Vec<_> = triangle
                .iter()
                .map(|&i| {
                    [
                        mesh.vertices[3 * i as usize],
                        mesh.vertices[3 * i as usize + 1],
                    ]
                })
                .collect();
            for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(3) {
                assert!(
                    crate::revolution::distance_to_segment([0.; 2], [*a, *b]) > spec.bore_radius_mm
                );
            }
        }

        let (mut thinner, _) = super::super::tests::fixture(24);
        thinner.bore_radius_mm = thinner.root_radius_mm - 0.0002;
        let thinner = Herringbone::new_helical(thinner, 0.5, 1e-5, 1e7)
            .unwrap()
            .with_hub(
                super::super::super::Hub {
                    radius_mm: thinner.root_radius_mm - 0.0001,
                    axial_start_mm: -2.,
                    axial_end_mm: 6.,
                },
                1e-5,
                1e7,
            )
            .unwrap();
        assert_eq!(thinner.mesh(options).err(), Some(MeshError::BudgetExceeded));
    }

    #[test]
    fn twisted_hub_meshes_share_every_cap_wall_and_continuous_bore_edge() {
        let options = Tessellation {
            deflection: 0.005,
            angle: 0.15,
            triangle_limit: 100_000,
        };
        for helical in [false, true] {
            for hand in [-1., 1.] {
                for bore in [0., 4.] {
                    let (mut spec, twist) = super::super::tests::fixture(24);
                    spec.bore_radius_mm = bore;
                    let construct = if helical {
                        Herringbone::new_helical
                    } else {
                        Herringbone::new
                    };
                    let plain = construct(spec, hand * twist, 1e-5, 1e7).unwrap();
                    for span in [[-2., 2.], [2., 6.], [-2., 6.], [1., 3.]] {
                        let gear = plain
                            .clone()
                            .with_hub(
                                super::super::super::Hub {
                                    radius_mm: 12.,
                                    axial_start_mm: span[0],
                                    axial_end_mm: span[1],
                                },
                                1e-5,
                                1e7,
                            )
                            .unwrap();
                        let mesh = gear.mesh(options).unwrap();
                        let proof = crate::manufacturing::certify(&mesh).unwrap();
                        assert!((proof.signed_volume_mm3 - gear.volume_mm3()).abs() < 3.);
                        let topology = gear.topology();
                        assert!(
                            mesh.face_indices
                                .iter()
                                .all(|&face| (face as usize) < topology.faces.len())
                        );
                        let faces: std::collections::BTreeSet<_> =
                            mesh.face_indices.iter().copied().collect();
                        assert_eq!(faces.len(), topology.faces.len());
                        let bounds = gear.profile().axial_bounds();
                        let low = mesh
                            .vertices
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .map(|v| v[2])
                            .fold(f64::INFINITY, f64::min);
                        let high = mesh
                            .vertices
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .map(|v| v[2])
                            .fold(f64::NEG_INFINITY, f64::max);
                        assert_eq!([low, high], bounds);
                        if span == [1., 3.] {
                            let unchanged = plain.mesh(options).unwrap();
                            assert_eq!(mesh.vertices, unchanged.vertices);
                            assert_eq!(mesh.indices, unchanged.indices);
                            assert_eq!(mesh.face_indices, unchanged.face_indices);
                        } else {
                            assert_eq!(
                                gear.mesh(Tessellation {
                                    triangle_limit: mesh.indices.len() / 3 - 1,
                                    ..options
                                })
                                .err(),
                                Some(MeshError::BudgetExceeded)
                            );
                        }
                    }
                }
            }
        }
    }

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
