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
}

impl Face {
    pub fn role(self) -> String {
        match self {
            Self::Cap { end: false } => "gear/end.start".into(),
            Self::Cap { end: true } => "gear/end.end".into(),
            Self::Bore => "gear/bore".into(),
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
                [x, y, if end { height } else { 0. }]
            }
            Self::BoreSeam { radius_mm } => [radius_mm, 0., fraction * height],
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
        let height = self.profile.height_mm();
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
            result
                .vertices_mm
                .extend([[radius_mm, 0., 0.], [radius_mm, 0., height]]);
            for (index, end) in [false, true].into_iter().enumerate() {
                let vertex = start + index as u32;
                result.edges.push(TopologyEdge {
                    vertices: [vertex, vertex],
                    faces: if end { [1, face] } else { [face, 0] },
                    curve: Curve::BoreRim { end, radius_mm },
                });
            }
            result.edges.push(TopologyEdge {
                vertices: [start, start + 1],
                faces: [face; 2],
                curve: Curve::BoreSeam { radius_mm },
            });
        }
        result
    }
}
