//! Rounded mortises on exact rectilinear stock.

use super::*;
use crate::geometry::{
    circular_pocket_body,
    rounded_slot_body::{point, rim_edge, wall_measure, wall_support},
};
use multicad_rust_kernel::{orthogonal_slots, stadium::Stadium};

#[derive(Clone, Debug)]
pub struct RoundedSlotTool {
    pub frame: PlaneFrame,
    pub centers_uv: [[f64; 2]; 2],
    pub radius_mm: f64,
    pub displacement_mm: [f64; 3],
    pub origin: Origin,
}

const OPERATION: &str = "rounded slot";

fn slot_origin(origin: &Origin, face: &str) -> Origin {
    Origin {
        source: origin.source,
        role: format!("slot.{face}/{}/{}", origin.source, origin.role),
    }
}

fn unsupported_slot(message: &str) -> GeometryError {
    error(OPERATION, GeometryErrorKind::Unsupported, message)
}

impl Solid {
    /// Subtract disjoint closed rounded slots from a planar orthogonal stock
    /// face. The cutter must enter from outside and leave a blind floor.
    pub fn rust_drill_rounded_slots(
        &self,
        tools: &[RoundedSlotTool],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if policy != self.policy || policy.boolean_fuzzy_mm != 0. {
            return Err(unsupported_slot(
                "Rounded slots require the stock's exact geometry policy",
            ));
        }
        if tools.is_empty() {
            return self.share();
        }
        let result = self.orthogonal_stock()?;
        let (body, frame) = result
            .orthogonal_body()
            .expect("stock was converted to an orthogonal solid");
        if tools.len()
            > orthogonal_slots::slot_capacity(&body.geometry)
                .saturating_sub(body.slots.len() + body.pockets.len())
        {
            return Err(circular_pocket_body::pocket_error(
                orthogonal_slots::PocketError::BudgetExceeded,
            ));
        }
        let mut slots = body.slots.to_vec();
        let mut origins = body.slot_origins.to_vec();
        let axes = [frame.u, frame.v, frame.normal()];
        let local = |vector| axes.map(|axis| dot(vector, axis));
        for tool in tools {
            let start = |uv| {
                circular_pocket_body::tool_point(
                    tool.frame,
                    uv,
                    tool.radius_mm,
                    tool.displacement_mm,
                    policy,
                )
                .map(|point| local(subtract(point, frame.origin)))
            };
            let [a, b] = tool.centers_uv.map(start);
            let slot = orthogonal_slots::identify_slot(
                &body.geometry,
                &body.boundary,
                [a?, b?],
                local(tool.displacement_mm),
                tool.radius_mm,
                policy.linear_tolerance_mm,
                policy.minimum_feature_mm,
            )
            .map_err(circular_pocket_body::pocket_error)?;
            slots.push(slot);
            origins.push(tool.origin.clone());
        }
        result.with_orthogonal_slots(slots, origins)
    }

    pub(in crate::geometry) fn with_orthogonal_slots(
        mut self,
        slots: Vec<OrthogonalSlot>,
        origins: Vec<Origin>,
    ) -> Result<Self, GeometryError> {
        let SolidBody::Orthogonal(body, _) = &mut self.body else {
            return Err(unsupported_slot("Rounded slots require orthogonal stock"));
        };
        if !body.through_bores.is_empty() {
            return Err(unsupported_slot(
                "Combined slots and orthogonal through-bores are unsupported",
            ));
        }
        circular_pocket_body::check_origins(slots.len(), &origins)?;
        body.validate_slots(&slots, &body.pockets, self.policy)?;
        let body = Rc::make_mut(body);
        body.slots = slots.into();
        body.slot_origins = origins.into();
        Ok(self)
    }

