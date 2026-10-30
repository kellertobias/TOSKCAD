//! Exact current planar-face boundaries as Extrude/Revolve profiles (CAD-146).
//!
//! The persistent selector is the canonical `profile/face` link on the
//! feature. It is resolved only through the shared reference inspection; a
//! missing, ambiguous or stale face is reported as an unresolved reference
//! that needs repair and never retargets to a nearby face. The profile is the
//! face's exact native outer wire and holes, expressed in the resolved planar
//! attachment frame oriented so that +N is the current outward face normal.
use crate::{ModelError, compiler::error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    manufacturing::PlanarEdge,
    model::PROFILE_FACE_ROLE,
    references::{self, Origin, Resolution, TopologyKey, TopologySnapshot},
};
use multicad_kernel::{CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid};
use std::collections::{BTreeMap, BTreeSet};

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn curved(edge: &PlanarEdge) -> CurvedEdge2d {
    match *edge {
        PlanarEdge::Line { a, b } => CurvedEdge2d::Line { a, b },
        PlanarEdge::Arc {
            a,
            center,
            sweep_rad,
            ..
        } if (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-9 => CurvedEdge2d::Circle {
            center,
            radius_mm: (a[0] - center[0]).hypot(a[1] - center[1]),
        },
        PlanarEdge::Arc {
            a,
            b,
            center,
            sweep_rad,
        } => CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise: sweep_rad < 0.,
        },
    }
}

pub(crate) struct FaceProfileInputs<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub feature: EntityId,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
}

impl FaceProfileInputs<'_> {
    fn failure(&self, message: impl std::fmt::Display) -> ModelError {
        error(Some(self.feature), message)
    }
    /// Resolve the profile face reference `reference` of this feature to its
    /// exact current boundary and semantic side-edge provenance.
    pub fn resolve(&self, reference: EntityId) -> Result<(CurvedProfile, Vec<Origin>), ModelError> {
        let Some(EntityData::Feature { inputs, .. }) =
            self.document.get(self.feature).map(|e| &e.data)
        else {
            return Err(self.failure("Missing profile feature"));
        };
        let link = inputs
            .iter()
            .find(|link| {
                link.role == PROFILE_FACE_ROLE
                    && link.geometry.as_ref().is_some_and(|i| i.id == reference)
            })
            .ok_or_else(|| self.failure("Profile face input is missing; repair its reference"))?;
        let report = references::inspect(self.document, self.revision, self.topologies);
        let diagnostic = report
            .get(references::ReferenceAddress {
                owner: self.feature,
                reference,
            })
            .ok_or_else(|| self.failure("Profile face has no current reference evaluation"))?;
        let Resolution::Resolved {
            key,
            frame: Some(frame),
            ..
        } = diagnostic.resolution
        else {
            return Err(self.failure(format!(
                "Profile face reference is unresolved ({:?}); repair the reference to choose its face again",
                diagnostic.resolution
            )));
        };
        frame.validate().map_err(|e| self.failure(e))?;
        let solid = self
            .solids
            .get(&link.target)
            .ok_or_else(|| self.failure("Profile face producer has no current solid"))?;
        let face = solid
            .faces()
            .map_err(|e| self.failure(e))?
            .into_iter()
            .find(|face| face.index == key.ordinal)
            .ok_or_else(|| self.failure("Profile face is missing from the current solid"))?;
        let (origin, outward) = face.plane.ok_or_else(|| {
            self.failure("This face is curved; Extrude and Revolve need a planar face")
        })?;
        let normal = frame.normal();
        if dot(normal, outward).abs() < 1. - 1e-8
            || dot(
                std::array::from_fn(|i| frame.origin_mm[i] - origin[i]),
                outward,
            )
            .abs()
                > self.policy.linear_tolerance_mm * 10.
        {
            return Err(self.failure(
                "The profile face frame no longer lies on its selected native face; repair the reference",
            ));
        }
        // +N always follows the current OUTWARD normal: positive distances add
        // material outside the source body regardless of inherited frame side.
        let plane = PlaneFrame {
            origin: frame.origin_mm,
            u: frame.u,
            v: if dot(normal, outward) < 0. {
                frame.v.map(|n| -n)
            } else {
                frame.v
            },
        };
        let sourced = solid
            .planar_face_contours_with_sources(key.ordinal, plane)
            .map_err(|e| self.failure(format!("Profile face boundary is unsupported: {e}")))?;
        let mut outer = None;
        let mut holes = vec![];
        let mut keys = vec![];
        let mut hole_keys = vec![];
        for (wire, edge_keys) in sourced.contours.loops.iter().zip(&sourced.edge_keys) {
            let edges: Vec<_> = wire.edges.iter().map(curved).collect();
            if wire.outer {
                if outer.replace(edges).is_some() {
                    return Err(self.failure("Profile face has more than one outer boundary"));
                }
                keys = edge_keys.clone();
            } else {
                holes.push(edges);
                hole_keys.extend(edge_keys.iter().copied());
            }
        }
        let outer = outer.ok_or_else(|| self.failure("Profile face has no outer boundary"))?;
        keys.extend(hole_keys);
        let origins = self.origins(link.target, &keys);
        Ok((
            CurvedProfile {
                frame: plane,
                outer,
                holes,
            },
            origins,
        ))
    }
    /// Side faces of the new body are named from the source edges' semantic
    /// provenance when that is unique, so they survive rebuild reordering.
    /// Otherwise an explicit boundary-position role is used for every edge.
    fn origins(&self, source: EntityId, keys: &[TopologyKey]) -> Vec<Origin> {
        let snapshot = self.topologies.get(&source);
        let sets: Option<Vec<&BTreeSet<Origin>>> = keys
            .iter()
            .map(|key| {
                snapshot?
                    .elements
                    .iter()
                    .find(|e| e.key == *key)
                    .map(|e| &e.origins)
            })
            .collect();
        // Provenance shared by every boundary edge (such as the selected face
        // itself) cannot distinguish them; use each edge's own neighbour.
        let common: BTreeSet<&Origin> = sets
            .as_ref()
            .and_then(|sets| {
                let mut iter = sets.iter();
                let first: BTreeSet<&Origin> = iter.next()?.iter().collect();
                Some(iter.fold(first, |acc, set| {
                    acc.into_iter().filter(|o| set.contains(*o)).collect()
                }))
            })
            .unwrap_or_default();
        let derived: Option<Vec<Origin>> = sets
            .unwrap_or_default()
            .into_iter()
            .map(|set| {
                let first = set.iter().find(|o| !common.contains(o))?;
                let role = format!("profile.face.{}", first.role);
                (role.len() <= 256).then_some(Origin {
                    source: first.source,
                    role,
                })
            })
            .collect();
        if let Some(derived) = derived
            && derived.len() == keys.len()
            && derived.iter().collect::<BTreeSet<_>>().len() == derived.len()
        {
            return derived;
        }
        (0..keys.len())
            .map(|i| Origin {
                source: self.feature,
                role: format!("profile.face.edge.{i}"),
            })
            .collect()
    }
}
