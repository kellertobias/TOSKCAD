//! Empty material proofs in an exact retained common construction chart.
use super::*;

fn planar_leaf(solid: &Solid) -> bool {
    if let Some((body, _)) = solid.orthogonal_body() {
        return !body.has_curved_cuts();
    }
    if let Some(body) = solid.polygon_prism() {
        return body.pockets.is_empty() && body.prism.circular_holes().is_empty();
    }
    if let Some((prism, _)) = solid.rust_prism_frame() {
        return prism.rounded_end().is_none() && prism.inner_loops().len() == 0;
    }
    matches!(
        solid.body,
        SolidBody::Linear(..) | SolidBody::PlanarBoolean(..)
    )
}

fn leaves(
    solid: &Solid,
    depth: usize,
    result: &mut Vec<(Solid, RigidTransform)>,
) -> Result<bool, GeometryError> {
    if depth > 64 || result.len() >= 256 {
        return Ok(false);
    }
    if let Some(compound) = solid.rust_compound() {
        for member in compound.members() {
            if !leaves(member, depth + 1, result)? {
                return Ok(false);
            }
        }
    } else if !planar_leaf(solid) {
        return Ok(false);
    } else if let Some(local) = solid.local_box_treatment_source()? {
        result.push(local);
    } else if matches!(
        solid.body,
        SolidBody::Prism(..)
            | SolidBody::PolygonPrism(..)
            | SolidBody::Linear(..)
            | SolidBody::Orthogonal(..)
            | SolidBody::PlanarBoolean(..)
    ) {
        result.push((solid.share()?, RigidTransform::default()));
    } else {
        return Ok(false);
    }
    Ok(true)
}
impl Solid {
    pub(crate) fn has_common_placed_planar_query_chart(
        &self,
        other: &Self,
    ) -> Result<bool, GeometryError> {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        if !leaves(self, 0, &mut left)? || !leaves(other, 0, &mut right)? {
            return Ok(false);
        }
        let Some((_, pose)) = left.first() else {
            return Ok(false);
        };
        Ok(*pose != RigidTransform::default()
            && left.iter().chain(&right).all(|(_, other)| other == pose))
    }
    /// None leaves the ordinary provider in charge. Some(true) proves no
    /// occupied overlap; Some(false) prevents a later rounded-world zero from
    /// erasing proved positive source material. No source coordinates move.
    pub(super) fn intrinsic_planar_overlap_empty(
        &self,
        other: &Self,
        policy: GeometryPolicy,
    ) -> Result<Option<bool>, GeometryError> {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        if !leaves(self, 0, &mut left)? || !leaves(other, 0, &mut right)? {
            return Ok(None);
        }
        let Some((_, pose)) = left.first() else {
            return Ok(None);
        };
        if left.iter().chain(&right).any(|(_, other)| other != pose) {
            return Ok(None);
        }
        if *pose == RigidTransform::default() {
            return Ok(None);
        }
        let bounds_left = left
            .iter()
            .map(|(s, _)| s.bounds())
            .collect::<Result<Vec<_>, _>>()?;
        let bounds_right = right
            .iter()
            .map(|(s, _)| s.bounds())
            .collect::<Result<Vec<_>, _>>()?;
        if bounds_left.iter().all(|a| {
            bounds_right
                .iter()
                .all(|b| (0..3).any(|i| a.max[i] <= b.min[i] || b.max[i] <= a.min[i]))
        }) {
            return Ok(Some(true));
        }
        let (mut lhs, mut rhs) = (Vec::new(), Vec::new());
        for (member, _) in &left {
            member.append_planar_material(&mut lhs, policy)?;
        }
        for (member, _) in &right {
            member.append_planar_material(&mut rhs, policy)?;
        }
        Ok(Some(
            Union::intersect(&lhs, &rhs, policy.linear_tolerance_mm)
                .map_err(planar_error)?
                .is_none(),
        ))
    }
}
