//! Verify membership in a flat STEP advanced BREP representation.

use super::*;

pub(super) fn read_import_members(
    input: &str,
    tolerance: f64,
    single: bool,
) -> Result<Vec<(Readback, ImportSolid)>> {
    read_members_by(
        input,
        tolerance,
        true,
        single,
        |shell, id, may_be_warped| {
            if let Some(member) = box_void::read_member(shell, id)? {
                return Ok(member);
            }
            let face_ids = shell.member_faces(id)?;
            if let Some(sphere) = sphere::read_member(shell, &face_ids)? {
                return Ok(sphere);
            }
            if let Some(torus) = torus::read_member(shell, &face_ids)? {
                return Ok(torus);
            }
            if let Some(cut) = import::read_helical_cut_member(shell, &face_ids)? {
                return Ok(cut);
            }
            if let Some((proof, loft)) = nonplanar_loft::read_member(shell, &face_ids)? {
                return Ok(import::recognize_nonplanar_loft(proof, loft));
            }
            if face_ids
                .iter()
                .any(|&face| spline_sweep::has_extrusion_support(shell, face))
            {
                return spline_sweep::import_member(shell, face_ids)
                    .map(|(proof, body)| import::recognize_spline(proof, body));
            }
            let regular = shell
                .validate_faces(face_ids)
                .and_then(|faces| import::recognize_member(shell, &faces));
            match regular {
                Err(error @ (ReadError::Invalid | ReadError::Unsupported)) if may_be_warped => {
                    fallback_warped(shell, id, error)
                        .map(|(readback, import)| import::recognize_warped(readback, import))
                }
                _ => regular,
            }
        },
    )
}

pub(super) fn read_analytic(input: &str, tolerance: f64, single: bool) -> Result<Vec<Readback>> {
    read_analytic_with_helical_cuts(input, tolerance, single)
        .map(|members| members.into_iter().map(|(proof, _)| proof).collect())
}

pub(super) fn read_analytic_with_helical_cuts(
    input: &str,
    tolerance: f64,
    single: bool,
) -> Result<Vec<(Readback, Option<ImportSolid>)>> {
    read_members_by(input, tolerance, true, single, read_analytic_member)
}

pub(super) fn read_analytic_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
    may_be_warped: bool,
) -> Result<(Readback, Option<ImportSolid>)> {
    if let Some((readback, _)) = box_void::read_member(shell, id)? {
        return Ok((readback, None));
    }
    let faces = shell.member_faces(id)?;
    if let Some((proof, _)) = sphere::read_member(shell, &faces)? {
        return Ok((proof, None));
    }
    if let Some((proof, _)) = torus::read_member(shell, &faces)? {
        return Ok((proof, None));
    }
    if let Some((proof, cut)) = import::read_helical_cut_member(shell, &faces)? {
        return Ok((proof, Some(cut)));
    }
    if let Some((proof, _)) = nonplanar_loft::read_member(shell, &faces)? {
        return Ok((proof, None));
    }
    if faces
        .iter()
        .any(|&face| spline_sweep::has_extrusion_support(shell, face))
    {
        return spline_sweep::read_member(shell, faces).map(|proof| (proof, None));
    }
    let regular = shell
        .validate_faces(faces)
        .and_then(|faces| shell.readback(&faces));
    match regular {
        Err(error @ (ReadError::Invalid | ReadError::Unsupported)) if may_be_warped => {
            fallback_warped(shell, id, error).map(|(proof, _)| (proof, None))
        }
        _ => regular.map(|proof| (proof, None)),
    }
}

fn fallback_warped(
    shell: &Shell<'_, '_>,
    id: u32,
    error: ReadError,
) -> Result<(Readback, ImportedWarped)> {
    // The independent verifier does not reuse a partially parsed shell.
    let mut clean = Shell::new(shell.entities, shell.tolerance, true);
    match warped::read_member(&mut clean, id) {
        Ok(result) => Ok(result),
        Err(ReadError::Limit) => Err(ReadError::Limit),
        Err(_) => Err(error),
    }
}

pub(super) fn read_members_by<T>(
    input: &str,
    tolerance: f64,
    allow_circles: bool,
    single: bool,
    mut member: impl FnMut(&mut Shell<'_, '_>, u32, bool) -> Result<T>,
) -> Result<Vec<T>> {
    let entities = parse_entities(input, tolerance)?;
    let solids = Shell::new(&entities, tolerance, allow_circles).represented_solids(single)?;
    let (topology_count, analytic_candidate) = inspect_flat_entities(entities.values().flatten())?;
    let may_be_warped = allow_circles && analytic_candidate;
    let mut owners = BTreeMap::new();
    let members = solids
        .into_iter()
        .map(|id| {
            let mut shell = Shell::new(&entities, tolerance, allow_circles);
            let result = member(&mut shell, id, may_be_warped)?;
            claim_topology(&shell, id, &mut owners)?;
            Ok(result)
        })
        .collect::<Result<Vec<_>>>()?;
    if owners.len() != topology_count {
        return Err(ReadError::Invalid);
    }
    Ok(members)
}

fn inspect_flat_entities<'a>(
    entities: impl Iterator<Item = &'a Entity<'a>>,
) -> Result<(usize, bool)> {
    let mut count = 0;
    let mut ellipse = false;
    let mut cylinder = false;
    for entity in entities {
        if matches!(
            entity.name,
            "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"
                | "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION"
                | "NEXT_ASSEMBLY_USAGE_OCCURRENCE"
                | "MAPPED_ITEM"
        ) {
            return Err(ReadError::Unsupported);
        }
        count += usize::from(topology(entity.name));
        ellipse |= entity.name == "ELLIPSE";
        cylinder |= entity.name == "CYLINDRICAL_SURFACE";
    }
    Ok((count, ellipse || cylinder))
}

fn topology(name: &str) -> bool {
    matches!(
        name,
        "MANIFOLD_SOLID_BREP"
            | "CLOSED_SHELL"
            | "ADVANCED_FACE"
            | "FACE_OUTER_BOUND"
            | "FACE_BOUND"
            | "EDGE_LOOP"
            | "ORIENTED_EDGE"
            | "EDGE_CURVE"
            | "VERTEX_POINT"
            | "BREP_WITH_VOIDS"
            | "OPEN_SHELL"
            | "ORIENTED_CLOSED_SHELL"
            | "FACE_SURFACE"
            | "VERTEX_LOOP"
            | "SHELL_BASED_SURFACE_MODEL"
    )
}

fn claim_topology(
    shell: &Shell<'_, '_>,
    solid: u32,
    owners: &mut BTreeMap<u32, u32>,
) -> Result<()> {
    use std::collections::btree_map::Entry;

    let mut pending = vec![solid];
    while let Some(id) = pending.pop() {
        let parts = shell.components(id)?;
        let [entity] = parts else {
            // A verified rational support is a complex geometric entity.
            // Topology remains restricted to single, exclusively owned entities.
            if parts.iter().any(|part| topology(part.name)) {
                return Err(ReadError::Unsupported);
            }
            continue;
        };
        if !topology(entity.name) {
            continue;
        }
        match owners.entry(id) {
            Entry::Occupied(owner) if *owner.get() == solid => continue,
            Entry::Occupied(_) => return Err(ReadError::Invalid),
            Entry::Vacant(owner) => owner.insert(solid),
        };
        // The member shell has already checked every topological reference.
        // Only topology has an exclusive owner; geometric supports can be shared.
        for arg in &entity.args {
            match arg {
                Value::Reference(id) => pending.push(*id),
                Value::List(items) => {
                    pending.extend(items.iter().filter_map(|item| reference(item).ok()));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
