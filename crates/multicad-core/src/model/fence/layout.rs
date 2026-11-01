//! Bounded, source-only flat-run planning. Every path vertex owns one post;
//! interiors are identified by their directed edge and ordinal. Rewired edges
//! never silently inherit another edge's explicit post-position overrides.
use super::*;
use std::collections::BTreeMap;

pub const MAX_FENCE_PATH_POINTS: usize = 256;
pub const MAX_FENCE_MEMBERS: usize = 4096;
const EPS: f64 = 1e-6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceVertex {
    pub id: EntityId,
    pub point: [Dimension; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FencePath {
    pub vertices: Vec<FenceVertex>,
    pub closed: bool,
    pub base_z: Dimension,
}
impl FencePath {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let minimum = if self.closed { 3 } else { 2 };
        if self.vertices.len() < minimum || self.vertices.len() > MAX_FENCE_PATH_POINTS {
            return Err(invalid(format!(
                "A fence path needs {minimum}-{MAX_FENCE_PATH_POINTS} vertices"
            )));
        }
        let mut ids = BTreeSet::new();
        for vertex in &self.vertices {
            if vertex.id.is_nil() || !ids.insert(vertex.id) {
                return Err(invalid(
                    "Fence path vertices need distinct nonempty identities",
                ));
            }
            for dimension in &vertex.point {
                check_dimension(dimension, Unit::Millimeter)?;
            }
        }
        check_dimension(&self.base_z, Unit::Millimeter)
    }
    pub fn inputs(&self) -> Vec<Link> {
        parameter_links(
            self.vertices
                .iter()
                .flat_map(|v| v.point.iter())
                .chain([&self.base_z])
                .collect(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FencePostKey {
    Vertex {
        vertex: EntityId,
    },
    Interior {
        start: EntityId,
        end: EntityId,
        ordinal: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FencePostPosition {
    pub post: FencePostKey,
    /// Center distance from the directed segment's start. Vertex posts are
    /// moved by editing the path, not by an off-path placement override.
    pub offset: Dimension,
}
impl FencePostPosition {
    pub fn inputs(&self) -> Vec<Link> {
        parameter_links(vec![&self.offset])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceBayKey {
    pub start: FencePostKey,
    pub end: FencePostKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceInfillKey {
    pub bay: FenceBayKey,
    pub recipe: EntityId,
    pub ordinal: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FenceLayoutPost {
    pub key: FencePostKey,
    pub position_mm: [f64; 3],
    pub direction: [f64; 2],
    pub overridden: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FenceLayoutBay {
    pub key: FenceBayKey,
    pub segment_start: EntityId,
    pub segment_end: EntityId,
    pub direction: [f64; 2],
    pub center_spacing_mm: f64,
    pub clear_width_mm: f64,
    pub clear_start_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FenceLayoutInfillKind {
    Panel { cut_from_width_mm: Option<f64> },
    Rail,
    Picket,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FenceLayoutInfill {
    pub key: FenceInfillKey,
    pub kind: FenceLayoutInfillKind,
    pub name: String,
    pub material: EntityId,
    /// Center in the horizontal plane, bottom in Z.
    pub position_mm: [f64; 3],
    pub direction: [f64; 2],
    pub width_mm: f64,
    pub depth_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FenceLayout {
    pub construction: ResolvedFenceConstruction,
    pub posts: Vec<FenceLayoutPost>,
    pub bays: Vec<FenceLayoutBay>,
    pub infills: Vec<FenceLayoutInfill>,
}

#[derive(Clone, Copy)]
struct Edge {
    start: [f64; 2],
    direction: [f64; 2],
    length: f64,
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn point(edge: Edge, offset: f64, z: f64) -> [f64; 3] {
    [
        edge.start[0] + edge.direction[0] * offset,
        edge.start[1] + edge.direction[1] * offset,
        z,
    ]
}
fn footprint_half(
    profile: ResolvedFencePostProfile,
    post_direction: [f64; 2],
    bay_direction: [f64; 2],
) -> f64 {
    match profile {
        ResolvedFencePostProfile::Round { diameter_mm } => diameter_mm / 2.,
        ResolvedFencePostProfile::Rectangular { width_mm, depth_mm } => {
            (dot(post_direction, bay_direction).abs() * width_mm
                + cross(post_direction, bay_direction).abs() * depth_mm)
                / 2.
        }
    }
}

/// Includes touching endpoints and collinear overlap, unlike a strict crossing
/// predicate. Adjacent edges are handled separately at their shared vertex.
fn edges_touch(a: Edge, b: Edge) -> bool {
    let ac = sub(b.start, a.start);
    let denominator = cross(a.direction, b.direction);
    if denominator.abs() > 1e-12 {
        let t = cross(ac, b.direction) / denominator;
        let u = cross(ac, a.direction) / denominator;
        return t >= -EPS && t <= a.length + EPS && u >= -EPS && u <= b.length + EPS;
    }
    if cross(ac, a.direction).abs() > EPS {
        return false;
    }
    let start = dot(ac, a.direction);
    let end = start + dot(b.direction, a.direction) * b.length;
    start.min(end) <= a.length + EPS && start.max(end) >= -EPS
}

/// Produce a complete candidate layout or an error. This never edits source
/// and does not certify native member solids or manufacturing joints.
pub fn plan_fence(
    document: &Document,
    parameters: &EvaluatedParameters,
    construction: &FenceConstruction,
    path: &FencePath,
    overrides: &[FencePostPosition],
) -> Result<FenceLayout, DocumentError> {
    path.validate()?;
    let construction = construction.resolve(document, parameters)?;
    let z = number(&path.base_z, Unit::Millimeter, parameters)?;
    if (z + construction.height_mm).abs() > 1e9 {
        return Err(invalid("Fence top exceeds the model distance limit"));
    }
    let points = path
        .vertices
        .iter()
        .map(|v| {
            Ok([
                number(&v.point[0], Unit::Millimeter, parameters)?,
                number(&v.point[1], Unit::Millimeter, parameters)?,
            ])
        })
        .collect::<Result<Vec<_>, DocumentError>>()?;
    let count = if path.closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut edges = Vec::with_capacity(count);
    for i in 0..count {
        let delta = sub(points[(i + 1) % points.len()], points[i]);
        let length = delta[0].hypot(delta[1]);
        if length <= EPS || length > 1e9 {
            return Err(invalid(
                "Fence path contains a zero-length or oversized segment",
            ));
        }
        edges.push(Edge {
            start: points[i],
            direction: [delta[0] / length, delta[1] / length],
            length,
        });
    }
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            if j == i + 1 || (path.closed && i == 0 && j + 1 == edges.len()) {
                continue;
            }
            if edges_touch(edges[i], edges[j]) {
                return Err(invalid(
                    "Fence path crosses, overlaps or touches itself; split it into separate runs",
                ));
            }
        }
    }
    for i in 0..points.len() {
        if !path.closed && (i == 0 || i + 1 == points.len()) {
            continue;
        }
        let before = edges[(i + count - 1) % count].direction;
        let after = edges[i % count].direction;
        let alignment = dot(before, after);
        if alignment < -1. + 1e-9 {
            return Err(invalid("Fence path doubles back at a post"));
        }
        if matches!(
            construction.post,
            ResolvedFencePostProfile::Rectangular { .. }
        ) && alignment.abs() > 1e-9
            && (alignment - 1.).abs() > 1e-9
        {
            return Err(invalid(
                "Rectangular-post fence corners currently require right angles; change the corner or use round posts",
            ));
        }
    }
    if overrides.len() > MAX_FENCE_MEMBERS {
        return Err(invalid("Too many fence post-position overrides"));
    }
    let mut positions = BTreeMap::new();
    for position in overrides {
        if !matches!(position.post, FencePostKey::Interior { ordinal: 1.., .. }) {
            return Err(invalid(
                "Only interior fence posts accept position overrides; edit path vertices directly",
            ));
        }
        check_dimension(&position.offset, Unit::Millimeter)?;
        let offset = number(&position.offset, Unit::Millimeter, parameters)?;
        if offset <= 0. || positions.insert(position.post, offset).is_some() {
            return Err(invalid(
                "Fence post-position overrides must be positive and distinct",
            ));
        }
    }
    let mut posts = Vec::new();
    let mut vertex_posts = Vec::new();
    for (i, vertex) in path.vertices.iter().enumerate() {
        vertex_posts.push(posts.len());
        posts.push(FenceLayoutPost {
            key: FencePostKey::Vertex { vertex: vertex.id },
            position_mm: [points[i][0], points[i][1], z],
            direction: edges[i.min(count - 1)].direction,
            overridden: false,
        });
    }
    let mut bays = Vec::new();
    for (i, edge) in edges.iter().copied().enumerate() {
        let j = (i + 1) % points.len();
        let ratio = edge.length / construction.preferred_post_spacing_mm;
        // A few floating-point rounding bits must not create a microscopic
        // extra bay at an otherwise integral pitch. The final endpoint keeps
        // its exact authored coordinate, including the residual distance.
        let nearest = ratio.round();
        let divisions = if (ratio - nearest).abs() <= 16. * f64::EPSILON * ratio.max(1.) {
            nearest
        } else {
            ratio.ceil()
        }
        .max(1.);
        if divisions > MAX_FENCE_MEMBERS as f64
            || posts.len() + divisions as usize - 1 > MAX_FENCE_MEMBERS
        {
            return Err(invalid(
                "Fence spacing exceeds the bounded member count; increase spacing or split the run",
            ));
        }
        let divisions = divisions as usize;
        let pitch = match construction.layout {
            FenceBayLayout::Equal => edge.length / divisions as f64,
            FenceBayLayout::ShortLast => construction.preferred_post_spacing_mm,
        };
        let mut last = vertex_posts[i];
        let mut last_offset = 0.;
        for k in 1..=divisions {
            let (next, offset) = if k == divisions {
                (vertex_posts[j], edge.length)
            } else {
                let key = FencePostKey::Interior {
                    start: path.vertices[i].id,
                    end: path.vertices[j].id,
                    ordinal: k as u32,
                };
                let override_offset = positions.remove(&key);
                let offset = override_offset.unwrap_or(k as f64 * pitch);
                let next = posts.len();
                posts.push(FenceLayoutPost {
                    key,
                    position_mm: point(edge, offset, z),
                    direction: edge.direction,
                    overridden: override_offset.is_some(),
                });
                (next, offset)
            };
            if offset <= last_offset + EPS || offset > edge.length {
                return Err(invalid(
                    "Fence post override crosses a neighbor or lies beyond its segment; repair post positions",
                ));
            }
            let left = footprint_half(construction.post, posts[last].direction, edge.direction);
            let right = footprint_half(construction.post, posts[next].direction, edge.direction);
            let clear_width_mm = offset - last_offset - left - right;
            if clear_width_mm <= EPS {
                return Err(invalid(
                    "Fence posts collide or leave no clear bay; change spacing or post positions",
                ));
            }
            bays.push(FenceLayoutBay {
                key: FenceBayKey {
                    start: posts[last].key,
                    end: posts[next].key,
                },
                segment_start: path.vertices[i].id,
                segment_end: path.vertices[j].id,
                direction: edge.direction,
                center_spacing_mm: offset - last_offset,
                clear_width_mm,
                clear_start_mm: point(edge, last_offset + left, z),
            });
            last = next;
            last_offset = offset;
        }
    }
    if !positions.is_empty() {
        return Err(invalid(
            "An overridden fence post no longer exists on this directed path edge; explicitly repair or remove its override",
        ));
    }
    let mut infills = Vec::new();
    for bay in &bays {
        for recipe in &construction.infills {
            let (number, width, depth, gap, kind) = match recipe.construction {
                ResolvedFenceInfillKind::Panel {
                    thickness_mm,
                    width,
                } => {
                    let actual = width.width_for_bay(bay.clear_width_mm)?;
                    let cut_from_width_mm = match width {
                        ResolvedFencePanelWidth::Purchased { width_mm, .. }
                            if actual < width_mm =>
                        {
                            Some(width_mm)
                        }
                        _ => None,
                    };
                    (
                        1,
                        actual,
                        thickness_mm,
                        (bay.clear_width_mm - actual) / 2.,
                        FenceLayoutInfillKind::Panel { cut_from_width_mm },
                    )
                }
                ResolvedFenceInfillKind::Rail { depth_mm } => (
                    1,
                    bay.clear_width_mm,
                    depth_mm,
                    0.,
                    FenceLayoutInfillKind::Rail,
                ),
                ResolvedFenceInfillKind::Pickets {
                    width_mm,
                    depth_mm,
                    maximum_gap_mm,
                } => {
                    if width_mm > bay.clear_width_mm {
                        return Err(invalid("Fence picket is wider than the clear bay"));
                    }
                    let number = ((bay.clear_width_mm - maximum_gap_mm)
                        / (width_mm + maximum_gap_mm))
                        .ceil()
                        .max(1.);
                    if number > MAX_FENCE_MEMBERS as f64 {
                        return Err(invalid(
                            "Fence picket spacing exceeds the bounded member count",
                        ));
                    }
                    let gap = (bay.clear_width_mm - number * width_mm) / (number + 1.);
                    if gap < 0. || gap > maximum_gap_mm + EPS {
                        return Err(invalid(
                            "Fence pickets cannot fit within the requested clear-gap limit",
                        ));
                    }
                    (
                        number as usize,
                        width_mm,
                        depth_mm,
                        gap,
                        FenceLayoutInfillKind::Picket,
                    )
                }
            };
            if posts.len() + infills.len() + number > MAX_FENCE_MEMBERS {
                return Err(invalid(
                    "Fence exceeds the bounded total member count; simplify or split the run",
                ));
            }
            let normal = [-bay.direction[1], bay.direction[0]];
            for ordinal in 0..number {
                let station = gap + width / 2. + ordinal as f64 * (width + gap);
                let position_mm = [
                    bay.clear_start_mm[0]
                        + bay.direction[0] * station
                        + normal[0] * recipe.normal_offset_mm,
                    bay.clear_start_mm[1]
                        + bay.direction[1] * station
                        + normal[1] * recipe.normal_offset_mm,
                    z + recipe.bottom_mm,
                ];
                if position_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > 1e9)
                {
                    return Err(invalid(
                        "Fence member placement exceeds the model distance limit",
                    ));
                }
                infills.push(FenceLayoutInfill {
                    key: FenceInfillKey {
                        bay: bay.key,
                        recipe: recipe.id,
                        ordinal: ordinal as u32,
                    },
                    kind,
                    name: recipe.name.clone(),
                    material: recipe.material,
                    position_mm,
                    direction: bay.direction,
                    width_mm: width,
                    depth_mm: depth,
                    height_mm: recipe.height_mm,
                });
            }
        }
    }
    Ok(FenceLayout {
        construction,
        posts,
        bays,
        infills,
    })
}
