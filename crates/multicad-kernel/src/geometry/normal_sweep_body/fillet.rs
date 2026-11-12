//! Full-length parallel box-edge blends as one exact circular normal sweep.
use super::*;
use crate::TreatmentEdge;
use multicad_core::references::{TopologyElement, TopologyKey, TopologyKind};

fn unsupported() -> GeometryError {
    error(
        "fillet",
        GeometryErrorKind::Unsupported,
        "Rust fillet requires complete parallel straight edges of a rectangular box",
    )
}

impl Solid {
    /// Keep the authored box chart exact while treating a rigidly placed body.
    /// Inverting world coordinates would introduce residual skew in straight
    /// edges that the box treatment deliberately checks without snapping.
    pub(crate) fn local_box_treatment_source(
        &self,
    ) -> Result<Option<(Self, crate::RigidTransform)>, GeometryError> {
        let frame = match &self.body {
            SolidBody::Prism(_, frame)
            | SolidBody::Linear(_, frame)
            | SolidBody::Orthogonal(_, frame)
            | SolidBody::PlanarBoolean(_, frame) => *frame,
            SolidBody::PolygonPrism(body) => body.frame,
            _ => return Ok(None),
        };
        if frame == PlaneFrame::default() {
            return Ok(None);
        }
        let mut source = self.share()?;
        // Metadata overrides store authored attachment frames in world space.
        // Resetting only the physical body chart would apply its placement a
        // second time to those anchors when the treated result is restored.
        if source.has_authored_metadata_override() {
            for element in &mut source.provenance {
                for anchor in &mut element.frames {
                    anchor.frame = super::super::revolution_body::local_attachment(
                        frame,
                        anchor.frame,
                        self.policy,
                    )
                    .map_err(|_| {
                        error(
                            "edge treatment",
                            GeometryErrorKind::Unsupported,
                            "Placed box attachment cannot be preserved within model tolerance",
                        )
                    })?;
                }
            }
        }
        source.retained_placement = None;
        match &mut source.body {
            SolidBody::Prism(_, frame)
            | SolidBody::Linear(_, frame)
            | SolidBody::Orthogonal(_, frame)
            | SolidBody::PlanarBoolean(_, frame) => *frame = PlaneFrame::default(),
            SolidBody::PolygonPrism(body) => Rc::make_mut(body).frame = PlaneFrame::default(),
            _ => unreachable!("checked box representation"),
        }
        Ok(Some((
            source,
            crate::RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
                translation_mm: frame.origin,
            },
        )))
    }

    pub(crate) fn rust_fillet_box_edges(
        &self,
        edges: &[TreatmentEdge],
        radius: f64,
        feature: EntityId,
        policy: GeometryPolicy,
        topology: &TopologySnapshot,
    ) -> Result<Self, GeometryError> {
        if self.policy != policy || policy.boolean_fuzzy_mm != 0. || edges.len() > 4 {
            return Err(unsupported());
        }
        if let Some((source, placement)) = self.local_box_treatment_source()? {
            let local = source.topology_snapshot(topology.owner, topology.revision)?;
            return source
                .rust_fillet_box_edges(edges, radius, feature, policy, &local)?
                .transformed(placement);
        }
        let cells = self.orthogonal_cells()?.ok_or_else(unsupported)?;
        let [stock] = cells.as_slice() else {
            return Err(unsupported());
        };
        let corner = |edge: &TreatmentEdge| {
            let Some(AnalyticGeometry::Line { a_mm, b_mm }) = topology
                .elements
                .iter()
                .find(|element| element.key == edge.key)
                .and_then(|element| element.geometry)
            else {
                return Err(unsupported());
            };
            let mut differing = (0..3).filter(|&axis| a_mm[axis] != b_mm[axis]);
            let along = differing.next().ok_or_else(unsupported)?;
            if differing.next().is_some()
                || a_mm[along].min(b_mm[along]) != stock.min[along]
                || a_mm[along].max(b_mm[along]) != stock.max[along]
                || (0..3).any(|axis| {
                    axis != along && a_mm[axis] != stock.min[axis] && a_mm[axis] != stock.max[axis]
                })
            {
                return Err(unsupported());
            }
            let mut point = a_mm;
            point[along] = stock.min[along];
            Ok((along, point))
        };
        let mut selected = edges
            .iter()
            .map(|edge| corner(edge).map(|(along, point)| (along, point, &edge.origin)))
            .collect::<Result<Vec<_>, _>>()?;
        selected.sort_unstable_by(|(_, a, _), (_, b, _)| {
            a[0].total_cmp(&b[0])
                .then_with(|| a[1].total_cmp(&b[1]))
                .then_with(|| a[2].total_cmp(&b[2]))
        });
        let Some(&(along, anchor, _)) = selected.first() else {
            return Err(unsupported());
        };
        let [across, up] = [(along + 1) % 3, (along + 2) % 3];
        let inward = |axis: usize| {
            if anchor[axis] == stock.min[axis] {
                1.
            } else {
                -1.
            }
        };
        let (across_sign, up_sign) = (inward(across), inward(up));
        let mut frame = PlaneFrame {
            origin: anchor,
            u: [0.; 3],
            v: [0.; 3],
        };
        frame.u[across] = across_sign;
        frame.v[up] = up_sign;
        frame.origin[along] = if across_sign == up_sign {
            stock.min[along]
        } else {
            stock.max[along]
        };
        let width = stock.max[across] - stock.min[across];
        let depth = stock.max[up] - stock.min[up];
        let height = stock.max[along] - stock.min[along];

        // Traverse the rectangle counterclockwise from the selected anchor.
        // This retains the side numbering and frame of single-edge fillets.
        let mut rounds = [None; 4];
        for (axis, point, origin) in selected {
            if axis != along {
                return Err(unsupported());
            }
            let index = match (point[across] != anchor[across], point[up] != anchor[up]) {
                (false, false) => 0,
                (true, false) => 1,
                (true, true) => 2,
                (false, true) => 3,
            };
            if rounds[index].replace(origin).is_some() {
                return Err(unsupported());
            }
        }
        let corners = [[0., 0.], [width, 0.], [width, depth], [0., depth]];
        let directions = [[1., 0.], [0., 1.], [-1., 0.], [0., -1.]];
        let clip = rounds.map(|round| if round.is_some() { radius } else { 0. });
        let shift = |point: [f64; 2], direction: [f64; 2], distance| {
            std::array::from_fn(|i| point[i] + direction[i] * distance)
        };
        let mut outer = Vec::with_capacity(4 + edges.len());
        let mut origins = Vec::with_capacity(outer.capacity());
        for (i, side) in [width, depth, width, depth].into_iter().enumerate() {
            let next = (i + 1) % 4;
            // Adjacent rounds must leave a positive straight face. Opposite
            // corner removals lie inside disjoint corner triangles since
            // each radius is smaller than both rectangle side lengths.
            policy.length(side - clip[i] - clip[next])?;
            let a = shift(corners[i], directions[i], clip[i]);
            let b = shift(corners[next], directions[i], -clip[next]);
            outer.push(CurvedEdge2d::Line { a, b });
            origins.push(Origin {
                source: feature,
                role: format!("fillet.side.{i}"),
            });
            if let Some(origin) = rounds[next] {
                outer.push(CurvedEdge2d::Arc {
                    center: shift(b, directions[next], radius),
                    a: b,
                    b: shift(corners[next], directions[next], radius),
                    clockwise: false,
                });
                origins.push(origin.clone());
            }
        }
        let profile = CurvedProfile {
            frame,
            outer,
            holes: Vec::new(),
        };
        let mut result =
            Self::rust_extrude_normal_profile_named(&profile, height, feature, &origins, policy)?;
        let SolidBody::NormalSweep(body, _) = &mut result.body else {
            unreachable!("constructed normal sweep");
        };
        let body = Rc::get_mut(body).expect("new normal sweep");
        let rounded = body.topology(frame, feature, 0);
        body.stock_history = Some(Rc::new(StockHistory::capture(
            topology,
            &rounded,
            frame,
            policy.linear_tolerance_mm,
        )?));
        result
            .topology_snapshot(feature, 0)?
            .validate()
            .map_err(|message| error("fillet", GeometryErrorKind::InvalidResult, message))?;
        Ok(result)
    }
}

