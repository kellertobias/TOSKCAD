//! Semantic prism sources and exact native ancestry, separate from display data.
use crate::{
    GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile, ProfileEdge,
    RigidTransform, Solid, ffi,
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
                    let mut frame = reference_frame(profile.frame);
                    if seed.role == 1 {
                        frame.origin_mm = std::array::from_fn(|i| {
                            frame.origin_mm[i] + profile.frame.normal()[i] * height_mm
                        });
                    }
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
                        Some(frame),
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
                        side_frame(profile, edges[index], height_mm),
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
        Ok(result)
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
        let elements = if self.provenance.is_empty() {
            ffi::topology(&self.native.0)
                .map_err(failure)?
                .iter()
                .map(|n| key(n.kind, n.index).map(empty))
                .collect::<Result<_, _>>()?
        } else {
            self.provenance.clone()
        };
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
