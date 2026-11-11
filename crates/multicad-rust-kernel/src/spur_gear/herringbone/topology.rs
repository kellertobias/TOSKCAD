use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Band {
    Lower,
    Upper,
}
impl Band {
    pub fn axial_interval_mm(self, height: f64) -> [f64; 2] {
        match self {
            Self::Lower => [0., height * 0.5],
            Self::Upper => [height * 0.5, height],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ring {
    Start,
    Apex,
    End,
}
impl Ring {
    fn height_mm(self, height: f64) -> f64 {
        match self {
            Self::Start => 0.,
            Self::Apex => height * 0.5,
            Self::End => height,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Face {
    Cap { end: bool },
    Tooth { band: Band, edge: ToothEdge },
    Bore,
    HubWall { extension: HubExtension },
    HubCap { extension: HubExtension },
}

impl Face {
    pub fn role(self) -> String {
        match self {
            Self::Cap { end: false } => "gear/end.start".into(),
            Self::Cap { end: true } => "gear/end.end".into(),
            Self::Bore => "gear/bore".into(),
            Self::HubWall { .. } => "gear/hub.side".into(),
            Self::HubCap { extension } => if extension.end_z_mm < extension.gear_z_mm {
                "gear/hub.end.start"
            } else {
                "gear/hub.end.end"
            }
            .into(),
            Self::Tooth { band, edge } => {
                let band = match band {
                    Band::Lower => "lower",
                    Band::Upper => "upper",
                };
                let role = match edge.role {
                    super::super::ToothRole::LeftFillet => "fillet.left",
                    super::super::ToothRole::LeftRadial => "continuation.left",
                    super::super::ToothRole::LeftFlank => "involute.left",
                    super::super::ToothRole::Tip => "tip",
                    super::super::ToothRole::RightFlank => "involute.right",
                    super::super::ToothRole::RightRadial => "continuation.right",
                    super::super::ToothRole::RightFillet => "fillet.right",
                    super::super::ToothRole::Root => "root",
                };
                format!("gear/{band}/tooth/{}/{role}", edge.tooth)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Rim { ring: Ring, edge: Edge },
    Rail { band: Band, point: Point },
    BoreRim { end: bool, radius_mm: f64 },
    BoreSeam { radius_mm: f64 },
    HubRim { radius_mm: f64, z_mm: f64 },
    HubSeam { radius_mm: f64, span_mm: [f64; 2] },
}
impl Curve {
    pub fn point(self, gear: &Herringbone, fraction: f64) -> [f64; 3] {
        let height = gear.profile.height_mm();
        match self {
            Self::Rim { ring, edge } => {
                gear.section_point(edge.point(fraction), ring.height_mm(height))
            }
            Self::Rail { band, point } => {
                let [start, end] = band.axial_interval_mm(height);
                gear.section_point(point, start + (end - start) * fraction)
            }
            Self::BoreRim { end, radius_mm } => {
                let [x, y] = rotate([radius_mm, 0.], -TAU * fraction);
                [x, y, gear.axial_bounds()[usize::from(end)]]
            }
            Self::BoreSeam { radius_mm } => {
                let [lo, hi] = gear.axial_bounds();
                [radius_mm, 0., (hi - lo).mul_add(fraction, lo)]
            }
            Self::HubRim { radius_mm, z_mm } => {
                let [x, y] = rotate([radius_mm, 0.], TAU * fraction);
                [x, y, z_mm]
            }
            Self::HubSeam {
                radius_mm,
                span_mm: [lo, hi],
            } => [radius_mm, 0., (hi - lo).mul_add(fraction, lo)],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopologyEdge {
    pub vertices: [u32; 2],
    /// Face using the directed edge, then face using its reversal. A
    /// periodic cylindrical seam is used twice by the bore face.
    pub faces: [u32; 2],
    pub curve: Curve,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Topology {
    pub vertices_mm: Vec<[f64; 3]>,
    pub edges: Vec<TopologyEdge>,
    pub faces: Vec<Face>,
}

impl Herringbone {
    pub fn topology(&self) -> Topology {
        let outline = self.profile.outline();
        let count = outline.len() as u32;
        let extensions = self.hub_extensions();
        let mut result = Topology {
            vertices_mm: Vec::with_capacity(3 * outline.len() + 2),
            edges: Vec::with_capacity(5 * outline.len() + 3),
            faces: vec![Face::Cap { end: false }, Face::Cap { end: true }],
        };
        let wall = |band: u32, index: u32| 2 + band * count + index;
        for (station, ring) in [Ring::Start, Ring::Apex, Ring::End].into_iter().enumerate() {
            let base = station as u32 * count;
            for (index, item) in (0..count).zip(outline) {
                let curve = Curve::Rim {
                    ring,
                    edge: item.edge,
                };
                let faces = match ring {
                    Ring::Start => [wall(0, index), 0],
                    Ring::Apex => [wall(1, index), wall(0, index)],
                    Ring::End => [1, wall(1, index)],
                };
                result.vertices_mm.push(curve.point(self, 0.));
                result.edges.push(TopologyEdge {
                    vertices: [base + index, base + (index + 1) % count],
                    faces,
                    curve,
                });
            }
        }
        for (layer, band) in [Band::Lower, Band::Upper].into_iter().enumerate() {
            let layer = layer as u32;
            for (index, item) in (0..count).zip(outline) {
                result.faces.push(Face::Tooth { band, edge: *item });
                result.edges.push(TopologyEdge {
                    vertices: [layer * count + index, (layer + 1) * count + index],
                    faces: [wall(layer, (index + count - 1) % count), wall(layer, index)],
                    curve: Curve::Rail {
                        band,
                        point: item.edge.point(0.),
                    },
                });
            }
        }
        if let Some(radius_mm) = self.profile.bore_radius_mm() {
            let face = result.faces.len() as u32;
            let start = result.vertices_mm.len() as u32;
            result.faces.push(Face::Bore);
            let [lo, hi] = self.axial_bounds();
            result
                .vertices_mm
                .extend([[radius_mm, 0., lo], [radius_mm, 0., hi]]);
            for (index, end) in [false, true].into_iter().enumerate() {
                let vertex = start + index as u32;
                let cap = extensions[index].map_or(index as u32, |extension| extension.cap_face);
                result.edges.push(TopologyEdge {
                    vertices: [vertex, vertex],
                    faces: if end { [cap, face] } else { [face, cap] },
                    curve: Curve::BoreRim { end, radius_mm },
                });
            }
            result.edges.push(TopologyEdge {
                vertices: [start, start + 1],
                faces: [face; 2],
                curve: Curve::BoreSeam { radius_mm },
            });
        }
        for extension in extensions.into_iter().flatten() {
            result
                .faces
                .extend([Face::HubWall { extension }, Face::HubCap { extension }]);
            let upper = extension.end_z_mm > extension.gear_z_mm;
            let low = extension.gear_z_mm.min(extension.end_z_mm);
            let high = extension.gear_z_mm.max(extension.end_z_mm);
            let start = result.vertices_mm.len() as u32;
            result.vertices_mm.extend([
                [extension.radius_mm, 0., low],
                [extension.radius_mm, 0., high],
            ]);
            let cap = u32::from(upper);
            for (index, z_mm) in [low, high].into_iter().enumerate() {
                let vertex = start + index as u32;
                let adjacent = if z_mm == extension.gear_z_mm {
                    cap
                } else {
                    extension.cap_face
                };
                result.edges.push(TopologyEdge {
                    vertices: [vertex; 2],
                    faces: if index == 0 {
                        [extension.wall_face, adjacent]
                    } else {
                        [adjacent, extension.wall_face]
                    },
                    curve: Curve::HubRim {
                        radius_mm: extension.radius_mm,
                        z_mm,
                    },
                });
            }
            result.edges.push(TopologyEdge {
                vertices: [start, start + 1],
                faces: [extension.wall_face; 2],
                curve: Curve::HubSeam {
                    radius_mm: extension.radius_mm,
                    span_mm: [low, high],
                },
            });
        }
        result
    }
}
