//! Resolve associative end planes against current upstream native geometry.
use crate::{ModelError, compiler::error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{ResolvedExtent, ResolvedExtrudeControls, ResolvedTermination},
    references::{self, Origin, Resolution, TopologySnapshot},
    sketch,
};
use multicad_kernel::{BooleanOperation, CurvedProfile, GeometryPolicy, PlaneFrame, Solid};
use std::collections::BTreeMap;

pub(crate) struct Extrusion<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub feature: EntityId,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
    pub interrupt: Option<&'a dyn Fn() -> bool>,
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
impl Extrusion<'_> {
    fn failure(&self, message: impl std::fmt::Display) -> ModelError {
        error(Some(self.feature), message)
    }
    fn plane(
        &self,
        end: &ResolvedTermination,
        profile: &CurvedProfile,
    ) -> Result<PlaneFrame, ModelError> {
        let report = references::inspect(self.document, self.revision, self.topologies);
        let (mut frame, offset) = match end {
            ResolvedTermination::ToPlane { plane, offset_mm } => {
                let frame =
                    sketch::inspect_plane(self.document, *plane, self.revision, Some(&report))
                        .map_err(|e| self.failure(e))?;
                (
                    PlaneFrame {
                        origin: frame.origin,
                        u: frame.u,
                        v: frame.v,
                    },
                    *offset_mm,
                )
            }
            ResolvedTermination::ToFace {
                reference,
                offset_mm,
            } => {
                let Some(EntityData::Feature { inputs, .. }) =
                    self.document.get(self.feature).map(|e| &e.data)
                else {
                    return Err(self.failure("Missing extrusion feature"));
                };
                let link = inputs
                    .iter()
                    .find(|link| {
                        link.geometry
                            .as_ref()
                            .is_some_and(|intent| intent.id == *reference)
                    })
                    .ok_or_else(|| {
                        self.failure("Termination face input is missing; repair its reference")
                    })?;
                let diagnostic = report
                    .get(references::ReferenceAddress {
                        owner: self.feature,
                        reference: *reference,
                    })
                    .ok_or_else(|| {
                        self.failure("Termination face has no current reference evaluation")
                    })?;
                let Resolution::Resolved { key, frame, .. } = diagnostic.resolution else {
                    return Err(self.failure(format!(
                        "Termination face is unresolved: {:?}",
                        diagnostic.resolution
                    )));
                };
                let solid = self.solids.get(&link.target).ok_or_else(|| {
                    self.failure("Termination face producer has no current solid")
                })?;
                let face = solid
                    .faces()
                    .map_err(|e| self.failure(e))?
                    .into_iter()
                    .find(|face| face.index == key.ordinal)
                    .ok_or_else(|| {
                        self.failure("Termination face is missing from the current solid")
                    })?;
                let (origin, normal) = face.plane.ok_or_else(|| {
                    self.failure("Select a planar termination face; this face is curved")
                })?;
                // Signed face offsets follow the current native OUTWARD
                // normal. Multiple inherited semantic frames must never choose
                // the offset side through UUID/insertion ordering.
                let target = if let Some(frame) = frame {
                    frame.validate().map_err(|e| self.failure(e))?;
                    if dot(frame.normal(), normal).abs() < 1. - 1e-8
                        || dot(
                            std::array::from_fn(|i| frame.origin_mm[i] - origin[i]),
                            normal,
                        )
                        .abs()
                            > self.policy.linear_tolerance_mm * 10.
                    {
                        return Err(self.failure("Termination attachment is not on its selected native face; use the explicit target-plane offset"));
                    }
                    PlaneFrame {
                        origin: frame.origin_mm,
                        u: frame.u,
                        v: if dot(frame.normal(), normal) < 0. {
                            frame.v.map(|n| -n)
                        } else {
                            frame.v
                        },
                    }
                } else {
                    // This is the current native support plane of the resolved
                    // face, not a saved datum or nearest-face replacement. Roll
                    // follows the profile basis without an explicit attachment.
                    let mut u = std::array::from_fn(|i| {
                        profile.frame.u[i] - normal[i] * dot(profile.frame.u, normal)
                    });
                    let mut length = dot(u, u).sqrt();
                    if length < 1e-8 {
                        u = std::array::from_fn(|i| {
                            profile.frame.v[i] - normal[i] * dot(profile.frame.v, normal)
                        });
                        length = dot(u, u).sqrt();
                    }
                    if length < 1e-8 {
                        return Err(
                            self.failure("Cannot construct the selected face's support frame")
                        );
                    }
                    let u = u.map(|n| n / length);
                    let height = dot(
                        std::array::from_fn(|i| profile.frame.origin[i] - origin[i]),
                        normal,
                    );
                    PlaneFrame {
                        // This deterministic projection also avoids adopting an
                        // arbitrary native surface-parameter origin as intent.
                        origin: std::array::from_fn(|i| {
                            profile.frame.origin[i] - normal[i] * height
                        }),
                        u,
                        v: cross(normal, u),
                    }
                };
                (target, *offset_mm)
            }
            ResolvedTermination::Distance { .. } => {
                return Err(self.failure("A distance end condition has no target plane"));
            }
        };
        let normal = frame.normal();
        frame.origin = std::array::from_fn(|i| frame.origin[i] + normal[i] * offset);
        Ok(frame)
    }
    fn side(
        &self,
        profile: &CurvedProfile,
        direction: [f64; 3],
        end: &ResolvedTermination,
        origins: &[Origin],
        taper_rad: f64,
    ) -> Result<Solid, ModelError> {
        if self.interrupt.is_some_and(|interrupt| interrupt()) {
            return Err(self.failure("Model evaluation superseded"));
        }
        match end {
            ResolvedTermination::Distance { distance_mm } => {
                Solid::extrude_curved_profile_vector_tapered_named(
                    profile,
                    direction.map(|n| n * distance_mm),
                    taper_rad,
                    self.feature,
                    origins,
                    self.policy,
                )
            }
            _ if taper_rad!=0. => return Err(self.failure("Taper with an associative end plane is not supported yet")),
            _ => Solid::extrude_curved_profile_to_plane_named(
                profile,
                direction,
                self.plane(end, profile)?,
                self.feature,
                origins,
                self.policy,
            ),
        }
        .map_err(|e| self.failure(e))
    }
    fn halves(
        &self,
        profile: &CurvedProfile,
        direction: [f64; 3],
        ends: [&ResolvedTermination; 2],
        origins: &[Origin],
        taper_rad: f64,
    ) -> Result<Solid, ModelError> {
        let forward = self.side(profile, direction, ends[0], origins, taper_rad)?;
        let backward = self
            .side(profile, direction.map(|n| -n), ends[1], origins, taper_rad)?
            .with_feature_cap_roles(self.feature, "cap.end", "cap.start")
            .map_err(|e| self.failure(e))?;
        if self.interrupt.is_some_and(|interrupt| interrupt()) {
            return Err(self.failure("Model evaluation superseded"));
        }
        let solid = forward
            .boolean(&backward, BooleanOperation::Union, self.policy)
            .map_err(|e| self.failure(e))?
            .ok_or_else(|| self.failure("Two-sided extrusion produced no solid"))?;
        if solid.solid_count().map_err(|e| self.failure(e))? != 1 {
            return Err(self.failure("Two-sided extrusion did not form one connected body"));
        }
        Ok(solid)
    }
    pub fn build(
        &self,
        mut profile: CurvedProfile,
        origins: &[Origin],
        controls: &ResolvedExtrudeControls,
    ) -> Result<Solid, ModelError> {
        let frame = profile.frame;
        let normal = frame.normal();
        let direction: [f64; 3] = std::array::from_fn(|i| {
            frame.u[i] * controls.direction[0]
                + frame.v[i] * controls.direction[1]
                + normal[i] * controls.direction[2]
        });
        profile.frame.origin =
            std::array::from_fn(|i| frame.origin[i] + normal[i] * controls.start_offset_mm);
        if controls.taper_rad != 0. {
            let referenced = match &controls.extent {
                ResolvedExtent::OneSided { end } => {
                    !matches!(end, ResolvedTermination::Distance { .. })
                }
                ResolvedExtent::TwoSided { forward, backward } => {
                    !matches!(forward, ResolvedTermination::Distance { .. })
                        || !matches!(backward, ResolvedTermination::Distance { .. })
                }
                ResolvedExtent::Symmetric { .. } => false,
            };
            if referenced {
                return Err(
                    self.failure("Taper with an associative end plane is not supported yet")
                );
            }
        }
        match &controls.extent {
            ResolvedExtent::OneSided { end } => {
                self.side(&profile, direction, end, origins, controls.taper_rad)
            }
            ResolvedExtent::Symmetric { total_mm } => {
                if controls.taper_rad != 0. {
                    let half = ResolvedTermination::Distance {
                        distance_mm: total_mm / 2.,
                    };
                    return self.halves(
                        &profile,
                        direction,
                        [&half, &half],
                        origins,
                        controls.taper_rad,
                    );
                }
                profile.frame.origin =
                    std::array::from_fn(|i| profile.frame.origin[i] - direction[i] * total_mm / 2.);
                Solid::extrude_curved_profile_vector_named(
                    &profile,
                    direction.map(|n| n * total_mm),
                    self.feature,
                    origins,
                    self.policy,
                )
                .map_err(|e| self.failure(e))
            }
            ResolvedExtent::TwoSided {
                forward:
                    ResolvedTermination::Distance {
                        distance_mm: forward,
                    },
                backward:
                    ResolvedTermination::Distance {
                        distance_mm: backward,
                    },
            } if controls.taper_rad == 0. => {
                profile.frame.origin =
                    std::array::from_fn(|i| profile.frame.origin[i] - direction[i] * backward);
                Solid::extrude_curved_profile_vector_named(
                    &profile,
                    direction.map(|n| n * (forward + backward)),
                    self.feature,
                    origins,
                    self.policy,
                )
                .map_err(|e| self.failure(e))
            }
            ResolvedExtent::TwoSided { forward, backward } => self.halves(
                &profile,
                direction,
                [forward, backward],
                origins,
                controls.taper_rad,
            ),
        }
    }
}
