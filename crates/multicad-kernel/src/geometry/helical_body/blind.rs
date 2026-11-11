//! One exact cap-normal blind thread in certified orthogonal stock.
use super::*;
use multicad_rust_kernel::{
    display::{DisplayMesh, MeshError, Tessellation, blind_tap_stock_face, tapped_orthogonal},
    orthogonal_pockets::{OrthogonalPocket, identify_pocket},
};

/// Authored blind threaded cut and the material it must leave around the tool.
pub struct BlindThreadCut {
    pub profile: HelicalThread,
    pub entry: PlaneFrame,
    pub receiving_radius_mm: f64,
    pub minimum_wall_mm: f64,
    pub minimum_floor_mm: f64,
}

pub struct BlindThreadMating {
    pub threads: [(Thread, PlaneFrame); 2],
    pub protected_radius_mm: f64,
    pub entry_is_support_plane: bool,
}

pub(super) struct BlindStock {
    body: Rc<super::super::orthogonal_body::OrthogonalBody>,
    stock_in_thread: stock_frame::StoredFrame,
    authored_frames: std::collections::BTreeMap<Origin, ReferenceFrame>,
    pocket: OrthogonalPocket,
    thread_axes: [Point; 2],
    protected_radius_mm: f64,
}

impl Solid {
    pub fn rust_cut_blind_internal_thread(
        &self,
        cut: BlindThreadCut,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        cut.entry.checked(policy)?;
        for length in [
            cut.receiving_radius_mm,
            cut.minimum_wall_mm,
            cut.minimum_floor_mm,
            cut.profile.length_mm,
        ] {
            policy.length(length)?;
        }
        if self.policy != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(input(
                "Blind tap requires the stock's exact geometry policy",
            ));
        }
        let stock = self.orthogonal_stock()?;
        let authored = self.topology_snapshot(feature, 0)?;
        let SolidBody::Orthogonal(body, stock_frame) = &stock.body else {
            unreachable!("orthogonal stock promotion");
        };
        if body.geometry.solid_count() != 1 || body.has_curved_cuts() || body.feature == feature {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Blind tap requires one distinct orthogonal stock solid without prior curved cuts",
            ));
        }
        let stock_frame = *stock_frame;
        let stock_axes = [stock_frame.u, stock_frame.v, stock_frame.normal()];
        let stock_chart = stock_frame::Chart::new(stock_frame);
        let local_vector = |world| stock_chart.vector(world);
        let local_point = |world| stock_chart.point(world);
        let tolerance = policy.linear_tolerance_mm;
        let overrun = policy.minimum_feature_mm.max(8. * tolerance);
        let protected_radius_mm =
            cut.receiving_radius_mm.max(cut.profile.major_radius_mm) + cut.minimum_wall_mm;
        policy.length(protected_radius_mm)?;
        let witness = identify_pocket(
            &body.geometry,
            &body.boundary,
            local_point(position(cut.entry, [0., 0., -overrun])),
            local_point(position(
                cut.entry,
                [0., 0., cut.profile.length_mm + cut.minimum_floor_mm],
            )),
            protected_radius_mm,
            tolerance,
            policy.minimum_feature_mm,
        )
        .map_err(crate::geometry::circular_pocket_body::pocket_error)?;
        let local_entry = local_point(cut.entry.origin);
        let [x, y, z] = std::array::from_fn(|i| witness.center_mm[i] - local_entry[i]);
        if x.hypot(y).hypot(z) > tolerance {
            return Err(input("Blind tap does not start at the declared stock face"));
        }
        let inward = body.boundary.faces[witness.face].normal.map(|v| -v);
        let u = local_vector(cut.entry.u);
        let axial = dot(u, inward);
        let planar = std::array::from_fn(|i| u[i] - axial * inward[i]);
        let length = dot(planar, planar).sqrt();
        let u = planar.map(|value| value / length);
        let v = cross(inward, u);
        let direction = |local| std::array::from_fn(|i| dot(local, stock_axes.map(|axis| axis[i])));
        let placed = PlaneFrame {
            origin: position(stock_frame, witness.center_mm),
            u: direction(u),
            v: direction(v),
        };
        let stock_in_thread = stock_frame::store(
            placed,
            stock_frame,
            body.bounds(PlaneFrame::default()),
            policy,
        )?;
        let blind = BlindStock {
            body: Rc::clone(body),
            stock_in_thread,
            authored_frames: stock_frame::store_anchors(placed, authored, policy)?,
            pocket: OrthogonalPocket {
                radius_mm: cut.profile.major_radius_mm,
                depth_mm: cut.profile.length_mm,
                ..witness
            },
            thread_axes: [u, v],
            protected_radius_mm,
        };
        Self::rust_helical_body(cut.profile, None, feature, placed, policy, |_| {
            Ok(HelicalForm::Blind(blind))
        })
    }

    /// The exact screw and blind cut profiles, plus the radius of the fully
    /// protected stock collar. These values come from current physical solids.
    pub fn rust_screw_blind_thread(&self, receiver: &Self) -> Option<BlindThreadMating> {
        let (screw, screw_frame) = self.helical_body()?;
        let (receiver, receiver_frame) = receiver.helical_body()?;
        let HelicalForm::Blind(stock) = &receiver.form else {
            return None;
        };
        matches!(screw.form, HelicalForm::Screw(_)).then_some(BlindThreadMating {
            threads: [
                (screw.geometry, screw_frame),
                (receiver.geometry, receiver_frame),
            ],
            protected_radius_mm: stock.protected_radius_mm,
            entry_is_support_plane: stock.entry_is_support_plane(),
        })
    }
}

