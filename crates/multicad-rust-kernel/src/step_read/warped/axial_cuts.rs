//! Recover axial removals from their boundary incidence, then replay them on
//! the verified chart and intact bosses. No STEP metadata supplies a recipe.
use super::*;
use crate::warped_union::{AxialCut, ConvexRecess};

mod recess;
mod verify;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Use {
    face: usize,
    hole: Option<usize>,
}

struct Incidence(BTreeMap<u32, Vec<Use>>);

impl Incidence {
    fn new(source: &Boundary) -> Self {
        let mut edges = BTreeMap::<_, Vec<_>>::new();
        for (face, boundary) in source.faces.iter().enumerate() {
            let wires = std::iter::once((None, &boundary.loop_)).chain(
                boundary
                    .holes
                    .iter()
                    .enumerate()
                    .map(|(i, wire)| (Some(i), wire)),
            );
            for (hole, wire) in wires {
                for &(edge, _) in wire {
                    edges.entry(edge).or_default().push(Use { face, hole });
                }
            }
        }
        Self(edges)
    }

    fn adjacent(&self, edge: u32, face: usize) -> Result<Use> {
        let [a, b] = self.0[&edge].as_slice() else {
            return Err(ReadError::Invalid);
        };
        match (a.face == face, b.face == face) {
            (true, false) => Ok(*b),
            (false, true) => Ok(*a),
            _ => Err(ReadError::Invalid),
        }
    }
}

#[derive(Clone, Copy)]
struct Patch {
    origin: Point,
    axis: Point,
    radius: f64,
    rims: [(u32, EllipseArc); 2],
    rails: [u32; 2],
}

impl Patch {
    fn read(source: &Boundary, face: &FaceLoop, tol: f64) -> Option<Self> {
        let Surface::Cylinder {
            origin,
            axis,
            radius,
            outward: false,
        } = face.surface
        else {
            return None;
        };
        if face.loop_.len() != 4 || !face.holes.is_empty() {
            return None;
        }
        let mut rims = Vec::with_capacity(2);
        let mut rails = Vec::with_capacity(2);
        for &(edge, _) in &face.loop_ {
            match source.curves[&edge].1 {
                TrimCurve::Line => rails.push(edge),
                TrimCurve::Arc(arc) => {
                    let ellipse = arc.ellipse();
                    if ellipse
                        .semiaxes_mm()
                        .into_iter()
                        .any(|r| (r - radius).abs() > tol)
                        || length(cross(ellipse.normal(), axis)) > 1e-10
                        || length(cross(sub(ellipse.center_mm(), origin), axis)) > tol
                        || (arc.sweep_rad().abs() - FRAC_PI_2).abs() * radius > tol
                    {
                        return None;
                    }
                    rims.push((edge, arc));
                }
            }
        }
        Some(Self {
            origin,
            axis,
            radius,
            rims: rims.try_into().ok()?,
            rails: rails.try_into().ok()?,
        })
    }

    fn same_support(self, other: Self, tol: f64) -> bool {
        length(cross(self.axis, other.axis)) <= 1e-10
            && length(cross(sub(self.origin, other.origin), self.axis)) <= tol
            && (self.radius - other.radius).abs() <= tol
    }
}

#[derive(Clone, Copy)]
struct End {
    center: Point,
    radial: Point,
    at: Use,
}

struct Removal {
    ends: [End; 2],
    axis: Point,
    radius: f64,
    remove: BTreeSet<usize>,
}

