//! Boundary-authoritative planar operations. Recovery cells provide occupancy;
//! their partition faces never replace the exposed source-face identities.
use super::*;
use multicad_rust_kernel::planar_boolean::AffineMap;

const MAX_OPERANDS: usize = 129;
const MAX_CELLS: usize = 256;

fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), GeometryError> {
    if interrupt.is_some_and(|interrupt| interrupt()) {
        Err(planar_error(Error::Cancelled))
    } else {
        Ok(())
    }
}

impl Solid {
    pub(in crate::geometry) fn default_certified_boolean(
        &self,
        other: &Self,
        operation: BooleanOperation,
        policy: GeometryPolicy,
    ) -> Option<Result<Option<Self>, GeometryError>> {
        let certified =
            self.selects_general_planar_material() || other.selects_general_planar_material();
        if !certified
            && (operation != BooleanOperation::Intersect
                || [self, other]
                    .into_iter()
                    .all(|solid| solid.orthogonal_cells().is_ok_and(|cells| cells.is_some())))
        {
            return None;
        }
        let topology = match self.rust_topology()? {
            Ok(topology) => topology,
            Err(error) => return Some(Err(error)),
        };
        Self::boolean_planar_certified(&[self, other], operation, topology.owner, policy, None)
    }
    pub(super) fn has_certified_planar_material(&self) -> bool {
        self.planar_boolean_body()
            .is_some_and(|(body, _)| matches!(body.material, Some(Material::Certified(_))))
    }

    fn selects_general_planar_material(&self) -> bool {
        self.has_certified_planar_material()
            || self
                .linear_body()
                .is_some_and(|(body, _)| body.has_plane_terminations())
            || self
                .linear_body()
                .and_then(|(body, _)| body.straight_section())
                .is_some_and(|(rings, _)| rings.len() > 1)
    }

    /// A selected provider returns exact empty material as `Ok(None)`. The outer
    /// `None` leaves evaluation to the existing specialized providers.
    pub fn boolean_planar_certified(
        solids: &[&Self],
        operation: BooleanOperation,
        feature: EntityId,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Option<Result<Option<Self>, GeometryError>> {
        if solids.len() < 2
            || policy.boolean_fuzzy_mm != 0.
            || solids
                .iter()
                .any(|solid| solid.policy != policy || !solid.uses_rust_geometry())
        {
            return None;
        }
        if operation != BooleanOperation::Intersect
            && !solids
                .iter()
                .any(|solid| solid.selects_general_planar_material())
        {
            return None;
        }
        // The existing exact rectilinear path is cheaper for ordinary boxes.
        if !solids
            .iter()
            .any(|solid| solid.selects_general_planar_material())
            && solids
                .iter()
                .all(|solid| solid.orthogonal_cells().is_ok_and(|cells| cells.is_some()))
        {
            return None;
        }
        let result = if let Some((sources, placement)) = common_placement(solids) {
            (|| {
                for source in &sources {
                    placement
                        .frame(PlaneFrame::default())
                        .checked_metric_bounds(source.bounds()?, policy)?;
                }
                certified_boolean(&sources, operation, feature, policy, interrupt)?
                    .map(|solid| solid.transformed(placement))
                    .transpose()
            })()
        } else {
            certified_boolean(solids, operation, feature, policy, interrupt)
        };
        if result
            .as_ref()
            .is_err_and(|error| error.kind == GeometryErrorKind::Unsupported)
        {
            None
        } else {
            Some(result)
        }
    }
}

fn common_placement<'a>(solids: &[&'a Solid]) -> Option<(Vec<&'a Solid>, RigidTransform)> {
    let retained = solids
        .iter()
        .map(|solid| solid.current_retained_placement())
        .collect::<Option<Vec<_>>>()?;
    let placement = retained.first()?.transform;
    if placement == RigidTransform::default() || retained.iter().any(|p| p.transform != placement) {
        return None;
    }
    Some((retained.iter().map(|p| &p.source).collect(), placement))
}

