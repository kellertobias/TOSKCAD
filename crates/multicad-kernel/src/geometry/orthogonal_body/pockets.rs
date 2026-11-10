//! Circular cuts preserve the immutable rectilinear stock boundary.

use super::*;
use crate::geometry::circular_pocket_body;
use multicad_rust_kernel::{orthogonal_pockets, step};
use std::f64::consts::TAU;

impl Solid {
    pub(in crate::geometry) fn with_orthogonal_pockets(
        self,
        pockets: Vec<OrthogonalPocket>,
        origins: Vec<Origin>,
    ) -> Result<Self, GeometryError> {
        self.with_orthogonal_round_cuts(pockets, origins, Vec::new(), Vec::new())
    }

    pub(in crate::geometry) fn with_orthogonal_round_cuts(
        mut self,
        pockets: Vec<OrthogonalPocket>,
        pocket_origins: Vec<Origin>,
        through: Vec<OrthogonalThroughBore>,
        through_origins: Vec<Origin>,
    ) -> Result<Self, GeometryError> {
        let SolidBody::Orthogonal(body, _) = &mut self.body else {
            return Err(invalid("Circular pockets require orthogonal stock"));
        };
        circular_pocket_body::check_origins(pockets.len(), &pocket_origins)?;
        circular_pocket_body::check_origins(through.len(), &through_origins)?;
        if pocket_origins
            .iter()
            .any(|origin| through_origins.contains(origin))
        {
            return Err(invalid("Circular cuts require distinct origins"));
        }
        orthogonal_pockets::validate_with_through_bores(
            &body.geometry,
            &body.boundary,
            &pockets,
            &through,
            self.policy.linear_tolerance_mm,
            self.policy.minimum_feature_mm,
        )
        .map_err(circular_pocket_body::pocket_error)?;
        if !body.slots.is_empty() {
            if !through.is_empty() {
                return Err(unsupported(
                    "Combined slots and orthogonal through-bores are unsupported",
                ));
            }
            body.validate_slots(&body.slots, &pockets, self.policy)?;
        }
        let body = Rc::make_mut(body);
        body.pockets = pockets.into();
        body.pocket_origins = pocket_origins.into();
        body.through_bores = through.into();
        body.through_origins = through_origins.into();
        Ok(self)
    }

    pub(in crate::geometry) fn subtract_orthogonal_cylinder(
        &self,
        tool: &Solid,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        let (cylinder, tool_frame) = tool.axial_cylinder_frame()?;
        if self.orthogonal_body().is_none()
            && self.rust_prism().is_none()
            && self.polygon_prism().is_none()
            && self.linear_body().is_none()
        {
            return None;
        }
        Some(self.orthogonal_stock().and_then(|solid| {
            let (stock, frame) = solid.orthogonal_body().expect("orthogonal stock");
            let axes = [frame.u, frame.v, frame.normal()];
            let local = |point| axes.map(|axis| dot(subtract(point, frame.origin), axis));
            let world_end = transform_point(tool_frame, [0., 0., cylinder.height().as_f64()]);
            let start = local(tool_frame.origin);
            let end = local(world_end);
            let radius = cylinder.circle().radius().as_f64();
            let origin = tool.cylinder_sketch().map_or_else(
                || Origin {
                    source: EntityId::from_uuid_bits(cylinder.feature()),
                    role: "cylinder.wall".into(),
                },
                |sketch| sketch.origin.clone(),
            );
            let mut pockets = stock.pockets.to_vec();
            let mut pocket_origins = stock.pocket_origins.to_vec();
            let mut through_origins = stock.through_origins.to_vec();
            let mut through_bores = stock.through_bores.to_vec();
            if let Ok(bore) = stock.identify_through(start, end, radius, policy) {
                through_bores.push(bore);
                through_origins.push(origin);
            } else {
                let pocket = stock
                    .identify_blind(start, end, radius, policy)
                    .or_else(|_| stock.identify_blind(end, start, radius, policy))
                    .map_err(circular_pocket_body::pocket_error)?;
                pockets.push(pocket);
                pocket_origins.push(origin);
            }
            solid.with_orthogonal_round_cuts(
                pockets,
                pocket_origins,
                through_bores,
                through_origins,
            )
        }))
    }

