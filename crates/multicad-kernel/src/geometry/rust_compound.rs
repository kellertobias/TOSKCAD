//! Exact topology for independently placed Rust assembly members.

use super::*;
use multicad_core::references::{TopologyKind, TopologySnapshot};

// Match the core snapshot budget before allocating aggregate topology.
const MAX_TOPOLOGY_ELEMENTS: u32 = 100_000;

struct Member {
    solid: Solid,
    offset: [u32; 3],
    count: [u32; 3],
}

pub(crate) struct RustCompound {
    pub(super) feature: EntityId,
    members: Vec<Member>,
    bounds: Bounds,
    volume_mm3: f64,
    solid_count: u32,
}

fn kind_index(kind: TopologyKind) -> usize {
    match kind {
        TopologyKind::Face => 0,
        TopologyKind::Edge => 1,
        TopologyKind::Vertex => 2,
    }
}

fn compound_error(kind: GeometryErrorKind, message: &str) -> GeometryError {
    error("compound", kind, message)
}

fn empty_bounds() -> Bounds {
    Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    }
}

fn include_bounds(
    bounds: &mut Bounds,
    child: Bounds,
    policy: GeometryPolicy,
) -> Result<(), GeometryError> {
    policy.point(child.min)?;
    policy.point(child.max)?;
    for i in 0..3 {
        bounds.min[i] = bounds.min[i].min(child.min[i]);
        bounds.max[i] = bounds.max[i].max(child.max[i]);
    }
    Ok(())
}

