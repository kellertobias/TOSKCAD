//! Semantic prism sources and exact native ancestry, separate from display data.
use crate::{
    AffineTransform, GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile,
    ProfileEdge, RigidTransform, Solid, ffi,
};
use multicad_core::{document::EntityId, references::*};
use std::collections::{BTreeMap, BTreeSet};

fn failure(message: impl std::fmt::Display) -> GeometryError {
    GeometryError {
        operation: "provenance",
        kind: GeometryErrorKind::InvalidResult,
        message: message.to_string(),
    }
}
fn key(kind: u8, ordinal: u32) -> Result<TopologyKey, GeometryError> {
    Ok(TopologyKey {
        kind: match kind {
            0 => TopologyKind::Face,
            1 => TopologyKind::Edge,
            2 => TopologyKind::Vertex,
            _ => return Err(failure("Unknown native topology kind")),
        },
        ordinal,
    })
}
fn empty(key: TopologyKey) -> TopologyElement {
    TopologyElement {
        key,
        origins: BTreeSet::new(),
        boundary: BTreeSet::new(),
        frames: vec![],
        geometry: None,
    }
}
fn reference_frame(frame: PlaneFrame) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
    }
}
fn transformed(frame: ReferenceFrame, transform: RigidTransform) -> ReferenceFrame {
    let rotate = |v: [f64; 3]| {
        std::array::from_fn(|i| {
            transform.rotation[i]
                .iter()
                .zip(v)
                .map(|(a, b)| a * b)
                .sum()
        })
    };
    let origin: [f64; 3] = rotate(frame.origin_mm);
    ReferenceFrame {
        origin_mm: std::array::from_fn(|i| origin[i] + transform.translation_mm[i]),
        u: rotate(frame.u),
        v: rotate(frame.v),
    }
}
fn side_frame(profile: &Profile, edge: &ProfileEdge, height: f64) -> Option<ReferenceFrame> {
    let ProfileEdge::Line { a, b } = edge else {
        return None;
    };
    let a = profile.frame.point(*a);
    let b = profile.frame.point(*b);
    let length = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
    Some(ReferenceFrame {
        origin_mm: a,
        u: std::array::from_fn(|i| (b[i] - a[i]) / length),
        v: profile.frame.normal().map(|v| v * height.signum()),
    })
}