    pub(crate) fn orthogonal_stock(&self) -> Result<Self, GeometryError> {
        if self.orthogonal_body().is_some() {
            return self.share();
        }
        let (body, frame) = match &self.body {
            SolidBody::Prism(prism, frame) => (
                SolidBody::Prism(Rc::clone(prism), PlaneFrame::default()),
                *frame,
            ),
            SolidBody::PolygonPrism(placed) => {
                let mut stock = (**placed).clone();
                stock.frame = PlaneFrame::default();
                stock.pockets = Rc::from([]);
                stock.pocket_origins = Rc::from([]);
                (SolidBody::PolygonPrism(Rc::new(stock)), placed.frame)
            }
            SolidBody::Linear(body, frame) => (
                SolidBody::Linear(Rc::clone(body), PlaneFrame::default()),
                *frame,
            ),
            _ => {
                return Err(unsupported_slot(
                    "Blind edge cuts require rectilinear stock",
                ));
            }
        };
        let mut local = self.share()?;
        local.body = body;
        let mut sources = Sources::default();
        let (feature, cells) = local
            .orthogonal_operand(false, &mut sources)?
            .ok_or_else(|| unsupported_slot("Blind edge cuts require rectilinear stock"))?;
        let geometry = orthogonal::boolean(
            &cells,
            &[],
            Operation::Union,
            self.policy.linear_tolerance_mm,
        )
        .map_err(operation_error)?
        .ok_or_else(|| unsupported_slot("Orthogonal stock has no material"))?;
        let body = Rc::new(OrthogonalBody::build(feature, geometry, Some(&sources))?);
        let result = Self {
            body: SolidBody::Orthogonal(Rc::clone(&body), frame),
            policy: self.policy,
            provenance: vec![],
            rust_origins: vec![],
        };
        let Some(stock) = self
            .polygon_prism()
            .filter(|stock| !stock.pockets.is_empty())
        else {
            return Ok(result);
        };
        let sign = if stock.reverse_sweep { -1. } else { 1. };
        let height = stock.prism.height().as_f64();
        let pockets = stock
            .pockets
            .iter()
            .map(|pocket| {
                let level = sign * pocket.mouth_z_mm(height);
                let normal = [0., 0., if pocket.end { sign } else { -sign }];
                let face = body
                    .boundary
                    .faces
                    .iter()
                    .position(|face| face.normal == normal && face.rings[0][0][2] == level)
                    .ok_or_else(|| {
                        unsupported_slot("Cannot recover the stock cap for an existing blind bore")
                    })?;
                Ok(OrthogonalPocket {
                    face,
                    center_mm: [pocket.center_mm[0], pocket.center_mm[1], level],
                    radius_mm: pocket.radius_mm,
                    depth_mm: pocket.depth_mm,
                })
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        drop(body);
        result.with_orthogonal_pockets(pockets, stock.pocket_origins.to_vec())
    }
}

impl OrthogonalBody {
    pub(super) fn validate_slots(
        &self,
        slots: &[OrthogonalSlot],
        pockets: &[OrthogonalPocket],
        policy: GeometryPolicy,
    ) -> Result<(), GeometryError> {
        orthogonal_slots::validate(
            &self.geometry,
            &self.boundary,
            slots,
            pockets,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .map_err(circular_pocket_body::pocket_error)
    }

    fn slot_face_start(&self) -> usize {
        self.faces.len() + 2 * self.pockets.len()
    }

    fn slot_edge_start(&self, slot: usize) -> u32 {
        self.edge_count + (2 * self.pockets.len() + 12 * slot) as u32
    }

    pub(super) fn append_slot_openings(&self, face: usize, loops: &mut Vec<Vec<EdgeUse>>) {
        loops.extend(
            self.slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.face == face)
                .map(|(i, _)| {
                    let start = self.slot_edge_start(i);
                    (start..start + 4).map(|edge| (edge, false)).collect()
                }),
        );
    }

    pub(super) fn slot_loops(&self, face: usize) -> Vec<Vec<EdgeUse>> {
        let offset = face - self.slot_face_start();
        let slot = offset / 5;
        if slot >= self.slots.len() {
            return vec![];
        }
        let start = self.slot_edge_start(slot);
        let side = (offset % 5) as u32;
        if side == 4 {
            return vec![(start + 4..start + 8).map(|edge| (edge, false)).collect()];
        }
        vec![vec![
            (start + side, false),
            (start + 8 + (side + 1) % 4, false),
            (start + 4 + side, true),
            (start + 8 + side, true),
        ]]
    }

    fn slot_frames(&self, slot: &OrthogonalSlot) -> (Stadium, ReferenceFrame, ReferenceFrame) {
        let face = &self.boundary.faces[slot.face];
        let profile = slot.profile(face).expect("validated rounded slot profile");
        let normal = face.normal;
        let axis = axis(normal);
        let u = std::array::from_fn(|i| f64::from(i == (axis + 1) % 3));
        let v = cross(normal, u);
        let origin_mm = std::array::from_fn(|i| if i == axis { slot.centers_mm[0][i] } else { 0. });
        let mouth = ReferenceFrame { origin_mm, u, v };
        let floor = ReferenceFrame {
            origin_mm: std::array::from_fn(|i| origin_mm[i] - slot.depth_mm * normal[i]),
            ..mouth
        };
        (profile, mouth, floor)
    }

    pub(super) fn append_slot_faces(&self, faces: &mut Vec<Face>) {
        for slot in self.slots.iter() {
            let (profile, mouth, floor) = self.slot_frames(slot);
            let area = profile.area_mm2();
            subtract_face_area(
                &mut faces[slot.face],
                point(mouth, profile.centroid()),
                area,
            );
            let first = faces.len() as u32;
            faces.extend((0..4).map(|edge| {
                let (length, center) = wall_measure(&profile, edge);
                let [a, b] = [mouth, floor].map(|frame| point(frame, center));
                Face {
                    index: first + edge as u32,
                    plane: wall_support(&profile, edge, mouth, true)
                        .map(|support| (support.origin_mm, support.normal())),
                    center_mm: std::array::from_fn(|i| a[i].midpoint(b[i])),
                    area_mm2: length * slot.depth_mm,
                }
            }));
            faces.push(Face {
                index: first + 4,
                plane: Some((floor.origin_mm, floor.normal())),
                center_mm: point(floor, profile.centroid()),
                area_mm2: area,
            });
        }
    }

    pub(super) fn append_slot_topology(&self, snapshot: &mut TopologySnapshot) {
        for (i, (slot, origin)) in self.slots.iter().zip(self.slot_origins.iter()).enumerate() {
            let (profile, mouth, floor) = self.slot_frames(slot);
            let first = self.slot_face_start() + i * 5;
            let edge_start = self.slot_edge_start(i) as usize;
            let vertex_start = self.vertex_count as usize + i * 8;
            let walls = [0, 1, 2, 3].map(|edge| {
                let wall_origin = slot_origin(origin, &format!("wall.{edge}"));
                BTreeSet::from([wall_origin, origin.clone()])
            });
            let floor_origin = slot_origin(origin, "floor");
            let floor_origins = BTreeSet::from([floor_origin.clone(), origin.clone()]);
            let cap = &mut snapshot.elements[slot.face];
            cap.boundary.extend(walls.iter().flatten().cloned());
            let cap_origins = cap.origins.clone();
            for edge in 0..4 {
                let support = wall_support(&profile, edge, mouth, true);
                let frames = support
                    .into_iter()
                    .map(|frame| AnchoredFrame {
                        origin: slot_origin(origin, &format!("wall.{edge}")),
                        frame,
                    })
                    .collect();
                snapshot.elements.push(TopologyElement {
                    key: TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: (first + edge) as u32,
                    },
                    origins: walls[edge].clone(),
                    boundary: cap_origins
                        .iter()
                        .chain(&floor_origins)
                        .chain(&walls[(edge + 3) % 4])
                        .chain(&walls[(edge + 1) % 4])
                        .cloned()
                        .collect(),
                    frames,
                    geometry: support.map(|frame| AnalyticGeometry::Plane { frame }),
                });
            }
            let mut floor_element = element(
                TopologyKind::Face,
                first + 4,
                floor_origins.clone(),
                AnalyticGeometry::Plane { frame: floor },
            );
            floor_element.frames.push(AnchoredFrame {
                origin: floor_origin,
                frame: ReferenceFrame {
                    origin_mm: point(floor, profile.centroid()),
                    ..floor
                },
            });
            floor_element.boundary = walls.iter().flatten().cloned().collect();
            snapshot.elements.push(floor_element);
            let corners = profile.corners();
            let boundary = |kind, ordinal, geometry, origins: BTreeSet<Origin>| {
                let mut item = element(kind, ordinal, origins.clone(), geometry);
                item.boundary = origins;
                item
            };
            for (side, (frame, adjacent)) in [(mouth, &cap_origins), (floor, &floor_origins)]
                .into_iter()
                .enumerate()
            {
                for (edge, corner) in corners.into_iter().enumerate() {
                    snapshot.elements.push(boundary(
                        TopologyKind::Edge,
                        edge_start + side * 4 + edge,
                        rim_edge(&profile, edge, frame),
                        adjacent.union(&walls[edge]).cloned().collect(),
                    ));
                    snapshot.elements.push(boundary(
                        TopologyKind::Vertex,
                        vertex_start + side * 4 + edge,
                        AnalyticGeometry::Point {
                            position_mm: point(frame, corner),
                        },
                        adjacent
                            .iter()
                            .chain(&walls[edge])
                            .chain(&walls[(edge + 3) % 4])
                            .cloned()
                            .collect(),
                    ));
                }
            }
            for (edge, corner) in corners.into_iter().enumerate() {
                snapshot.elements.push(boundary(
                    TopologyKind::Edge,
                    edge_start + 8 + edge,
                    AnalyticGeometry::Line {
                        a_mm: point(mouth, corner),
                        b_mm: point(floor, corner),
                    },
                    walls[edge].union(&walls[(edge + 3) % 4]).cloned().collect(),
                ));
            }
        }
    }
}