impl RustCompound {
    pub(super) fn assemble(
        solids: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Solid, GeometryError> {
        let compound = Self::build(
            solids.iter().map(|solid| solid.share()),
            feature,
            policy,
            interrupt,
        )?;
        Ok(Solid {
            body: SolidBody::RustCompound(Rc::new(compound)),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    fn build(
        solids: impl Iterator<Item = Result<Solid, GeometryError>>,
        feature: EntityId,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, GeometryError> {
        let checkpoint = || {
            if interrupt.is_some_and(|check| check()) {
                Err(compound_error(
                    GeometryErrorKind::Cancelled,
                    "Compound construction superseded",
                ))
            } else {
                Ok(())
            }
        };
        checkpoint()?;
        let mut members = Vec::new();
        let mut bounds = empty_bounds();
        let mut volume_mm3 = 0.;
        let mut solid_count = 0u32;
        let mut offset = [0u32; 3];
        for solid in solids {
            checkpoint()?;
            let solid = solid?;
            if !solid.uses_rust_geometry() || solid.policy != policy {
                return Err(compound_error(
                    GeometryErrorKind::Input,
                    "Rust compound members must share the Rust backend and geometry policy",
                ));
            }
            let topology = solid.topology_snapshot(feature, 0)?;
            let mut count = [0u32; 3];
            for element in &topology.elements {
                count[kind_index(element.key.kind)] += 1;
            }
            if topology
                .elements
                .iter()
                .any(|element| element.key.ordinal >= count[kind_index(element.key.kind)])
            {
                return Err(compound_error(
                    GeometryErrorKind::InvalidResult,
                    "Rust compound member has noncontiguous topology ordinals",
                ));
            }
            let next = std::array::from_fn(|i| offset[i] + count[i]);
            if next.into_iter().sum::<u32>() > MAX_TOPOLOGY_ELEMENTS {
                return Err(compound_error(
                    GeometryErrorKind::Limit,
                    "Compound topology budget exceeded",
                ));
            }
            include_bounds(&mut bounds, solid.bounds()?, policy)?;
            volume_mm3 += solid.volume_mm3()?;
            solid_count += solid.solid_count()?;
            members.push(Member {
                solid,
                offset,
                count,
            });
            offset = next;
        }
        checkpoint()?;
        if !volume_mm3.is_finite() || volume_mm3 <= 0. {
            return Err(compound_error(
                GeometryErrorKind::InvalidResult,
                "Compound has invalid member volume",
            ));
        }
        Ok(Self {
            feature,
            members,
            bounds,
            volume_mm3,
            solid_count,
        })
    }

    pub(super) fn bounds(&self) -> Bounds {
        self.bounds
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    pub(super) fn solid_count(&self) -> u32 {
        self.solid_count
    }

    pub(crate) fn members(&self) -> impl Iterator<Item = &Solid> {
        self.members.iter().map(|member| &member.solid)
    }

    pub(crate) fn topology(
        &self,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let mut elements = Vec::new();
        for member in &self.members {
            let topology = member.solid.topology_snapshot(owner, revision)?;
            elements.extend(topology.elements.into_iter().map(|mut element| {
                element.key.ordinal += member.offset[kind_index(element.key.kind)];
                element
            }));
        }
        Ok(TopologySnapshot {
            owner,
            revision,
            elements,
        })
    }

    pub(crate) fn face_source(&self, face: u32) -> Option<(&Solid, u32, u32)> {
        let i = self
            .members
            .partition_point(|member| member.offset[0] <= face);
        let member = self.members.get(i.checked_sub(1)?)?;
        let local = face - member.offset[0];
        (local < member.count[0]).then_some((&member.solid, local, member.offset[1]))
    }

    pub(super) fn faces(&self) -> Result<Vec<Face>, GeometryError> {
        let mut faces = Vec::new();
        for member in &self.members {
            faces.extend(member.solid.faces()?.into_iter().map(|mut face| {
                face.index += member.offset[0];
                face
            }));
        }
        Ok(faces)
    }

    pub(super) fn mesh(&self, options: MeshOptions, usage: MeshUse) -> Result<Mesh, GeometryError> {
        let budget_error = || {
            compound_error(
                GeometryErrorKind::Limit,
                "Compound tessellation triangle budget exceeded",
            )
        };
        let mut result = Mesh {
            vertices: vec![],
            indices: vec![],
            face_indices: vec![],
        };
        let mut remaining = options.maximum_triangles;
        for member in &self.members {
            if remaining == 0 {
                return Err(budget_error());
            }
            let mesh = member.solid.rust_mesh(
                MeshOptions {
                    maximum_triangles: remaining,
                    ..options
                },
                usage,
            )?;
            let triangles = u32::try_from(mesh.face_indices.len()).map_err(|_| budget_error())?;
            remaining = remaining.checked_sub(triangles).ok_or_else(budget_error)?;
            let vertices = result.vertices.len() / 3;
            u32::try_from(vertices + mesh.vertices.len() / 3).map_err(|_| budget_error())?;
            let vertex_offset = vertices as u32;
            result.vertices.extend(mesh.vertices);
            result
                .indices
                .extend(mesh.indices.into_iter().map(|index| index + vertex_offset));
            result.face_indices.extend(
                mesh.face_indices
                    .into_iter()
                    .map(|face| face + member.offset[0]),
            );
        }
        Ok(result)
    }

    pub(super) fn transformed(
        &self,
        transform: RigidTransform,
        policy: GeometryPolicy,
    ) -> Result<Rc<Self>, GeometryError> {
        let mut members = Vec::with_capacity(self.members.len());
        let mut bounds = empty_bounds();
        for member in &self.members {
            let solid = member.solid.transformed(transform)?;
            include_bounds(&mut bounds, solid.bounds()?, policy)?;
            // Rigid placement keeps each member's local topology and order.
            members.push(Member {
                solid,
                offset: member.offset,
                count: member.count,
            });
        }
        Ok(Rc::new(Self {
            members,
            bounds,
            feature: self.feature,
            volume_mm3: self.volume_mm3,
            solid_count: self.solid_count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeSet};

    fn stock(policy: GeometryPolicy) -> Solid {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            [4, 5, 6].map(|dimension| Rational::new(dimension, 1).unwrap()),
            PlaneFrame::default(),
            policy,
        )
        .unwrap()
    }

    #[test]
    fn coincident_members_keep_distinct_ordinals_and_share_a_triangle_budget() {
        let policy = GeometryPolicy::default();
        let source = stock(policy);
        let owner = EntityId::new();
        let compound = RustCompound::assemble(&[&source, &source], owner, policy, None).unwrap();
        assert_eq!(compound.solid_count().unwrap(), 2);
        assert_eq!(compound.volume_mm3().unwrap(), 240.);
        assert_eq!(compound.bounds().unwrap(), source.bounds().unwrap());

        let mesh = compound
            .mesh(MeshOptions {
                maximum_triangles: 24,
                ..MeshOptions::default()
            })
            .unwrap();
        assert_eq!(mesh.face_indices.len(), 24);
        let topology = compound.topology_snapshot(owner, 17).unwrap();
        assert_eq!(topology.revision, 17);
        let topology_faces: BTreeSet<_> = topology
            .elements
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Face)
            .map(|element| element.key.ordinal)
            .collect();
        assert_eq!(topology_faces, (0..12).collect());
        assert_eq!(
            mesh.face_indices.into_iter().collect::<BTreeSet<_>>(),
            topology_faces
        );
        let helper = compound.rust_compound().unwrap();
        let (_, local_face, edge_offset) = helper.face_source(6).unwrap();
        assert_eq!((local_face, edge_offset), (0, 12));
        assert!(helper.face_source(12).is_none());
        assert_eq!(
            compound
                .mesh(MeshOptions {
                    maximum_triangles: 12,
                    ..MeshOptions::default()
                })
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Limit
        );
    }

    #[test]
    fn compound_rejects_mismatched_policy_and_can_cancel_between_members() {
        let policy = GeometryPolicy::default();
        let source = stock(policy);
        let other = stock(GeometryPolicy {
            maximum_coordinate_mm: 100.,
            ..policy
        });
        let owner = EntityId::new();
        assert_eq!(
            RustCompound::assemble(&[&source, &other], owner, policy, None)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Input
        );
        let checks = Cell::new(0);
        let stop = || {
            checks.set(checks.get() + 1);
            checks.get() == 3
        };
        assert_eq!(
            RustCompound::assemble(&[&source, &source], owner, policy, Some(&stop))
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Cancelled
        );
    }
}
