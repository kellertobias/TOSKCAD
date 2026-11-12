//! Owner-thread loft solids with exact walls and persistent authored origins.
use super::*;
use crate::rust_prism_topology::{Builder, place_frame, polygon_planar_loops, position};
use multicad_core::references::{AnalyticGeometry, AnchoredFrame};
use multicad_rust_kernel::planar_boolean::{
    self, LoftDrawingEdge, LoftProjection, LoftSectionCurve, NonplanarLoft,
};

type Point = [f64; 3];

struct LoftFace {
    plane: Option<ReferenceFrame>,
    anchors: Vec<AnchoredFrame>,
    origins: Vec<Origin>,
    center: Point,
}

pub(crate) struct NonplanarLoftBody {
    pub(super) geometry: NonplanarLoft,
    pub(super) feature: EntityId,
    faces: Vec<LoftFace>,
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn norm([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

fn normalized(vector: Point) -> Point {
    let magnitude = norm(vector);
    vector.map(|value| value / magnitude)
}

fn anchored(
    origins: &[Origin],
    frames: impl IntoIterator<Item = ReferenceFrame>,
) -> Vec<AnchoredFrame> {
    origins
        .iter()
        .cloned()
        .zip(frames)
        .map(|(origin, frame)| AnchoredFrame { origin, frame })
        .collect()
}

impl Solid {
    pub(super) fn from_nonplanar_loft(
        geometry: NonplanarLoft,
        feature: EntityId,
        placement: PlaneFrame,
        sections: [&[Origin]; 2],
        frames: [ReferenceFrame; 2],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let [start, end] = geometry.rings();
        for (cap, ring) in [start, end].into_iter().enumerate() {
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                policy.point(position(placement, a))?;
                policy.length(norm(subtract(b, a)))?;
            }
            if sections[cap].len() != ring.len() {
                return Err(input("Loft section does not have a source origin per edge"));
            }
        }
        for (&a, &b) in start.iter().zip(end) {
            policy.length(norm(subtract(b, a)))?;
        }

        let mut faces = Vec::with_capacity(start.len() + 2);
        for (cap, mut frame) in frames.into_iter().enumerate() {
            let normal = if cap == 0 {
                [0., 0., -1.]
            } else {
                [0., 0., 1.]
            };
            frame.v = cross(normal, frame.u);
            let role = if cap == 0 { "cap.start" } else { "cap.end" };
            let origins = vec![Origin {
                source: feature,
                role: role.into(),
            }];
            let ring = &geometry.rings()[cap];
            let center = std::array::from_fn(|axis| {
                let anchor = ring[0][axis];
                anchor + ring.iter().map(|p| p[axis] - anchor).sum::<f64>() / ring.len() as f64
            });
            faces.push(LoftFace {
                plane: Some(frame),
                anchors: anchored(&origins, [frame]),
                origins,
                center,
            });
        }
        for (i, wall) in geometry.walls().enumerate() {
            let first = &sections[0][i];
            let last = &sections[1][i];
            let origins = vec![
                Origin {
                    source: feature,
                    role: format!("side/{}/{}", first.source, first.role),
                },
                first.clone(),
                last.clone(),
            ];
            let [a, b, c, d] = [
                wall.point(0., 0.),
                wall.point(1., 0.),
                wall.point(0., 1.),
                wall.point(1., 1.),
            ];
            let tangent = subtract(b, a);
            let upper = subtract(d, c);
            // Some walls in a twisted loft are still planar. Recognize only
            // roundoff-level parallelism, retaining the original edge anchors.
            let epsilon = (64.
                * f64::EPSILON
                * [a, b, c, d]
                    .into_iter()
                    .flatten()
                    .map(f64::abs)
                    .fold(1., f64::max))
            .min(policy.linear_tolerance_mm);
            let planar = dot(tangent, upper) > 0.
                && norm(cross(tangent, upper)) <= epsilon * norm(tangent).max(norm(upper));
            let (plane, anchors) = if planar {
                let normal = normalized(cross(tangent, subtract(c, a)))
                    .map(|value| if geometry.clockwise() { -value } else { value });
                let frame = |origin, edge: Point| {
                    let u = normalized(std::array::from_fn(|axis| {
                        edge[axis] - normal[axis] * dot(normal, edge)
                    }));
                    ReferenceFrame {
                        origin_mm: origin,
                        u,
                        v: cross(normal, u),
                    }
                };
                let lower = frame(a, tangent);
                (
                    Some(lower),
                    anchored(&origins, [lower, lower, frame(c, upper)]),
                )
            } else {
                (None, vec![])
            };
            faces.push(LoftFace {
                plane,
                anchors,
                origins,
                center: wall.point(0.5, 0.5),
            });
        }
        let body = NonplanarLoftBody {
            geometry,
            feature,
            faces,
        };
        body.topology(placement, feature, 0)
            .validate()
            .map_err(|message| error("loft topology", GeometryErrorKind::InvalidResult, message))?;
        Ok(Self {
            body: SolidBody::NonplanarLoft(Rc::new(body), placement),
            policy,
            provenance: vec![],
            rust_origins: vec![],
            retained_placement: None,
        })
    }

    pub(super) fn from_imported_nonplanar_loft(
        geometry: NonplanarLoft,
        feature: EntityId,
        placement: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let section_origins = std::array::from_fn::<_, 2, _>(|cap| {
            (0..geometry.rings()[cap].len())
                .map(|edge| Origin {
                    source: feature,
                    role: format!("import.loft.section.{cap}.edge.{edge}"),
                })
                .collect::<Vec<_>>()
        });
        let frames = geometry.rings().each_ref().map(|ring| ReferenceFrame {
            origin_mm: ring[0],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        });
        Self::from_nonplanar_loft(
            geometry,
            feature,
            placement,
            section_origins.each_ref().map(Vec::as_slice),
            frames,
            policy,
        )
    }

    pub(crate) fn nonplanar_loft_body(&self) -> Option<(&NonplanarLoftBody, PlaneFrame)> {
        match &self.body {
            SolidBody::NonplanarLoft(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl NonplanarLoftBody {
    pub(crate) fn loft_geometry(&self) -> &NonplanarLoft {
        &self.geometry
    }

    pub(crate) fn drawing_edges(
        &self,
        options: LoftProjection,
    ) -> Result<Vec<LoftDrawingEdge>, planar_boolean::Error> {
        self.geometry.drawing_edges(options)
    }

    pub(crate) fn drawing_depth_span(&self, axis: Point) -> [f64; 2] {
        self.geometry
            .rings()
            .iter()
            .flatten()
            .map(|&point| dot(axis, point))
            .fold([f64::INFINITY, f64::NEG_INFINITY], |[min, max], z| {
                [min.min(z), max.max(z)]
            })
    }

    pub(crate) fn drawing_height(&self) -> f64 {
        self.geometry.rings()[1][0][2]
    }

    pub(crate) fn section(
        &self,
        placement: PlaneFrame,
        origin: Point,
        normal: Point,
        tolerance: f64,
    ) -> Result<Vec<LoftSectionCurve>, planar_boolean::Error> {
        let axes = [placement.u, placement.v, placement.normal()];
        let local_normal = axes.map(|axis| dot(axis, normal));
        let length = norm(local_normal);
        let local_normal = local_normal.map(|value| value / length);
        let depth = dot(normal, subtract(origin, placement.origin)) / length;
        self.geometry.section(
            local_normal.map(|value| value * depth),
            local_normal,
            tolerance,
        )
    }

    pub(super) fn bounds(&self, placement: PlaneFrame) -> Bounds {
        let mut bounds = Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        for &point in self.geometry.rings().iter().flatten() {
            for (axis, coordinate) in position(placement, point).into_iter().enumerate() {
                bounds.min[axis] = bounds.min[axis].min(coordinate);
                bounds.max[axis] = bounds.max[axis].max(coordinate);
            }
        }
        bounds
    }

    pub(super) fn faces(&self, placement: PlaneFrame) -> Result<Vec<Face>, GeometryError> {
        let areas = self
            .geometry
            .cap_areas_mm2()
            .into_iter()
            .map(Ok)
            .chain(self.geometry.walls().map(|wall| wall.area_mm2()));
        self.faces
            .iter()
            .zip(areas)
            .enumerate()
            .map(|(index, (face, area))| {
                let area_mm2 = area.map_err(|cause| {
                    error(
                        "loft area",
                        GeometryErrorKind::Limit,
                        format!("Could not bound loft area: {cause:?}"),
                    )
                })?;
                let plane = face.plane.map(|p| place_frame(placement, p));
                Ok(Face {
                    index: index as u32,
                    plane: plane.map(|p| (p.origin_mm, p.normal())),
                    center_mm: position(placement, face.center),
                    area_mm2,
                })
            })
            .collect()
    }

    pub(crate) fn topology(
        &self,
        placement: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let count = self.geometry.rings()[0].len();
        let mut builder =
            Builder::new(self.faces.iter().map(|face| face.origins.clone()).collect());
        for (cap, face) in self.faces[..2].iter().enumerate() {
            builder.face(
                cap,
                face.plane.map(|frame| place_frame(placement, frame)),
                None,
                2..count + 2,
            );
        }
        for i in 0..count {
            let previous = (i + count - 1) % count;
            let next = (i + 1) % count;
            let plane = self.faces[i + 2]
                .plane
                .map(|frame| place_frame(placement, frame));
            builder.face(i + 2, plane, None, [0, 1, previous + 2, next + 2]);
        }
        let rings = self.geometry.rings();
        let point = |point| position(placement, point);
        for (cap, ring) in rings.iter().enumerate() {
            for i in 0..count {
                let [a_mm, b_mm] = [ring[i], ring[(i + 1) % count]].map(point);
                builder.non_face(AnalyticGeometry::Line { a_mm, b_mm }, &[cap, i + 2]);
            }
        }
        for (i, (&start, &end)) in rings[0].iter().zip(&rings[1]).enumerate() {
            let previous = (i + count - 1) % count;
            let [a_mm, b_mm] = [start, end].map(point);
            builder.non_face(
                AnalyticGeometry::Line { a_mm, b_mm },
                &[previous + 2, i + 2],
            );
        }
        for (cap, ring) in rings.iter().enumerate() {
            for (i, &vertex) in ring.iter().enumerate() {
                let previous = (i + count - 1) % count;
                builder.non_face(
                    AnalyticGeometry::Point {
                        position_mm: point(vertex),
                    },
                    &[cap, previous + 2, i + 2],
                );
            }
        }
        let mut elements = builder.into_elements();
        for (element, face) in elements.iter_mut().zip(&self.faces) {
            element.frames = face
                .anchors
                .iter()
                .map(|anchor| AnchoredFrame {
                    origin: anchor.origin.clone(),
                    frame: place_frame(placement, anchor.frame),
                })
                .collect();
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        if !self
            .faces
            .get(face as usize)
            .is_some_and(|face| face.plane.is_some())
        {
            return vec![];
        }
        polygon_planar_loops(std::slice::from_ref(&self.geometry.rings()[0]), 0, face)
    }
}