impl Removal {
    fn read(
        first: usize,
        basis: Patch,
        patches: &[Option<Patch>],
        incidence: &Incidence,
        visited: &mut BTreeSet<usize>,
        tol: f64,
    ) -> Result<Option<Self>> {
        let mut stack = vec![first];
        let mut component = BTreeSet::new();
        while let Some(face) = stack.pop() {
            if !component.insert(face) {
                continue;
            }
            visited.insert(face);
            if component.len() > 4 {
                return Err(ReadError::Unsupported);
            }
            for rail in patches[face].ok_or(ReadError::Invalid)?.rails {
                let neighbor = incidence.adjacent(rail, face)?.face;
                if patches[neighbor].is_some_and(|other| basis.same_support(other, tol)) {
                    stack.push(neighbor);
                }
            }
        }
        if component.len() != 4 {
            return Ok(None);
        }
        let mut rings: [Vec<End>; 2] = std::array::from_fn(|_| Vec::with_capacity(4));
        for &face in &component {
            let patch = patches[face].ok_or(ReadError::Invalid)?;
            for rail in patch.rails {
                if !component.contains(&incidence.adjacent(rail, face)?.face) {
                    return Err(ReadError::Invalid);
                }
            }
            let mut rims = patch.rims;
            rims.sort_by(|a, b| {
                let level =
                    |arc: EllipseArc| dot(sub(arc.ellipse().center_mm(), basis.origin), basis.axis);
                level(a.1).total_cmp(&level(b.1))
            });
            for ((edge, arc), ends) in rims.into_iter().zip(&mut rings) {
                let center = arc.ellipse().center_mm();
                let at = incidence.adjacent(edge, face)?;
                let radial = unit(sub(arc.endpoints()[0], center))?;
                ends.push(End { center, radial, at });
            }
        }
        for ring in &rings {
            if ring.iter().any(|entry| {
                entry.at != ring[0].at
                    || length(sub(entry.center, ring[0].center)) > tol
                    || component.contains(&entry.at.face)
            }) {
                return Err(ReadError::Invalid);
            }
        }
        let ends = rings.map(|ring| ring[0]);
        if dot(sub(ends[1].center, ends[0].center), basis.axis) <= tol {
            return Err(ReadError::Invalid);
        }
        Ok(Some(Self {
            ends,
            axis: basis.axis,
            radius: basis.radius,
            remove: component,
        }))
    }
}

struct Recipe {
    ends: [Point; 2],
    radial: Point,
    radius: f64,
    widen: Option<ConvexRecess>,
}

pub(super) fn reconstruct(
    reader: &Shell<'_, '_>,
    source: &Boundary,
) -> Result<Option<ImportedWarped>> {
    let tol = reader.tolerance;
    let patches: Vec<_> = source
        .faces
        .iter()
        .map(|face| Patch::read(source, face, tol))
        .collect();
    let incidence = Incidence::new(source);
    let mut visited = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut filled = BTreeSet::new();
    let mut recipes = Vec::new();
    for (first, patch) in patches.iter().enumerate() {
        let Some(patch) = *patch else { continue };
        if visited.contains(&first) {
            continue;
        }
        let Some(removal) = Removal::read(first, patch, &patches, &incidence, &mut visited, tol)?
        else {
            continue;
        };
        let recipe = recover(
            reader,
            source,
            &incidence,
            removal,
            &mut removed,
            &mut filled,
        )?;
        add_recipe(&mut recipes, recipe)?;
    }
    if recipes.is_empty() {
        return Ok(None);
    }
    let (base_reader, base) = uncut(reader, source, &removed, &filled)?;
    let (mut imported, chart) = if base.faces.iter().any(|face| !face.holes.is_empty()) {
        bosses::reconstruct_with_chart(&base_reader, &base)?
    } else {
        resolve_chart(&base_reader, &base, true)?
    };
    let tools = recipes
        .into_iter()
        .enumerate()
        .map(|(source, mut recipe)| {
            let [start_mm, end_mm] = recipe.ends.map(|point| chart.local(point));
            if let Some(recess) = &mut recipe.widen {
                for point in &mut recess.rim_mm {
                    *point = chart.local(*point);
                }
                recess.travel_mm = chart.vector(recess.travel_mm);
                recess.source = source;
            }
            (
                AxialCut {
                    start_mm,
                    end_mm,
                    radius_mm: recipe.radius,
                    source,
                    wall_source_face: 0,
                    cap_source_faces: [1, 2],
                    recess: recipe.widen,
                },
                chart.vector(recipe.radial),
            )
        })
        .collect::<Vec<_>>();
    imported.shell = imported
        .shell
        .cut_axial_with_reference(&tools, tol)
        .map_err(replay_error)?;
    verify::replayed(reader, source, chart, &imported.shell)?;
    Ok(Some(imported))
}

