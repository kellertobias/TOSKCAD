//! Coaxial extensions inside the tooth root, with one continuous bore.

use super::{Error, MAX_COORDINATE_MM, Point, Profile, TAU, normal_sweep, polar};
use crate::display::{DisplayMesh, MeshError, Tessellation, triangulate_region, wall};
use spade::Point2;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hub {
    pub radius_mm: f64,
    pub axial_start_mm: f64,
    pub axial_end_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HubExtension {
    pub radius_mm: f64,
    pub gear_z_mm: f64,
    pub end_z_mm: f64,
    pub wall_face: u32,
    pub cap_face: u32,
}

impl Profile {
    /// Attach a coaxial hub within the gear root and around its optional bore.
    /// A hub fully contained in the tooth band leaves the solid unchanged.
    pub fn with_hub(
        mut self,
        hub: Hub,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
    ) -> Result<Self, Error> {
        if !(0. < minimum_mm
            && minimum_mm < maximum_coordinate_mm
            && maximum_coordinate_mm <= MAX_COORDINATE_MM)
            || ![hub.axial_start_mm, hub.axial_end_mm]
                .into_iter()
                .all(|z| z.is_finite() && z.abs() <= maximum_coordinate_mm)
            || !(minimum_mm..=maximum_coordinate_mm).contains(&hub.radius_mm)
            || !(minimum_mm..=maximum_coordinate_mm)
                .contains(&(hub.axial_end_mm - hub.axial_start_mm))
        {
            return Err(Error::InvalidDimensions);
        }
        let bore = self.bore_radius_mm().unwrap_or(0.);
        let overlap = hub.axial_end_mm.min(self.height_mm) - hub.axial_start_mm.max(0.);
        let axial_lengths = [-hub.axial_start_mm, hub.axial_end_mm - self.height_mm];
        if self.root_radius_mm - hub.radius_mm < minimum_mm
            || hub.radius_mm - bore < minimum_mm
            || overlap < minimum_mm
            || axial_lengths
                .into_iter()
                .any(|length| length > 0. && length < minimum_mm)
        {
            return Err(Error::InvalidProfile);
        }
        self.hub = Some(hub);
        Ok(self)
    }

    pub fn hub(&self) -> Option<Hub> {
        self.hub
    }

    /// Full material Z range. The tooth band remains 0..height_mm().
    pub fn axial_bounds(&self) -> [f64; 2] {
        self.hub.map_or([0., self.height_mm], |hub| {
            [
                hub.axial_start_mm.min(0.),
                hub.axial_end_mm.max(self.height_mm),
            ]
        })
    }

    /// Lower and upper protrusions. Tooth-band cap, tooth-wall and bore
    /// ordinals are unchanged. Each protrusion appends its wall and end cap.
    pub fn hub_extensions(&self) -> [Option<HubExtension>; 2] {
        let Some(hub) = self.hub else {
            return [None; 2];
        };
        let mut index = self.outline.len() as u32 + 2 + u32::from(self.bore.is_some());
        let [low, high] = self.axial_bounds();
        [(0., low), (self.height_mm, high)].map(|(gear_z_mm, end_z_mm)| {
            (gear_z_mm != end_z_mm).then(|| {
                let wall_face = index;
                index += 2;
                HubExtension {
                    radius_mm: hub.radius_mm,
                    gear_z_mm,
                    end_z_mm,
                    wall_face,
                    cap_face: wall_face + 1,
                }
            })
        })
    }

    pub(super) fn bore_radius_mm(&self) -> Option<f64> {
        self.bore.and_then(normal_sweep::Edge::radius_mm)
    }

    pub(super) fn extension_length_mm(&self) -> f64 {
        let [low, high] = self.axial_bounds();
        (high - self.height_mm) - low
    }
}

#[derive(Clone, Copy)]
struct Ring {
    start: usize,
    count: usize,
}

impl Ring {
    fn indices(self) -> Range<usize> {
        self.start..self.start + self.count
    }

    fn edge(self, index: usize) -> [usize; 2] {
        [self.start + index, self.start + (index + 1) % self.count]
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
            count: options.arc_segments(radius, TAU)?,
        })
    }

    fn at(self, mesh: &mut DisplayMesh, z: f64) -> Ring {
        ring(
            mesh,
            (0..self.count).map(|i| polar(self.radius, TAU * i as f64 / self.count as f64)),
            z,
        )
    }
}

