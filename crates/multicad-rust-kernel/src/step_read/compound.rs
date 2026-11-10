//! Verify membership in a flat STEP advanced BREP representation.

use super::*;

pub(super) fn read_members<T>(
    input: &str,
    tolerance: f64,
    allow_circles: bool,
    single: bool,
    member: impl Fn(&Shell<'_, '_>, &[Face]) -> Result<T>,
) -> Result<Vec<T>> {
    read_members_by(input, tolerance, allow_circles, single, |shell, id, _| {
        let faces = shell.validate_faces(shell.member_faces(id)?)?;
        member(shell, &faces)
    })
}

pub(super) fn read_members_with_warped<T>(
    input: &str,
    tolerance: f64,
    single: bool,
    member: impl Fn(&Shell<'_, '_>, &[Face]) -> Result<T>,
    warped_member: impl Fn(Readback, ImportedWarped) -> T,
) -> Result<Vec<T>> {
    read_members_by(input, tolerance, true, single, |shell, id, has_ellipses| {
        let regular = shell
            .member_faces(id)
            .and_then(|face_ids| shell.validate_faces(face_ids))
            .and_then(|faces| member(shell, &faces));
        match regular {
            Err(ReadError::Invalid | ReadError::Unsupported) if has_ellipses => {
                // The independent verifier accepts only its own complete
                // chart class, and does not reuse a partially parsed shell.
                let mut clean = Shell::new(shell.entities, shell.tolerance, true);
                match warped::read_member(&mut clean, id) {
                    Ok((readback, import)) => Ok(warped_member(readback, import)),
                    Err(ReadError::Limit) => Err(ReadError::Limit),
                    Err(_) => regular,
                }
            }
            _ => regular,
        }
    })
}

pub(super) fn read_members_by<T>(
    input: &str,
    tolerance: f64,
    allow_circles: bool,
    single: bool,
    member: impl Fn(&mut Shell<'_, '_>, u32, bool) -> Result<T>,
) -> Result<Vec<T>> {
    let entities = parse_entities(input, tolerance)?;
    let solids = Shell::new(&entities, tolerance, allow_circles).represented_solids(single)?;
    let topology_count = count_flat_topology(entities.values().flatten())?;
    let has_ellipses = allow_circles
        && entities
            .values()
            .any(|items| matches!(items.as_slice(), [item] if item.name == "ELLIPSE"));
    let mut owners = BTreeMap::new();
    let members = solids
        .into_iter()
        .map(|id| {
            let mut shell = Shell::new(&entities, tolerance, allow_circles);
            let result = member(&mut shell, id, has_ellipses)?;
            claim_topology(&shell, id, &mut owners)?;
            Ok(result)
        })
        .collect::<Result<Vec<_>>>()?;
    if owners.len() != topology_count {
        return Err(ReadError::Invalid);
    }
    Ok(members)
}

fn count_flat_topology<'a>(entities: impl Iterator<Item = &'a Entity<'a>>) -> Result<usize> {
    let mut count = 0;
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
    }
    Ok(count)
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
        let entity = shell.simple(id)?;
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