impl BlindStock {
    fn entry_is_support_plane(&self) -> bool {
        let normal = self.body.boundary.faces[self.pocket.face].normal;
        let bounds = self.body.geometry.bounds();
        normal.into_iter().enumerate().all(|(axis, sign)| {
            sign == 0.
                || self.pocket.center_mm[axis]
                    == if sign > 0. {
                        bounds.max[axis]
                    } else {
                        bounds.min[axis]
                    }
        })
    }

    fn stock_frame(&self, thread_frame: PlaneFrame) -> PlaneFrame {
        self.stock_in_thread.placed(thread_frame)
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        self.stock_in_thread
            .bounds(frame, self.body.geometry.cells())
    }

    pub(super) fn volume_mm3(&self, geometry: Thread) -> f64 {
        self.body.geometry.volume_mm3() - geometry.volume_mm3()
    }

    pub(super) fn contains(&self, point: Point, tolerance: f64) -> bool {
        let chart = self.stock_in_thread.local(point);
        self.body.geometry.cells().iter().any(|cell| {
            (0..3)
                .all(|i| cell.min[i] - tolerance <= chart[i] && chart[i] <= cell.max[i] + tolerance)
        })
    }

    pub(super) fn stock_faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.body.faces(self.stock_frame(frame))
    }

    pub(super) fn stock_face_index(&self, index: usize, thread_faces: usize) -> usize {
        let original = index - thread_faces;
        original + usize::from(original >= self.pocket.face)
    }

    pub(super) fn extend_properties(
        &self,
        properties: &mut Vec<(Point, f64)>,
        hole_area: f64,
        mouth_moment: [f64; 2],
    ) {
        for (index, face) in self
            .body
            .faces(PlaneFrame::default())
            .into_iter()
            .enumerate()
        {
            let center = self.stock_in_thread.point(face.center_mm);
            if index == self.pocket.face {
                let area = face.area_mm2 - hole_area;
                let mut center = center;
                for i in 0..2 {
                    center[i] = (center[i] * face.area_mm2 - mouth_moment[i]) / area;
                }
                properties[0] = (center, area);
            } else {
                properties.push((center, face.area_mm2));
            }
        }
    }

    pub(super) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let mut topology = self
            .body
            .topology(self.stock_frame(frame), owner, revision)
            .map_err(|cause| error(OPERATION, GeometryErrorKind::InvalidResult, cause))?;
        for anchor in topology.elements.iter_mut().flat_map(|e| &mut e.frames) {
            if let Some(authored) = self.authored_frames.get(&anchor.origin) {
                anchor.frame =
                    boundary::placed_frame(frame, authored.origin_mm, authored.u, authored.v);
            }
        }
        Ok(topology)
    }

    pub(super) fn extend_topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
        rails: usize,
        elements: &mut Vec<TopologyElement>,
    ) -> Result<(), GeometryError> {
        for mut element in self.topology(frame, owner, revision)?.elements {
            let ordinal = element.key.ordinal as usize;
            match element.key.kind {
                TopologyKind::Face if ordinal == self.pocket.face => {
                    let entry = &mut elements[0];
                    entry.origins.extend(element.origins);
                    entry.boundary.extend(element.boundary);
                    entry.frames.extend(element.frames);
                    continue;
                }
                TopologyKind::Face => {
                    element.key.ordinal = blind_tap_stock_face(ordinal, self.pocket.face, 2 + rails)
                }
                TopologyKind::Edge => element.key.ordinal += (3 * rails) as u32,
                TopologyKind::Vertex => element.key.ordinal += (2 * rails) as u32,
            }
            elements.push(element);
        }
        Ok(())
    }

    pub(super) fn mesh(
        &self,
        geometry: &Thread,
        options: Tessellation,
    ) -> Result<DisplayMesh, MeshError> {
        let mut mesh = tapped_orthogonal(
            &self.body.boundary,
            geometry,
            self.pocket,
            self.thread_axes,
            options,
        )?;
        for point in mesh.vertices.as_chunks_mut::<3>().0 {
            *point = self.stock_in_thread.point(*point);
        }
        Ok(mesh)
    }
}
