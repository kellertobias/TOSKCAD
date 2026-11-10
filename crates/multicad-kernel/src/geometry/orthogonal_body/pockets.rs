//! Blind circular cuts preserve the immutable rectilinear stock boundary.

use super::*;
use crate::geometry::circular_pocket_body;
use multicad_rust_kernel::{orthogonal_pockets, step};
use std::f64::consts::TAU;

impl Solid {
    pub(in crate::geometry) fn with_orthogonal_pockets(
        mut self,
        pockets: Vec<OrthogonalPocket>,
        origins: Vec<Origin>,
    ) -> Result<Self, GeometryError> {
        let SolidBody::Orthogonal(body, _) = &mut self.body else {
            return Err(invalid("Circular pockets require orthogonal stock"));
        };
        circular_pocket_body::check_origins(pockets.len(), &origins)?;
        orthogonal_pockets::validate(
            &body.geometry,
            &body.boundary,
            &pockets,
            self.policy.linear_tolerance_mm,
            self.policy.minimum_feature_mm,
        )
        .map_err(circular_pocket_body::pocket_error)?;
        if !body.slots.is_empty() {
            body.validate_slots(&body.slots, &pockets, self.policy)?;
        }
        let body = Rc::make_mut(body);
        body.pockets = pockets.into();
        body.pocket_origins = origins.into();
        Ok(self)
    }
}

impl OrthogonalBody {
    pub(in crate::geometry) fn drill(
        &self,
        frame: PlaneFrame,
        tools: &[CircularPocketTool],
        policy: GeometryPolicy,
    ) -> Result<(Vec<OrthogonalPocket>, Vec<Origin>), GeometryError> {
        if tools.len() > orthogonal_pockets::pocket_capacity(&self.geometry) - self.pockets.len() {
            return Err(circular_pocket_body::pocket_error(
                orthogonal_pockets::PocketError::BudgetExceeded,
            ));
        }
        let mut pockets = self.pockets.to_vec();
        let mut origins = self.pocket_origins.to_vec();
        let axes = [frame.u, frame.v, frame.normal()];
        let local = |vector| axes.map(|axis| dot(vector, axis));
        for tool in tools {
            let start = circular_pocket_body::tool_start(tool, policy)?;
            let start = local(subtract(start, frame.origin));
            let travel = local(tool.displacement_mm);
            let end = std::array::from_fn(|i| start[i] + travel[i]);
            let pocket = orthogonal_pockets::identify_pocket(
                &self.geometry,
                &self.boundary,
                start,
                end,
                tool.radius_mm,
                policy.linear_tolerance_mm,
                policy.minimum_feature_mm,
            )
            .map_err(circular_pocket_body::pocket_error)?;
            pockets.push(pocket);
            origins.push(tool.origin.clone());
        }
        Ok((pockets, origins))
    }

    pub(in crate::geometry) fn volume_mm3(&self) -> f64 {
        orthogonal_pockets::volume_mm3(&self.geometry, &self.pockets)
            - multicad_rust_kernel::orthogonal_slots::removed_volume_mm3(&self.slots)
    }

    pub(in crate::geometry) fn supports_step(&self) -> bool {
        if self.slots.is_empty() {
            step::supports_pocketed_orthogonal(&self.boundary, &self.pockets)
        } else {
            step::supports_slotted_orthogonal(&self.boundary, &self.pockets, &self.slots)
        }
    }

    pub(in crate::geometry) fn step(&self, frame: PlaneFrame) -> Result<String, step::StepError> {
        let placed = |point| transform_point(frame, point);
        if !self.slots.is_empty() {
            step::write_slotted_orthogonal(
                &self.geometry,
                &self.boundary,
                &self.pockets,
                &self.slots,
                &placed,
            )
        } else if self.pockets.is_empty() {
            step::write_orthogonal(&self.boundary, placed)
        } else {
            step::write_pocketed_orthogonal(&self.geometry, &self.boundary, &self.pockets, &placed)
        }
    }

    fn pocket_frames(&self, pocket: &OrthogonalPocket) -> [ReferenceFrame; 2] {
        let normal = self.boundary.faces[pocket.face].normal;
        let u = std::array::from_fn(|i| f64::from(i == (axis(normal) + 1) % 3));
        let v = cross(normal, u);
        let floor = pocket.floor_center_mm(normal);
        [pocket.center_mm, floor].map(|origin_mm| ReferenceFrame { origin_mm, u, v })
    }

    pub(super) fn append_pocket_faces(&self, faces: &mut Vec<Face>) {
        for pocket in self.pockets.iter() {
            let [mouth, floor] = self.pocket_frames(pocket);
            let area = pocket.area_mm2();
            subtract_face_area(&mut faces[pocket.face], mouth.origin_mm, area);
            let wall = faces.len() as u32;
            faces.extend([
                Face {
                    index: wall,
                    plane: None,
                    center_mm: std::array::from_fn(|i| {
                        mouth.origin_mm[i].midpoint(floor.origin_mm[i])
                    }),
                    area_mm2: TAU * pocket.radius_mm * pocket.depth_mm,
                },
                Face {
                    index: wall + 1,
                    plane: Some((floor.origin_mm, floor.normal())),
                    center_mm: floor.origin_mm,
                    area_mm2: area,
                },
            ]);
        }
    }

    pub(super) fn append_pocket_topology(&self, snapshot: &mut TopologySnapshot) {
        for (i, (pocket, origin)) in self
            .pockets
            .iter()
            .zip(self.pocket_origins.iter())
            .enumerate()
        {
            let [mouth, floor] = self.pocket_frames(pocket);
            let [wall_origin, floor_origin] =
                ["wall", "floor"].map(|role| circular_pocket_body::face_origin(origin, role));
            let wall_origins = BTreeSet::from([wall_origin, origin.clone()]);
            let floor_origins = BTreeSet::from([floor_origin.clone(), origin.clone()]);
            let wall = self.faces.len() + 2 * i;
            let cap = &mut snapshot.elements[pocket.face];
            cap.boundary.extend(wall_origins.iter().cloned());
            let cap_origins = cap.origins.clone();
            snapshot.elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: wall as u32,
                },
                origins: wall_origins.clone(),
                boundary: cap_origins.union(&floor_origins).cloned().collect(),
                frames: vec![],
                geometry: None,
            });
            let mut floor_element = element(
                TopologyKind::Face,
                wall + 1,
                floor_origins.clone(),
                AnalyticGeometry::Plane { frame: floor },
            );
            floor_element.frames.push(AnchoredFrame {
                origin: floor_origin,
                frame: floor,
            });
            floor_element.boundary = wall_origins.clone();
            snapshot.elements.push(floor_element);
            for (rim, (frame, adjacent)) in [(mouth, cap_origins), (floor, floor_origins)]
                .into_iter()
                .enumerate()
            {
                let origins: BTreeSet<_> = adjacent.union(&wall_origins).cloned().collect();
                let mut rim = element(
                    TopologyKind::Edge,
                    self.edge_count as usize + 2 * i + rim,
                    origins.clone(),
                    AnalyticGeometry::CircularArc {
                        frame,
                        radius_mm: pocket.radius_mm,
                        start_rad: 0.,
                        sweep_rad: TAU,
                    },
                );
                rim.boundary = origins;
                snapshot.elements.push(rim);
            }
        }
    }
}