fn certified_boolean(
    solids: &[&Solid],
    operation: BooleanOperation,
    feature: EntityId,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Option<Solid>, GeometryError> {
    checkpoint(interrupt)?;
    if solids.len() > MAX_OPERANDS {
        return Err(planar_error(Error::Limit));
    }
    let mut materials = Vec::with_capacity(solids.len());
    let mut sources = Vec::with_capacity(solids.len());
    let mut topologies = Vec::with_capacity(solids.len());
    let mut cells = 0usize;
    for (operand, solid) in solids.iter().enumerate() {
        checkpoint(interrupt)?;
        let topology = solid.rust_topology().ok_or_else(planar_unsupported)??;
        let (material, faces) = source_material(solid, &topology, policy, interrupt)?;
        cells = cells
            .checked_add(material.pieces().len())
            .filter(|&count| count <= MAX_CELLS)
            .ok_or_else(|| planar_error(Error::Limit))?;
        materials.push(material);
        sources.push(SourcePart {
            operand,
            faces,
            normal_sign: if operation == BooleanOperation::Subtract && operand != 0 {
                -1.
            } else {
                1.
            },
        });
        topologies.push(topology);
    }
    checkpoint(interrupt)?;
    let inputs: Vec<_> = materials.iter().collect();
    let tolerance = policy.linear_tolerance_mm;
    let material = match operation {
        BooleanOperation::Intersect => {
            CutMaterial::intersect_certified_with_interrupt(&inputs, tolerance, interrupt)
        }
        BooleanOperation::Subtract => {
            CutMaterial::subtract_certified_with_interrupt(&inputs, tolerance, interrupt)
        }
        BooleanOperation::Union => {
            CutMaterial::union_certified_with_interrupt(&inputs, tolerance, interrupt).map(Some)
        }
    }
    .map_err(planar_error)?;
    checkpoint(interrupt)?;
    material
        .map(|material| {
            let geometry = material.boundary().clone();
            let result = PlanarBooleanBody::build(
                geometry,
                sources,
                &topologies,
                feature,
                policy,
                Some(Material::Certified(Box::new(material))),
            )?;
            checkpoint(interrupt)?;
            Ok(result)
        })
        .transpose()
}

fn source_material(
    solid: &Solid,
    topology: &TopologySnapshot,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(CutMaterial, Vec<Option<usize>>), GeometryError> {
    let tolerance = policy.linear_tolerance_mm;
    if let Some((body, frame)) = solid.planar_boolean_body() {
        let faces = face_indices(topology, body.geometry.faces().len())?;
        let certified = match body.material.as_ref() {
            Some(Material::Certified(material)) => Some((**material).clone()),
            Some(Material::Cut(recipe)) => {
                Some(recipe.controlled_material(&body.geometry, tolerance, interrupt)?)
            }
            _ => None,
        };
        if frame == PlaneFrame::default()
            && let Some(material) = certified.as_ref()
        {
            return Ok((material.clone(), faces));
        }
        let geometry = if frame == PlaneFrame::default() {
            body.geometry.clone()
        } else {
            let map = AffineMap::new([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [0.; 3])
                .and_then(|map| map.with_rigid_input(frame.origin, frame.u, frame.v))
                .map_err(planar_error)?;
            body.geometry
                .affine_transformed(map, tolerance)
                .map_err(planar_error)?
        };
        let pieces = match certified.as_ref() {
            Some(material) => {
                PlanarBooleanBody::placed_pieces(material.pieces(), frame, tolerance)?
            }
            None => body.material_pieces(frame, tolerance)?,
        };
        checkpoint(interrupt)?;
        let material =
            CutMaterial::certify_existing_with_interrupt(&geometry, &pieces, tolerance, interrupt)
                .map_err(planar_error)?;
        return Ok((material, faces));
    }
    let parts = solid
        .planar_convex_parts(topology, policy)
        .ok_or_else(planar_unsupported)??;
    if parts.len() > MAX_CELLS {
        return Err(planar_error(Error::Limit));
    }
    let pieces: Vec<_> = parts.iter().map(|(piece, _)| piece.clone()).collect();
    checkpoint(interrupt)?;
    let geometry =
        Union::new_with_interrupt(&pieces, tolerance, interrupt).map_err(planar_error)?;
    let faces = geometry
        .faces()
        .iter()
        .map(|face| {
            parts[face.source]
                .1
                .get(face.source_face)
                .copied()
                .flatten()
        })
        .collect();
    let material =
        CutMaterial::certify_existing_with_interrupt(&geometry, &pieces, tolerance, interrupt)
            .map_err(planar_error)?;
    Ok((material, faces))
}

impl CutRecipe {
    fn controlled_material(
        &self,
        boundary: &Union,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<CutMaterial, GeometryError> {
        checkpoint(interrupt)?;
        if let Some(material) = self.certified.get() {
            return material.clone().map_err(planar_error);
        }
        let material = CutMaterial::recover_with_interrupt(
            boundary,
            &self.stock,
            &self.tools,
            tolerance,
            interrupt,
        )
        .map_err(planar_error)?;
        checkpoint(interrupt)?;
        // A cancelled or otherwise failed request never poisons a later retry.
        let _ = self.certified.set(Ok(material.clone()));
        Ok(material)
    }
}

fn face_indices(
    topology: &TopologySnapshot,
    count: usize,
) -> Result<Vec<Option<usize>>, GeometryError> {
    let mut faces = vec![None; count];
    for (index, element) in topology
        .elements
        .iter()
        .enumerate()
        .filter(|(_, element)| element.key.kind == TopologyKind::Face)
    {
        let face = faces
            .get_mut(element.key.ordinal as usize)
            .ok_or_else(|| planar_error(Error::Invalid))?;
        if face.replace(index).is_some() {
            return Err(planar_error(Error::Invalid));
        }
    }
    if faces.iter().any(Option::is_none) {
        return Err(planar_error(Error::Invalid));
    }
    Ok(faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn retained_placement_reuse_requires_current_geometry_chart_and_metadata() {
        let policy = GeometryPolicy::default();
        let stock = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [10, 8, 6].map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame::default(),
            policy,
        )
        .unwrap();
        let pose = RigidTransform {
            rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
            translation_mm: [90., -40., 23.],
        };
        let moved = stock.transformed(pose).unwrap();
        assert!(moved.current_retained_placement().is_some());
        let shared = moved.share().unwrap();
        assert!(shared.current_retained_placement().is_some());
        assert!(common_placement(&[&moved, &shared]).is_some());
        assert!(common_placement(&[&stock, &shared]).is_none());
        let different = stock
            .transformed(RigidTransform {
                translation_mm: [91., -40., 23.],
                ..pose
            })
            .unwrap();
        assert!(common_placement(&[&moved, &different]).is_none());
        let mut replaced = moved.share().unwrap();
        let replacement = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [2, 3, 4].map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame::default(),
            policy,
        )
        .unwrap()
        .transformed(pose)
        .unwrap();
        replaced.body = replacement.body;
        assert!(replaced.current_retained_placement().is_none());
        let (local, _) = moved.local_box_treatment_source().unwrap().unwrap();
        assert!(local.current_retained_placement().is_none());
        let tagged = shared
            .with_component_origin(EntityId::new(), EntityId::new())
            .unwrap();
        assert!(tagged.current_retained_placement().is_none());
        assert!(moved.current_retained_placement().is_some());
        // A second placement retains its immediate source without growing a
        // recursive chain. Sharing never changes either source's geometry.
        let twice = moved.transformed(pose).unwrap();
        let retained = twice.current_retained_placement().unwrap();
        assert!(retained.source.retained_placement.is_none());
        near_volume(&retained.source, 480.);
        near_volume(&moved, 480.);
    }

    fn near_volume(solid: &Solid, expected: f64) {
        assert!((solid.volume_mm3().unwrap() - expected).abs() < 1e-7);
    }

    #[test]
    fn cancelled_cold_cut_recovery_is_retryable_in_local_and_posed_source_charts() {
        for translation_mm in [[0.; 3], [11., -7., 23.]] {
            let policy = GeometryPolicy::default();
            let [a, b, c, d] = [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
            let cell = ConvexPiece::new(
                vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]],
                policy.linear_tolerance_mm,
            )
            .unwrap();
            let geometry = Union::new(&[cell], policy.linear_tolerance_mm).unwrap();
            let stock =
                Solid::from_imported_planar_boundary(geometry, EntityId::new(), policy).unwrap();
            let notch = Solid::rust_box(
                EntityId::new().uuid_bits(),
                [4, 1, 1].map(|n| Rational::new(n, 1).unwrap()),
                PlaneFrame {
                    origin: [1., 0., 0.],
                    ..Default::default()
                },
                policy,
            )
            .unwrap();
            let cut = stock
                .rust_subtract_planar_tools(&[&notch], EntityId::new(), policy)
                .unwrap()
                .unwrap();
            let pose = RigidTransform {
                translation_mm,
                ..Default::default()
            };
            let source = cut.transformed(pose).unwrap();
            let envelope = stock.transformed(pose).unwrap();
            let (body, _) = source.planar_boolean_body().unwrap();
            let Some(Material::Cut(recipe)) = &body.material else {
                panic!("expected cold ordinary cut recipe")
            };
            assert!(recipe.certified.get().is_none());
            let calls = Cell::new(0);
            let interrupt = || {
                let next = calls.get() + 1;
                calls.set(next);
                next > 6
            };
            let error = Solid::boolean_planar_certified(
                &[&source, &envelope],
                BooleanOperation::Intersect,
                EntityId::new(),
                policy,
                Some(&interrupt),
            )
            .unwrap()
            .err()
            .unwrap();
            assert_eq!(error.kind, GeometryErrorKind::Cancelled);
            assert!(
                recipe.certified.get().is_none(),
                "cancelled proof poisoned a later retry"
            );
            let result = Solid::boolean_planar_certified(
                &[&source, &envelope],
                BooleanOperation::Intersect,
                EntityId::new(),
                policy,
                None,
            )
            .unwrap()
            .unwrap()
            .unwrap();
            assert!((result.volume_mm3().unwrap() - 26. / 3.).abs() < 1e-7);
            assert!(recipe.certified.get().is_some_and(Result::is_ok));
            assert!((source.volume_mm3().unwrap() - 26. / 3.).abs() < 1e-7);
        }
    }
}