impl Solid {
    /// Curve origins are supplied in authored outer-wire then hole-wire order.
    /// Rewinding a wire for OCCT preserves each curve's supplied identity.
    pub fn extrude_profile_named(
        profile: &Profile,
        height_mm: f64,
        feature: EntityId,
        curves: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure("Feature identity is nil"));
        }
        let edges: Vec<_> = profile
            .outer
            .iter()
            .chain(profile.holes.iter().flatten())
            .collect();
        if curves.len() != edges.len()
            || curves.iter().collect::<BTreeSet<_>>().len() != curves.len()
        {
            return Err(failure(
                "Every profile curve needs distinct source provenance",
            ));
        }
        // Validate before invoking native code, including the qualified side role.
        for curve in curves {
            let probe = GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins: BTreeSet::from([
                    curve.clone(),
                    Origin {
                        source: feature,
                        role: format!("side/{}/{}", curve.source, curve.role),
                    },
                ]),
                boundary: None,
                attachment: None,
            };
            probe.validate().map_err(failure)?;
        }
        let mut result = Self::extrude_profile(profile, height_mm, policy)?;
        let side_frames: Vec<_> = edges
            .iter()
            .map(|edge| side_frame(profile, edge, height_mm))
            .collect();
        result.name_profile_prism(profile.frame, height_mm, feature, curves, &side_frames)?;
        Ok(result)
    }
    pub(crate) fn name_profile_prism(
        &mut self,
        frame: PlaneFrame,
        height_mm: f64,
        feature: EntityId,
        curves: &[Origin],
        side_frames: &[Option<ReferenceFrame>],
    ) -> Result<(), GeometryError> {
        self.name_profile_vector_prism(
            frame,
            frame.normal().map(|n| n * height_mm),
            feature,
            curves,
            side_frames,
        )
    }
    pub(crate) fn name_profile_vector_prism(
        &mut self,
        frame: PlaneFrame,
        displacement_mm: [f64; 3],
        feature: EntityId,
        curves: &[Origin],
        side_frames: &[Option<ReferenceFrame>],
    ) -> Result<(), GeometryError> {
        let start = reference_frame(frame);
        let mut end = start;
        end.origin_mm = std::array::from_fn(|i| start.origin_mm[i] + displacement_mm[i]);
        self.name_profile_sweep([Some(start), Some(end)], feature, curves, side_frames)
    }
    pub(crate) fn name_profile_sweep(
        &mut self,
        cap_frames: [Option<ReferenceFrame>; 2],
        feature: EntityId,
        curves: &[Origin],
        side_frames: &[Option<ReferenceFrame>],
    ) -> Result<(), GeometryError> {
        if feature.is_nil()
            || curves.len() != side_frames.len()
            || curves.iter().collect::<BTreeSet<_>>().len() != curves.len()
        {
            return Err(failure(
                "Every exact profile edge needs a distinct authored origin",
            ));
        }
        for curve in curves {
            GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins: BTreeSet::from([
                    curve.clone(),
                    Origin {
                        source: feature,
                        role: format!("side/{}/{}", curve.source, curve.role),
                    },
                ]),
                boundary: None,
                attachment: None,
            }
            .validate()
            .map_err(failure)?;
        }
        let result = self;
        let nodes = ffi::topology(&result.native.0).map_err(failure)?;
        let mut elements: BTreeMap<_, _> = nodes
            .iter()
            .map(|n| key(n.kind, n.index).map(|k| (k, empty(k))))
            .collect::<Result<_, _>>()?;
        for seed in ffi::seeds(&result.native.0).map_err(failure)? {
            let key = key(0, seed.face)?;
            let element = elements
                .get_mut(&key)
                .ok_or_else(|| failure("Native seed refers to absent face"))?;
            let (origin, frame, curve) = match seed.role {
                0 | 1 => {
                    let cap_frame = cap_frames[seed.role as usize]
                        .ok_or_else(|| failure("Native sweep supplied an unexpected cap face"))?;
                    (
                        Origin {
                            source: feature,
                            role: if seed.role == 0 {
                                "cap.start"
                            } else {
                                "cap.end"
                            }
                            .into(),
                        },
                        Some(cap_frame),
                        None,
                    )
                }
                2 => {
                    let index = seed.source_index as usize;
                    let curve = curves
                        .get(index)
                        .ok_or_else(|| failure("Native seed refers to absent profile source"))?;
                    (
                        Origin {
                            source: feature,
                            role: format!("side/{}/{}", curve.source, curve.role),
                        },
                        *side_frames
                            .get(index)
                            .ok_or_else(|| failure("Native profile source index is absent"))?,
                        Some(curve.clone()),
                    )
                }
                _ => return Err(failure("Unknown native prism seed")),
            };
            if !element.origins.is_empty() {
                return Err(failure("Conflicting native face seeds"));
            }
            element.origins.insert(origin.clone());
            if let Some(curve) = curve {
                element.origins.insert(curve);
            }
            if let Some(frame) = frame {
                element.frames.push(AnchoredFrame { origin, frame });
            }
        }
        if elements
            .values()
            .any(|e| e.key.kind == TopologyKind::Face && e.origins.is_empty())
        {
            return Err(failure(
                "Native prism did not provide complete face ancestry",
            ));
        }
        result.finish_provenance(elements, &nodes)?;
        Ok(())
    }
    /// Simultaneously relabel this feature's default start/end caps. Used for
    /// the backward half of a two-sided sweep before exact union. Geometry and
    /// attachment frames stay unchanged; foreign feature provenance is retained.
    pub fn with_feature_cap_roles(
        mut self,
        feature: EntityId,
        start: &str,
        end: &str,
    ) -> Result<Self, GeometryError> {
        let invalid = |message: &str| GeometryError {
            operation: "cap roles",
            kind: GeometryErrorKind::Input,
            message: message.into(),
        };
        if feature.is_nil()
            || start.trim().is_empty()
            || end.trim().is_empty()
            || start.len() > 256
            || end.len() > 256
            || start == end
        {
            return Err(invalid(
                "Cap roles need a non-nil feature and distinct nonblank names of at most 256 bytes",
            ));
        }
        let old = [
            Origin {
                source: feature,
                role: "cap.start".into(),
            },
            Origin {
                source: feature,
                role: "cap.end".into(),
            },
        ];
        let mut found = [false; 2];
        let faces = self.faces()?;
        for element in &self.provenance {
            for origin in element
                .origins
                .iter()
                .chain(&element.boundary)
                .chain(element.frames.iter().map(|f| &f.origin))
            {
                if origin.source == feature
                    && !old.contains(origin)
                    && (origin.role == start || origin.role == end)
                {
                    return Err(invalid(
                        "Cap role collides with other provenance owned by this feature",
                    ));
                }
            }
            if element.key.kind != TopologyKind::Face {
                continue;
            }
            for (i, cap) in old.iter().enumerate() {
                if element.origins.contains(cap) {
                    if !element.frames.iter().any(|frame| frame.origin == *cap)
                        || !faces
                            .iter()
                            .any(|face| face.index == element.key.ordinal && face.plane.is_some())
                    {
                        return Err(invalid(
                            "Cap origin also identifies non-cap or unanchored geometry; explicit repair is required",
                        ));
                    }
                    found[i] = true;
                }
            }
        }
        if !found.into_iter().all(|present| present) {
            return Err(invalid(
                "Both owned start/end planar caps are required for a cap-role override",
            ));
        }
        let remap = |origin: &Origin| {
            if origin == &old[0] {
                Origin {
                    source: feature,
                    role: start.into(),
                }
            } else if origin == &old[1] {
                Origin {
                    source: feature,
                    role: end.into(),
                }
            } else {
                origin.clone()
            }
        };
        for element in &mut self.provenance {
            element.origins = element.origins.iter().map(&remap).collect();
            element.boundary = element.boundary.iter().map(&remap).collect();
            for frame in &mut element.frames {
                frame.origin = remap(&frame.origin);
            }
        }
        self.topology_snapshot(feature, 0)?;
        Ok(self)
    }
    pub fn box_named(
        size_mm: [f64; 3],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if size_mm.iter().any(|v| {
            !v.is_finite() || *v < policy.minimum_feature_mm || *v > policy.maximum_coordinate_mm
        }) {
            return Err(GeometryError {
                operation: "box",
                kind: GeometryErrorKind::Input,
                message: "Box dimensions must be positive and within the numerical policy".into(),
            });
        }
        let curves: Vec<_> = ["y.min", "x.max", "y.max", "x.min"]
            .into_iter()
            .map(|role| Origin {
                source: feature,
                role: role.into(),
            })
            .collect();
        Self::extrude_profile_named(
            &Profile::rectangle(size_mm[0], size_mm[1]),
            size_mm[2],
            feature,
            &curves,
            policy,
        )
    }
    /// Evaluation data: source Links persist intent, never these local ordinals.
    pub fn topology_snapshot(
        &self,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let mut elements: Vec<TopologyElement> = if self.provenance.is_empty() {
            ffi::topology(&self.native.0)
                .map_err(failure)?
                .iter()
                .map(|n| key(n.kind, n.index).map(empty))
                .collect::<Result<_, _>>()?
        } else {
            self.provenance.clone()
        };
        let xyz = |p: ffi::Point3| [p.x, p.y, p.z];
        let mut geometry = BTreeMap::new();
        for edge in ffi::analytic_edges(&self.native.0).map_err(failure)? {
            let exact = match edge.kind {
                0 => AnalyticGeometry::Line {
                    a_mm: xyz(edge.a),
                    b_mm: xyz(edge.b),
                },
                1 => AnalyticGeometry::CircularArc {
                    frame: ReferenceFrame {
                        origin_mm: xyz(edge.frame.origin),
                        u: xyz(edge.frame.u),
                        v: xyz(edge.frame.v),
                    },
                    radius_mm: edge.radius,
                    start_rad: edge.start,
                    sweep_rad: edge.sweep,
                },
                _ => return Err(failure("Unknown native analytic edge kind")),
            };
            geometry.insert(key(1, edge.index)?, exact);
        }
        for vertex in ffi::vertices(&self.native.0).map_err(failure)? {
            geometry.insert(
                key(2, vertex.index)?,
                AnalyticGeometry::Point {
                    position_mm: xyz(vertex.point),
                },
            );
        }
        for face in self.faces()? {
            if let Some((origin_mm, normal)) = face.plane {
                let seed = if normal[0].abs() < 0.9 {
                    [1., 0., 0.]
                } else {
                    [0., 1., 0.]
                };
                let dot = seed.iter().zip(normal).map(|(a, b)| a * b).sum::<f64>();
                let mut u: [f64; 3] = std::array::from_fn(|i| seed[i] - normal[i] * dot);
                let length = u.iter().map(|v| v * v).sum::<f64>().sqrt();
                u = u.map(|v| v / length);
                let v = [
                    normal[1] * u[2] - normal[2] * u[1],
                    normal[2] * u[0] - normal[0] * u[2],
                    normal[0] * u[1] - normal[1] * u[0],
                ];
                geometry.insert(
                    key(0, face.index)?,
                    AnalyticGeometry::Plane {
                        frame: ReferenceFrame { origin_mm, u, v },
                    },
                );
            }
        }
        for element in &mut elements {
            // Native supported geometry takes precedence. Proven affine
            // Plane/Line/Point descriptors remain exact when OCCT stores their
            // output as NURBS; these are never inferred from sampled meshes.
            if let Some(current) = geometry.remove(&element.key) {
                element.geometry = Some(current);
            }
        }
        let snapshot = TopologySnapshot {
            owner,
            revision,
            elements,
        };
        snapshot.validate().map_err(failure)?;
        Ok(snapshot)
    }
    pub(crate) fn inherit_provenance(
        &mut self,
        operands: &[&Self],
        transform: Option<RigidTransform>,
    ) -> Result<(), GeometryError> {
        if operands.iter().all(|s| s.provenance.is_empty()) {
            return Ok(());
        }
        let nodes = ffi::topology(&self.native.0).map_err(failure)?;
        let mut elements: BTreeMap<_, _> = nodes
            .iter()
            .map(|n| key(n.kind, n.index).map(|k| (k, empty(k))))
            .collect::<Result<_, _>>()?;
        let inputs: Vec<BTreeMap<_, _>> = operands
            .iter()
            .map(|s| s.provenance.iter().map(|e| (e.key, e)).collect())
            .collect();
        let mut frames: BTreeMap<TopologyKey, BTreeMap<Origin, Option<ReferenceFrame>>> =
            BTreeMap::new();
        for relation in ffi::ancestry(&self.native.0).map_err(failure)? {
            let out = key(relation.kind, relation.output)?;
            let source = key(relation.kind, relation.input)?;
            let operand = inputs
                .get(relation.operand as usize)
                .ok_or_else(|| failure("Invalid native history operand"))?;
            let Some(input) = operand.get(&source) else {
                continue;
            };
            let element = elements
                .get_mut(&out)
                .ok_or_else(|| failure("Invalid native history result"))?;
            element.origins.extend(input.origins.iter().cloned());
            if let Some(transform) = transform
                && let Some(geometry) = input.geometry
            {
                element.geometry = AffineTransform {
                    matrix: transform.rotation,
                    translation_mm: transform.translation_mm,
                }
                .analytic(geometry)?;
            }
            for anchored in &input.frames {
                let frame = transform.map_or(anchored.frame, |t| transformed(anchored.frame, t));
                frames
                    .entry(out)
                    .or_default()
                    .entry(anchored.origin.clone())
                    .and_modify(|old| {
                        if *old != Some(frame) {
                            *old = None;
                        }
                    })
                    .or_insert(Some(frame));
            }
        }
        for (key, origins) in frames {
            let element = elements.get_mut(&key).expect("mapped native element");
            element.frames = origins
                .into_iter()
                .filter_map(|(origin, frame)| frame.map(|frame| AnchoredFrame { origin, frame }))
                .collect();
        }
        self.finish_provenance(elements, &nodes)
    }
    pub(crate) fn inherit_affine_provenance(
        &mut self,
        source: &Self,
        transform: AffineTransform,
        owner: EntityId,
    ) -> Result<(), GeometryError> {
        let snapshot = source.topology_snapshot(owner, 0)?;
        let inputs: BTreeMap<_, _> = snapshot.elements.iter().map(|e| (e.key, e)).collect();
        let nodes = ffi::topology(&self.native.0).map_err(failure)?;
        let mut elements: BTreeMap<_, _> = nodes
            .iter()
            .map(|n| key(n.kind, n.index).map(|k| (k, empty(k))))
            .collect::<Result<_, _>>()?;
        let mut mapped = BTreeSet::new();
        for relation in ffi::ancestry(&self.native.0).map_err(failure)? {
            let out = key(relation.kind, relation.output)?;
            if relation.operand != 0 || !mapped.insert(out) {
                return Err(failure("Affine history is not a unique source mapping"));
            }
            let input = inputs
                .get(&key(relation.kind, relation.input)?)
                .ok_or_else(|| failure("Affine source topology is absent"))?;
            let element = elements
                .get_mut(&out)
                .ok_or_else(|| failure("Affine output topology is absent"))?;
            element.origins = input.origins.clone();
            element.frames = input
                .frames
                .iter()
                .map(|a| {
                    Ok(AnchoredFrame {
                        origin: a.origin.clone(),
                        frame: transform.frame(a.frame)?,
                    })
                })
                .collect::<Result<_, GeometryError>>()?;
            element.geometry = input
                .geometry
                .map(|g| transform.analytic(g))
                .transpose()?
                .flatten();
        }
        if mapped.len() != elements.len() {
            return Err(failure("Affine output has incomplete source topology"));
        }
        self.finish_provenance(elements, &nodes)
    }
    fn finish_provenance(
        &mut self,
        mut elements: BTreeMap<TopologyKey, TopologyElement>,
        nodes: &[ffi::TopologyNode],
    ) -> Result<(), GeometryError> {
        let faces: BTreeMap<_, _> = elements
            .values()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .map(|e| (e.key.ordinal, e.origins.clone()))
            .collect();
        for node in nodes {
            let element = elements
                .get_mut(&key(node.kind, node.index)?)
                .expect("enumerated element");
            for index in &node.incident_faces {
                let origins = faces
                    .get(index)
                    .ok_or_else(|| failure("Invalid native adjacency face"))?;
                element.boundary.extend(origins.iter().cloned());
            }
            if element.key.kind != TopologyKind::Face {
                element.origins.extend(element.boundary.iter().cloned());
            }
        }
        self.provenance = elements.into_values().collect();
        // Validate native-derived data before it can enter the shared API.
        self.topology_snapshot(EntityId::new(), 0)?;
        Ok(())
    }
}