pub(super) struct StockHistory {
    matches: Vec<(TopologyKey, BTreeSet<Origin>, Vec<AnchoredFrame>)>,
}

impl StockHistory {
    fn capture(
        stock: &TopologySnapshot,
        result: &TopologySnapshot,
        placement: PlaneFrame,
        tolerance: f64,
    ) -> Result<Self, GeometryError> {
        let mut matches = Vec::new();
        for element in &result.elements {
            let Some(geometry) = element.geometry else {
                continue;
            };
            let mut candidates = stock.elements.iter().filter(|old| {
                old.geometry
                    .is_some_and(|old| survives(old, geometry, tolerance))
            });
            let Some(source) = candidates.next() else {
                if element.key.kind == TopologyKind::Face {
                    return Err(unsupported());
                }
                continue;
            };
            if candidates.next().is_some() {
                return Err(unsupported());
            }
            matches.push((
                element.key,
                source.origins.clone(),
                source
                    .frames
                    .iter()
                    .map(|anchored| AnchoredFrame {
                        origin: anchored.origin.clone(),
                        frame: local_frame(placement, anchored.frame),
                    })
                    .collect(),
            ));
        }
        Ok(Self { matches })
    }

    pub(super) fn add_face_origins(&self, faces: &mut [Vec<Origin>]) {
        for (key, origins, _) in &self.matches {
            if key.kind == TopologyKind::Face {
                faces[key.ordinal as usize].extend(origins.iter().cloned());
            }
        }
    }