    /// Cut an exact through bore and certify the surrounding material along
    /// its complete swept axis. Returned mouth centers are in the solid's
    /// current placement, in tool direction order.
    pub fn rust_connection_through_bore(
        &self,
        tool: &CircularPocketTool,
        minimum_wall_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<(Self, [Point; 2]), GeometryError> {
        policy.validate()?;
        if policy != self.policy || policy.boolean_fuzzy_mm != 0. {
            return Err(invalid(
                "A receiving bore requires the stock's exact geometry policy",
            ));
        }
        policy.length(minimum_wall_mm)?;
        let protected_radius_mm = tool.radius_mm + minimum_wall_mm;
        policy.length(protected_radius_mm)?;
        let start = circular_pocket_body::tool_start(tool, policy)?;
        let finish = std::array::from_fn(|i| start[i] + tool.displacement_mm[i]);
        let stock = self.orthogonal_stock()?;
        let (body, frame) = stock.orthogonal_body().expect("orthogonal stock");
        if body.geometry.solid_count() != 1 || body.has_curved_cuts() {
            return Err(unsupported(
                "A receiving bore requires one connected orthogonal stock solid without existing curved cuts",
            ));
        }
        let axes = [frame.u, frame.v, frame.normal()];
        let local = |point| axes.map(|axis| dot(subtract(point, frame.origin), axis));
        let witness = body
            .identify_through(local(start), local(finish), protected_radius_mm, policy)
            .map_err(circular_pocket_body::pocket_error)?;
        let mouths = witness
            .centers_mm
            .map(|point| transform_point(frame, point));
        let bore = OrthogonalThroughBore {
            radius_mm: tool.radius_mm,
            ..witness
        };
        let cut = stock.with_orthogonal_round_cuts(
            Vec::new(),
            Vec::new(),
            vec![bore],
            vec![tool.origin.clone()],
        )?;
        Ok((cut, mouths))
    }

    pub(in crate::geometry) fn drill_orthogonal_stock(
        &self,
        tools: &[CircularPocketTool],
        through_modes: Option<&[bool]>,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let (stock, frame) = self.orthogonal_body().expect("orthogonal stock");
        if tools.len()
            > orthogonal_pockets::pocket_capacity(&stock.geometry)
                .saturating_sub(stock.pockets.len() + stock.through_bores.len())
        {
            return Err(circular_pocket_body::pocket_error(
                orthogonal_pockets::PocketError::BudgetExceeded,
            ));
        }
        let mut pockets = stock.pockets.to_vec();
        let mut pocket_origins = stock.pocket_origins.to_vec();
        let mut through = stock.through_bores.to_vec();
        let mut through_origins = stock.through_origins.to_vec();
        let axes = [frame.u, frame.v, frame.normal()];
        let local = |vector| axes.map(|axis| dot(vector, axis));
        for (index, tool) in tools.iter().enumerate() {
            let start = circular_pocket_body::tool_start(tool, policy)?;
            let start = local(subtract(start, frame.origin));
            let travel = local(tool.displacement_mm);
            let end = std::array::from_fn(|i| start[i] + travel[i]);
            if through_modes.is_some_and(|modes| modes[index]) {
                through.push(
                    stock
                        .identify_through(start, end, tool.radius_mm, policy)
                        .map_err(circular_pocket_body::pocket_error)?,
                );
                through_origins.push(tool.origin.clone());
            } else {
                pockets.push(
                    stock
                        .identify_blind(start, end, tool.radius_mm, policy)
                        .map_err(circular_pocket_body::pocket_error)?,
                );
                pocket_origins.push(tool.origin.clone());
            }
        }
        self.share()?
            .with_orthogonal_round_cuts(pockets, pocket_origins, through, through_origins)
    }
}

impl OrthogonalBody {
    fn identify_blind(
        &self,
        start: Point,
        end: Point,
        radius: f64,
        policy: GeometryPolicy,
    ) -> Result<OrthogonalPocket, orthogonal_pockets::PocketError> {
        orthogonal_pockets::identify_pocket(
            &self.geometry,
            &self.boundary,
            start,
            end,
            radius,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
    }

    fn identify_through(
        &self,
        start: Point,
        end: Point,
        radius: f64,
        policy: GeometryPolicy,
    ) -> Result<OrthogonalThroughBore, orthogonal_pockets::PocketError> {
        orthogonal_pockets::identify_through_bore(
            &self.geometry,
            &self.boundary,
            start,
            end,
            radius,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
    }

    pub(in crate::geometry) fn volume_mm3(&self) -> f64 {
        orthogonal_pockets::volume_with_through_bores_mm3(
            &self.geometry,
            &self.pockets,
            &self.through_bores,
        ) - multicad_rust_kernel::orthogonal_slots::removed_volume_mm3(&self.slots)
    }

    pub(in crate::geometry) fn supports_step(&self) -> bool {
        if !self.through_bores.is_empty() {
            return self.slots.is_empty()
                && step::supports_drilled_orthogonal(
                    &self.boundary,
                    &self.pockets,
                    &self.through_bores,
                );
        }
        if self.slots.is_empty() {
            step::supports_pocketed_orthogonal(&self.boundary, &self.pockets)
        } else {
            step::supports_slotted_orthogonal(&self.boundary, &self.pockets, &self.slots)
        }
    }

    pub(in crate::geometry) fn append_step(
        &self,
        writer: &mut step::Writer,
        frame: PlaneFrame,
    ) -> Result<(), step::StepError> {
        let placed = |point| transform_point(frame, point);
        if !self.through_bores.is_empty() {
            writer.append_drilled_orthogonal(
                &self.geometry,
                &self.boundary,
                &self.pockets,
                &self.through_bores,
                &placed,
            )
        } else if !self.slots.is_empty() {
            writer.append_slotted_orthogonal(
                &self.geometry,
                &self.boundary,
                &self.pockets,
                &self.slots,
                &placed,
            )
        } else if self.pockets.is_empty() {
            writer.append_orthogonal(&self.boundary, placed)
        } else {
            writer.append_pocketed_orthogonal(
                &self.geometry,
                &self.boundary,
                &self.pockets,
                &placed,
            )
        }
    }

    pub(super) fn append_through_faces(&self, faces: &mut Vec<Face>) {
        for bore in self.through_bores.iter() {
            for (face, center) in bore.faces.into_iter().zip(bore.centers_mm) {
                subtract_face_area(&mut faces[face], center, bore.area_mm2());
            }
            let [a, b] = bore.centers_mm;
            faces.push(Face {
                index: faces.len() as u32,
                plane: None,
                center_mm: std::array::from_fn(|i| a[i].midpoint(b[i])),
                area_mm2: TAU * bore.radius_mm * bore.depth_mm(),
            });
        }
    }

    pub(super) fn append_through_topology(&self, snapshot: &mut TopologySnapshot) {
        for (i, (bore, origin)) in self
            .through_bores
            .iter()
            .zip(&*self.through_origins)
            .enumerate()
        {
            let wall_origins = BTreeSet::from([
                circular_pocket_body::face_origin(origin, "wall"),
                origin.clone(),
            ]);
            let mut adjacent = BTreeSet::new();
            for (rim, (mouth, center)) in bore.faces.into_iter().zip(bore.centers_mm).enumerate() {
                let face = &mut snapshot.elements[mouth];
                face.boundary.extend(wall_origins.iter().cloned());
                let origins = face.origins.union(&wall_origins).cloned().collect();
                adjacent.extend(face.origins.iter().cloned());
                let normal = self.boundary.faces[mouth].normal;
                let u = std::array::from_fn(|j| f64::from(j == (axis(normal) + 1) % 3));
                let mut rim = element(
                    TopologyKind::Edge,
                    self.edge_count as usize + 2 * (self.pockets.len() + i) + rim,
                    origins,
                    AnalyticGeometry::CircularArc {
                        frame: ReferenceFrame {
                            origin_mm: center,
                            u,
                            v: cross(normal, u),
                        },
                        radius_mm: bore.radius_mm,
                        start_rad: 0.,
                        sweep_rad: TAU,
                    },
                );
                rim.boundary = rim.origins.clone();
                snapshot.elements.push(rim);
            }
            snapshot.elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: (self.faces.len() + 2 * self.pockets.len() + i) as u32,
                },
                origins: wall_origins,
                boundary: adjacent,
                frames: vec![],
                geometry: None,
            });
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