pub(super) fn mesh(
    profile: &Profile,
    extension: [Option<HubExtension>; 2],
    hub_radius: f64,
    outer_samples: &[(Point, u32)],
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    let bore = profile
        .bore_radius_mm()
        .map(|r| {
            // The outer hub circle is sampled as an inscribed polygon. Finer
            // hub sampling below keeps a narrow annular cap safely connected.
            Circle::new(r, options)
        })
        .transpose()?;
    let hub_options = Tessellation {
        deflection: bore.map_or(options.deflection, |bore| {
            options.deflection.min((hub_radius - bore.radius) / 2.)
        }),
        ..options
    };
    let hub = Circle::new(hub_radius, hub_options)?;
    let circles = extension.iter().flatten().count() * hub.count;
    let points = outer_samples.len() + circles + bore.map_or(0, |bore| bore.count);
    let triangles = 4 * points - if bore.is_none() { 4 } else { 0 };
    if triangles > options.triangle_limit {
        return Err(MeshError::BudgetExceeded);
    }
    let mut mesh = DisplayMesh {
        vertices: Vec::with_capacity(6 * points),
        indices: Vec::with_capacity(3 * triangles),
        face_indices: Vec::with_capacity(triangles),
    };
    let band_z = [0., profile.height_mm];
    let rim = band_z.map(|z| ring(&mut mesh, outer_samples.iter().map(|&(p, _)| p), z));
    for (i, &(_, face)) in outer_samples.iter().enumerate() {
        wall(&mut mesh, rim[0].edge(i), rim[1].edge(i), face);
    }
    let mut bore_ends = [None; 2];
    for (i, extension) in extension.into_iter().enumerate() {
        let upwards = i == 1;
        let z = extension.map_or(band_z[i], |ext| ext.end_z_mm);
        let bore_end = bore.map(|circle| circle.at(&mut mesh, z));
        bore_ends[i] = bore_end;
        let hole = if let Some(ext) = extension {
            let opening = hub.at(&mut mesh, ext.gear_z_mm);
            let end = hub.at(&mut mesh, ext.end_z_mm);
            let [low, high] = if upwards {
                [opening, end]
            } else {
                [end, opening]
            };
            for k in 0..hub.count {
                wall(&mut mesh, low.edge(k), high.edge(k), ext.wall_face);
            }
            cap(&mut mesh, end, bore_end, upwards, ext.cap_face)?;
            Some(opening)
        } else {
            bore_end
        };
        cap(&mut mesh, rim[i], hole, upwards, i as u32)?;
    }
    if let [Some(low), Some(high)] = bore_ends {
        let face = profile.outline.len() as u32 + 2;
        for i in 0..low.count {
            let [a, b] = low.edge(i);
            let [c, d] = high.edge(i);
            wall(&mut mesh, [b, a], [d, c], face);
        }
    }
    Ok(mesh)
}

fn ring(mesh: &mut DisplayMesh, points: impl ExactSizeIterator<Item = Point>, z: f64) -> Ring {
    let rim = Ring {
        start: mesh.vertices.len() / 3,
        count: points.len(),
    };
    mesh.vertices.extend(points.flat_map(|[x, y]| [x, y, z]));
    rim
}

fn cap(
    mesh: &mut DisplayMesh,
    outer: Ring,
    hole: Option<Ring>,
    upwards: bool,
    face: u32,
) -> Result<(), MeshError> {
    let boundary: Vec<_> = outer
        .indices()
        .chain(hole.into_iter().flat_map(|rim| rim.indices().rev()))
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