    pub(super) fn restore(&self, placement: PlaneFrame, elements: &mut [TopologyElement]) {
        for (key, origins, frames) in &self.matches {
            let element = elements
                .iter_mut()
                .find(|element| element.key == *key)
                .expect("normal sweep stock history");
            if key.kind == TopologyKind::Face {
                element
                    .frames
                    .extend(frames.iter().map(|anchored| AnchoredFrame {
                        origin: anchored.origin.clone(),
                        frame: placed_frame(placement, anchored.frame),
                    }));
            } else {
                element.origins.extend(origins.iter().cloned());
            }
        }
    }
}

fn vector_sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn local_frame(placement: PlaneFrame, world: ReferenceFrame) -> ReferenceFrame {
    let axes = [placement.u, placement.v, placement.normal()];
    let local = |vector| axes.map(|axis| dot(axis, vector));
    ReferenceFrame {
        origin_mm: local(vector_sub(world.origin_mm, placement.origin)),
        u: local(world.u),
        v: local(world.v),
    }
}

fn placed_frame(placement: PlaneFrame, local: ReferenceFrame) -> ReferenceFrame {
    let linear = PlaneFrame {
        origin: [0.; 3],
        ..placement
    };
    ReferenceFrame {
        origin_mm: position(placement, local.origin_mm),
        u: position(linear, local.u),
        v: position(linear, local.v),
    }
}

fn survives(old: AnalyticGeometry, result: AnalyticGeometry, tolerance: f64) -> bool {
    match (old, result) {
        (AnalyticGeometry::Plane { frame: a }, AnalyticGeometry::Plane { frame: b }) => {
            dot(a.normal(), b.normal()) > 1. - 1e-10
                && dot(a.normal(), vector_sub(a.origin_mm, b.origin_mm)).abs() <= tolerance
        }
        (AnalyticGeometry::Line { a_mm, b_mm }, AnalyticGeometry::Line { a_mm: a, b_mm: b }) => {
            let delta = vector_sub(b_mm, a_mm);
            let length = dot(delta, delta).sqrt();
            let unit = delta.map(|coordinate| coordinate / length);
            [a, b].into_iter().all(|point| {
                let offset = vector_sub(point, a_mm);
                let along = dot(unit, offset);
                let across = vector_sub(offset, unit.map(|coordinate| coordinate * along));
                (-tolerance..=length + tolerance).contains(&along)
                    && dot(across, across) <= tolerance * tolerance
            })
        }
        (
            AnalyticGeometry::Point { position_mm: a },
            AnalyticGeometry::Point { position_mm: b },
        ) => {
            let delta = vector_sub(a, b);
            dot(delta, delta) <= tolerance * tolerance
        }
        _ => false,
    }
}