fn recover(
    reader: &Shell<'_, '_>,
    source: &Boundary,
    incidence: &Incidence,
    removal: Removal,
    removed: &mut BTreeSet<usize>,
    filled: &mut BTreeSet<Use>,
) -> Result<Recipe> {
    let mut remove = removal.remove;
    let mut fill = Vec::new();
    let [lower, upper] = removal.ends;
    let mut ends = [lower.center, upper.center];
    let mut widen = None;
    for (index, end) in removal.ends.into_iter().enumerate() {
        let face = &source.faces[end.at.face];
        if !matches!(face.surface, Surface::Plane { .. }) {
            return Err(ReadError::Unsupported);
        }
        let direction = if index == 0 { -1. } else { 1. };
        let step = |point: Point| {
            std::array::from_fn(|i| point[i] + direction * removal.axis[i] * 4. * reader.tolerance)
        };
        if end.at.hole.is_none() {
            if !face.holes.is_empty()
                || face.loop_.len() != 4
                || removal.ends[1 - index].at.hole.is_none()
            {
                return Err(ReadError::Unsupported);
            }
            remove.insert(end.at.face);
        } else if let Some(extension) =
            recess::read(reader, source, incidence, end, removal.axis, direction)?
        {
            ends[index] = step(extension.mouth);
            fill.push(extension.host);
            remove.insert(end.at.face);
            remove.extend(extension.walls);
            retain_widening(&mut widen, extension.recess)?;
        } else {
            fill.push(end.at);
            ends[index] = step(end.center);
        }
    }
    for index in remove {
        if !removed.insert(index) {
            return Err(ReadError::Invalid);
        }
    }
    for usage in fill {
        if !filled.insert(usage) {
            return Err(ReadError::Invalid);
        }
    }
    Ok(Recipe {
        ends,
        radial: lower.radial,
        radius: removal.radius,
        widen,
    })
}

fn add_recipe(recipes: &mut Vec<Recipe>, recipe: Recipe) -> Result<()> {
    if recipes.len() == 64 {
        return Err(ReadError::Limit);
    }
    recipes.push(recipe);
    Ok(())
}

fn retain_widening(widening: &mut Option<ConvexRecess>, recess: ConvexRecess) -> Result<()> {
    if widening.replace(recess).is_some() {
        return Err(ReadError::Unsupported);
    }
    Ok(())
}

fn replay_error(cause: warped_union::Error) -> ReadError {
    match cause {
        warped_union::Error::Limit => ReadError::Limit,
        _ => ReadError::Invalid,
    }
}

fn uncut<'a, 'input>(
    reader: &Shell<'a, 'input>,
    source: &Boundary,
    removed: &BTreeSet<usize>,
    filled: &BTreeSet<Use>,
) -> Result<(Shell<'a, 'input>, Boundary)> {
    if filled.iter().any(|usage| removed.contains(&usage.face)) {
        return Err(ReadError::Invalid);
    }
    let mut base_reader = Shell::new(reader.entities, reader.tolerance, true);
    let mut base = Boundary {
        faces: Vec::new(),
        curves: BTreeMap::new(),
    };
    for (index, face) in source.faces.iter().enumerate() {
        if removed.contains(&index) {
            continue;
        }
        let holes: Vec<_> = face
            .holes
            .iter()
            .enumerate()
            .filter(|&(hole, _)| {
                !filled.contains(&Use {
                    face: index,
                    hole: Some(hole),
                })
            })
            .map(|(_, hole)| hole.clone())
            .collect();
        for wire in std::iter::once(&face.loop_).chain(&holes) {
            for &(edge, _) in wire {
                let curve = source.curves[&edge];
                base.curves.insert(edge, curve);
                for id in curve.0 {
                    base_reader.points.insert(id, reader.points[&id]);
                }
            }
        }
        base.faces.push(FaceLoop {
            surface: face.surface,
            loop_: face.loop_.clone(),
            holes,
        });
    }
    Ok((base_reader, base))
}

#[cfg(test)]
mod tests;
