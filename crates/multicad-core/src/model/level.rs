//! Building levels and level-hosted architecture: slabs and pedestals,
//! straight, quarter-turn and half-turn stairs, and service envelopes around
//! placed equipment. Levels are document extensions; features link to them,
//! so a level edit rebuilds every hosted element. Stair checks are geometric
//! reports only, never building-code, accessibility or structural approval.
use super::{
    CreatedBody, ModelPrepared,
    common::{error, number, parameter_links, revision},
    source::{BodyOutput, check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    parameters::{EvaluatedParameters, Unit},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const LEVEL_KIND: &str = "architecture.level";
pub const LEVEL_CAPABILITY: &str = "architecture.level.1";
pub const SLAB_TYPE: &str = "architecture.slab";
pub const SLAB_CAPABILITY: &str = "architecture.slab.1";
/// Required by documents whose slabs carry vertical-cutout holes, so a build
/// that does not know holes refuses the file instead of building solid slabs.
pub const SLAB_HOLE_CAPABILITY: &str = "architecture.slab_hole.1";
pub const MAX_SLAB_HOLES: usize = 256;
pub const STAIR_TYPE: &str = "architecture.stair";
pub const STAIR_CAPABILITY: &str = "architecture.stair.1";
pub const ENVELOPE_KIND: &str = "architecture.envelope";
pub const ENVELOPE_CAPABILITY: &str = "architecture.envelope.1";
pub const MAX_RISERS: u32 = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelDefinition {
    pub elevation: Dimension,
    /// Floor-to-floor height up to the next level.
    pub height: Dimension,
    pub slab_thickness: Dimension,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlabKind {
    /// Top at the level elevation, extending downward by the slab thickness.
    Slab,
    /// A raised platform standing on the level, `height` tall.
    Pedestal,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlabDefinition {
    pub output: BodyOutput,
    pub level: EntityId,
    pub kind: SlabKind,
    /// Closed outline in world X/Y millimeters.
    pub outline: Vec<[Dimension; 2]>,
    /// Slab thickness override, or pedestal height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<Dimension>,
    /// Rectangular holes cut by vertical cutouts (shafts, floor
    /// penetrations). Omitted from the file when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<SlabHole>,
}
/// One rectangular hole through a slab or pedestal, from its bottom to its
/// top. `id` names the vertical cutout it belongs to; a shaft shares one
/// identity across every slab it cuts, and with the wall cutout when it also
/// cuts a wall.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlabHole {
    pub id: EntityId,
    /// Rectangle center in world X/Y millimeters.
    pub center: [Dimension; 2],
    /// Size along the rotated X direction.
    pub width: Dimension,
    /// Size along the rotated Y direction.
    pub depth: Dimension,
    /// Rotation of the rectangle's width direction, degrees from +X.
    #[serde(default)]
    pub rotation_deg: f64,
}
impl SlabHole {
    fn dimensions(&self) -> [&Dimension; 4] {
        [&self.center[0], &self.center[1], &self.width, &self.depth]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Turn {
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StairForm {
    Straight,
    QuarterTurn { turn: Turn },
    HalfTurn { turn: Turn },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StairDefinition {
    pub output: BodyOutput,
    /// The stair rises from this level to `to_level`, or by its floor-to-floor height.
    pub level: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_level: Option<EntityId>,
    pub form: StairForm,
    /// Start of the first riser's centerline, world X/Y.
    pub start: [Dimension; 2],
    /// Walking direction of the first flight, degrees from +X.
    pub direction_deg: f64,
    pub width: Dimension,
    pub risers: u32,
    pub going: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeDefinition {
    /// The placed Part or Instance the envelope surrounds.
    pub target: EntityId,
    /// Clearances beyond the target's own bounds, in its local frame:
    /// front (−Y), back (+Y), left (−X), right (+X), top (+Z).
    pub front: Dimension,
    pub back: Dimension,
    pub left: Dimension,
    pub right: Dimension,
    pub top: Dimension,
    #[serde(default)]
    pub label: String,
}

fn definition<T: Serialize>(type_id: &str, data: &T) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: type_id.into(),
        schema_version: 1,
        data: serde_json::to_value(data).map_err(|e| invalid(e.to_string()))?,
    })
}
fn parse<T: for<'a> Deserialize<'a>>(d: &Definition, type_id: &str) -> Result<T, DocumentError> {
    if d.type_id != type_id || d.schema_version != 1 {
        return Err(invalid(format!("Expected {type_id} schema 1")));
    }
    serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))
}
fn check_all(dims: &[&Dimension]) -> Result<(), DocumentError> {
    for d in dims {
        check_dimension(d, Unit::Millimeter)?;
    }
    Ok(())
}
impl LevelDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        check_all(&[&self.elevation, &self.height, &self.slab_thickness])
    }
    fn inputs(&self) -> Vec<Link> {
        parameter_links(vec![&self.elevation, &self.height, &self.slab_thickness])
    }
}
impl SlabDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.outline.len() < 3 || self.outline.len() > 256 {
            return Err(invalid("A slab outline needs 3–256 points"));
        }
        if self.holes.len() > MAX_SLAB_HOLES {
            return Err(invalid(format!(
                "A slab carries at most {MAX_SLAB_HOLES} cutout holes"
            )));
        }
        let mut ids = BTreeSet::new();
        for hole in &self.holes {
            if hole.id.is_nil() || !ids.insert(hole.id) {
                return Err(invalid("Slab cutout holes need distinct identities"));
            }
            if !hole.rotation_deg.is_finite() {
                return Err(invalid("Slab cutout rotation must be finite"));
            }
        }
        check_all(&self.dimensions())
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut dims: Vec<&Dimension> = self.outline.iter().flatten().collect();
        dims.extend(&self.thickness);
        dims.extend(self.holes.iter().flat_map(SlabHole::dimensions));
        dims
    }
    /// Whether this slab carries a hole of the vertical cutout `id`.
    pub fn has_hole(&self, id: EntityId) -> bool {
        self.holes.iter().any(|h| h.id == id)
    }
    pub fn inputs(&self) -> Vec<Link> {
        let dims = self.dimensions();
        let mut links = parameter_links(dims);
        links.push(Link::new(self.level, "level"));
        links
    }
}
impl StairDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if !(2..=MAX_RISERS).contains(&self.risers) {
            return Err(invalid(format!("A stair needs 2–{MAX_RISERS} risers")));
        }
        if matches!(
            self.form,
            StairForm::QuarterTurn { .. } | StairForm::HalfTurn { .. }
        ) && self.risers < 4
        {
            return Err(invalid("A turning stair needs at least 4 risers"));
        }
        if !self.direction_deg.is_finite() {
            return Err(invalid("Stair direction must be finite"));
        }
        check_all(&[&self.start[0], &self.start[1], &self.width, &self.going])
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(vec![
            &self.start[0],
            &self.start[1],
            &self.width,
            &self.going,
        ]);
        links.push(Link::new(self.level, "level"));
        links.extend(self.to_level.map(|l| Link::new(l, "to_level")));
        links
    }
}
impl EnvelopeDefinition {
    pub fn entity(&self, id: EntityId, name: &str) -> Result<Entity, DocumentError> {
        self.validate()?;
        let mut entity = Entity::new(
            name,
            EntityData::Extension {
                definition: definition(ENVELOPE_KIND, self)?,
                inputs: self.inputs(),
            },
        );
        entity.id = id;
        Ok(entity)
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.label.len() > 256 {
            return Err(invalid("Envelope label is too long"));
        }
        check_all(&[&self.front, &self.back, &self.left, &self.right, &self.top])
    }
    fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(vec![
            &self.front,
            &self.back,
            &self.left,
            &self.right,
            &self.top,
        ]);
        links.push(Link::new(self.target, "target"));
        links
    }
}

fn extension(document: &Document, id: EntityId, kind: &str) -> Result<Definition, DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, .. }) if definition.type_id == kind => {
            Ok(definition.clone())
        }
        Some(_) => Err(invalid(format!("{id} is not a {kind}"))),
        None => Err(DocumentError::Missing(id)),
    }
}
pub fn read_level(document: &Document, id: EntityId) -> Result<LevelDefinition, DocumentError> {
    let v: LevelDefinition = parse(&extension(document, id, LEVEL_KIND)?, LEVEL_KIND)?;
    v.validate()?;
    Ok(v)
}
pub fn read_envelope(
    document: &Document,
    id: EntityId,
) -> Result<EnvelopeDefinition, DocumentError> {
    let v: EnvelopeDefinition = parse(&extension(document, id, ENVELOPE_KIND)?, ENVELOPE_KIND)?;
    v.validate()?;
    if document.get(v.target).is_some()
        && !matches!(
            document.get(v.target).map(|e| &e.data),
            Some(EntityData::Part { .. } | EntityData::Instance { .. })
        )
    {
        return Err(invalid(
            "A service envelope surrounds a placed Part or Instance",
        ));
    }
    Ok(v)
}
fn kind_entities<'a>(
    document: &'a Document,
    kind: &'a str,
) -> impl Iterator<Item = &'a Entity> + 'a {
    document.entities().filter(move |e| matches!(&e.data, EntityData::Extension{definition,..} if definition.type_id==kind))
}
pub fn levels(document: &Document) -> impl Iterator<Item = &Entity> {
    kind_entities(document, LEVEL_KIND)
}
pub fn envelopes(document: &Document) -> impl Iterator<Item = &Entity> {
    kind_entities(document, ENVELOPE_KIND)
}
fn feature<T: for<'a> Deserialize<'a>>(
    document: &Document,
    id: EntityId,
    type_id: &str,
) -> Result<(T, Vec<Link>), DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose an architecture feature"));
    };
    Ok((parse(definition, type_id)?, inputs.clone()))
}
fn check_links(actual: &[Link], expected: &[Link]) -> Result<(), DocumentError> {
    if actual.len() != expected.len() || expected.iter().any(|l| !actual.contains(l)) {
        return Err(invalid(
            "Architecture source and canonical input links disagree",
        ));
    }
    Ok(())
}
fn check_output(
    document: &Document,
    feature: EntityId,
    body: EntityId,
) -> Result<(), DocumentError> {
    if let Some(entity) = document.get(body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Architecture output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("Architecture output Part must contain its feature"));
        }
    }
    Ok(())
}
pub fn read_slab(document: &Document, id: EntityId) -> Result<SlabDefinition, DocumentError> {
    let (v, inputs): (SlabDefinition, _) = feature(document, id, SLAB_TYPE)?;
    v.validate()?;
    check_links(&inputs, &v.inputs())?;
    if document.get(v.level).is_some() {
        read_level(document, v.level)?;
    }
    check_output(document, id, v.output.body)?;
    Ok(v)
}
pub fn read_stair(document: &Document, id: EntityId) -> Result<StairDefinition, DocumentError> {
    let (v, inputs): (StairDefinition, _) = feature(document, id, STAIR_TYPE)?;
    v.validate()?;
    check_links(&inputs, &v.inputs())?;
    for l in std::iter::once(v.level).chain(v.to_level) {
        if document.get(l).is_some() {
            read_level(document, l)?;
        }
    }
    check_output(document, id, v.output.body)?;
    Ok(v)
}
fn features_of<'a>(
    document: &'a Document,
    type_id: &'a str,
) -> impl Iterator<Item = &'a Entity> + 'a {
    document.entities().filter(move |e| matches!(&e.data, EntityData::Feature{definition,..} if definition.type_id==type_id))
}
/// Every slab and pedestal feature.
pub fn slabs(document: &Document) -> impl Iterator<Item = &Entity> {
    features_of(document, SLAB_TYPE)
}
impl SlabDefinition {
    /// The canonical feature definition of this slab.
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        definition(SLAB_TYPE, self)
    }
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for e in levels(document) {
        read_level(document, e.id)?;
    }
    for e in envelopes(document) {
        read_envelope(document, e.id)?;
    }
    for e in features_of(document, SLAB_TYPE) {
        read_slab(document, e.id)?;
    }
    for e in features_of(document, STAIR_TYPE) {
        read_stair(document, e.id)?;
    }
    Ok(())
}

// ---- Resolution ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedLevel {
    pub elevation_mm: f64,
    pub height_mm: f64,
    pub slab_mm: f64,
}
pub fn resolve_level(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
) -> Result<ResolvedLevel, DocumentError> {
    let l = read_level(document, id)?;
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let r = ResolvedLevel {
        elevation_mm: mm(&l.elevation)?,
        height_mm: mm(&l.height)?,
        slab_mm: mm(&l.slab_thickness)?,
    };
    if r.height_mm <= 1e-3 || r.slab_mm <= 1e-3 {
        return Err(invalid("Level height and slab thickness must be positive"));
    }
    Ok(r)
}
/// A slab or pedestal as a closed outline and a vertical span.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSlab {
    pub outline: Vec<[f64; 2]>,
    pub bottom_mm: f64,
    pub top_mm: f64,
    pub holes: Vec<ResolvedSlabHole>,
}
/// A slab hole as its four plan corners, counterclockwise.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSlabHole {
    pub id: EntityId,
    pub corners: [[f64; 2]; 4],
}
/// The counterclockwise corners of a `width` x `depth` rectangle centered at
/// `center` whose width runs `rotation_deg` from +X.
pub fn rectangle_corners(
    center: [f64; 2],
    width: f64,
    depth: f64,
    rotation_deg: f64,
) -> [[f64; 2]; 4] {
    let (s, c) = rotation_deg.to_radians().sin_cos();
    let at = |u: f64, v: f64| [center[0] + c * u - s * v, center[1] + s * u + c * v];
    let (u, v) = (width / 2., depth / 2.);
    [at(-u, -v), at(u, -v), at(u, v), at(-u, v)]
}
pub fn resolve_slab(
    document: &Document,
    parameters: &EvaluatedParameters,
    slab: &SlabDefinition,
) -> Result<ResolvedSlab, DocumentError> {
    let level = resolve_level(document, parameters, slab.level)?;
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let outline = slab
        .outline
        .iter()
        .map(|[x, y]| Ok([mm(x)?, mm(y)?]))
        .collect::<Result<Vec<_>, DocumentError>>()?;
    let n = outline.len();
    let area: f64 = (0..n)
        .map(|i| outline[i][0] * outline[(i + 1) % n][1] - outline[(i + 1) % n][0] * outline[i][1])
        .sum::<f64>()
        / 2.;
    if area.abs() < 1. {
        return Err(invalid("Slab outline has no area"));
    }
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if crosses(
                outline[i],
                outline[(i + 1) % n],
                outline[j],
                outline[(j + 1) % n],
            ) {
                return Err(invalid("Slab outline self-intersects"));
            }
        }
    }
    let (bottom_mm, top_mm) = match slab.kind {
        SlabKind::Slab => {
            let t = slab
                .thickness
                .as_ref()
                .map(mm)
                .transpose()?
                .unwrap_or(level.slab_mm);
            (level.elevation_mm - t, level.elevation_mm)
        }
        SlabKind::Pedestal => {
            let h = slab
                .thickness
                .as_ref()
                .map(mm)
                .transpose()?
                .ok_or_else(|| invalid("A pedestal needs a height"))?;
            (level.elevation_mm, level.elevation_mm + h)
        }
    };
    if top_mm - bottom_mm <= 1e-3 {
        return Err(invalid("Slab or pedestal thickness must be positive"));
    }
    let mut holes: Vec<ResolvedSlabHole> = vec![];
    for (k, hole) in slab.holes.iter().enumerate() {
        let label = format!("Vertical cutout hole {}", k + 1);
        let (w, d) = (mm(&hole.width)?, mm(&hole.depth)?);
        if w <= 1e-3 || d <= 1e-3 {
            return Err(invalid(format!("{label} needs a positive width and depth")));
        }
        let corners = rectangle_corners(
            [mm(&hole.center[0])?, mm(&hole.center[1])?],
            w,
            d,
            hole.rotation_deg,
        );
        if !inside_outline(&outline, &corners) {
            return Err(invalid(format!(
                "{label} is not inside the slab outline; a cutout must keep slab material all around it and may not reach, cross or split the slab edge"
            )));
        }
        if holes.iter().any(|h| convex_overlap(&h.corners, &corners)) {
            return Err(invalid(format!(
                "{label} overlaps or touches another cutout in this slab"
            )));
        }
        holes.push(ResolvedSlabHole {
            id: hole.id,
            corners,
        });
    }
    Ok(ResolvedSlab {
        outline,
        bottom_mm,
        top_mm,
        holes,
    })
}
fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-18 {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0., 1.)
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}
/// Even-odd point in polygon; points on the boundary may go either way.
fn point_in_polygon(p: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let n = polygon.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        {
            inside = !inside;
        }
    }
    inside
}
fn distance_to_boundary(p: [f64; 2], polygon: &[[f64; 2]]) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| distance_to_segment(p, polygon[i], polygon[(i + 1) % n]))
        .fold(f64::INFINITY, f64::min)
}
/// Whether `inner` lies strictly inside `outline`, clear of its boundary.
pub fn inside_outline(outline: &[[f64; 2]], inner: &[[f64; 2]]) -> bool {
    const CLEAR: f64 = 1e-3;
    let (n, m) = (outline.len(), inner.len());
    inner
        .iter()
        .all(|p| point_in_polygon(*p, outline) && distance_to_boundary(*p, outline) > CLEAR)
        && outline
            .iter()
            .all(|p| !point_in_polygon(*p, inner) && distance_to_boundary(*p, inner) > CLEAR)
        && (0..n).all(|i| {
            (0..m).all(|j| {
                !crosses(
                    outline[i],
                    outline[(i + 1) % n],
                    inner[j],
                    inner[(j + 1) % m],
                )
            })
        })
}
/// Whether two convex polygons overlap or touch (separating-axis test).
pub fn convex_overlap(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    const GAP: f64 = 1e-3;
    for polygon in [a, b] {
        let n = polygon.len();
        for i in 0..n {
            let (p, q) = (polygon[i], polygon[(i + 1) % n]);
            let axis = [p[1] - q[1], q[0] - p[0]];
            let l = axis[0].hypot(axis[1]);
            if l < 1e-12 {
                continue;
            }
            let project = |s: &[[f64; 2]]| {
                s.iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                        let d = (v[0] * axis[0] + v[1] * axis[1]) / l;
                        (lo.min(d), hi.max(d))
                    })
            };
            let ((a0, a1), (b0, b1)) = (project(a), project(b));
            if a1 < b0 - GAP || b1 < a0 - GAP {
                return false;
            }
        }
    }
    true
}
/// Whether two simple polygons share any area or boundary.
pub fn polygons_meet(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    let (n, m) = (a.len(), b.len());
    a.iter().any(|p| point_in_polygon(*p, b))
        || b.iter().any(|p| point_in_polygon(*p, a))
        || (0..n).any(|i| {
            (0..m).any(|j| {
                distance_to_segment(a[i], b[j], b[(j + 1) % m]) < 1e-3
                    || crosses(a[i], a[(i + 1) % n], b[j], b[(j + 1) % m])
            })
        })
}
fn crosses(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross = |p: [f64; 2], q: [f64; 2]| p[0] * q[1] - p[1] * q[0];
    let sub = |p: [f64; 2], q: [f64; 2]| [p[0] - q[0], p[1] - q[1]];
    let (r, s) = (sub(b, a), sub(d, c));
    let den = cross(r, s);
    if den.abs() < 1e-12 {
        return false;
    }
    let t = cross(sub(c, a), s) / den;
    let u = cross(sub(c, a), r) / den;
    t > 1e-9 && t < 1. - 1e-9 && u > 1e-9 && u < 1. - 1e-9
}

/// One flight or landing of a stair as a vertical side profile in its own
/// frame: `origin` and unit `along` on plan, extruded by `width` to the left
/// of `along`. Profile points are (distance along, height above `base_z`).
#[derive(Clone, Debug, PartialEq)]
pub struct StairPiece {
    pub origin: [f64; 2],
    pub along: [f64; 2],
    pub width_mm: f64,
    pub base_z_mm: f64,
    pub profile: Vec<[f64; 2]>,
    pub role: String,
}
/// Geometric stair measures. These are reported for review; they are not a
/// building-code or accessibility approval.
#[derive(Clone, Debug, PartialEq)]
pub struct StairCheck {
    pub rise_mm: f64,
    pub riser_mm: f64,
    pub going_mm: f64,
    /// 2 × riser + going, the classic step-comfort measure.
    pub step_measure_mm: f64,
    pub run_mm: f64,
    pub notes: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStair {
    pub pieces: Vec<StairPiece>,
    pub check: StairCheck,
    /// The walking line on plan, start to end, for plan arrows.
    pub walking_line: Vec<[f64; 2]>,
}
/// A flight's side profile: `risers` risers lifting from `lift` above the
/// base, a tread between each, and a top platform `top` deep. The mass runs
/// down to the base so flights and landings share whole faces.
fn flight_profile(risers: u32, riser: f64, going: f64, lift: f64, top: f64) -> Vec<[f64; 2]> {
    let mut p = vec![[0., 0.]];
    for k in 0..risers {
        let x = k as f64 * going;
        p.push([x, lift + (k + 1) as f64 * riser]);
        if k + 1 < risers {
            p.push([x + going, lift + (k + 1) as f64 * riser]);
        }
    }
    let end = (risers - 1) as f64 * going + top;
    p.push([end, lift + risers as f64 * riser]);
    p.push([end, 0.]);
    p
}
pub fn resolve_stair(
    document: &Document,
    parameters: &EvaluatedParameters,
    stair: &StairDefinition,
) -> Result<ResolvedStair, DocumentError> {
    let base = resolve_level(document, parameters, stair.level)?;
    let rise = match stair.to_level {
        Some(top) => resolve_level(document, parameters, top)?.elevation_mm - base.elevation_mm,
        None => base.height_mm,
    };
    if rise <= 1. {
        return Err(invalid(
            "The stair's upper level must be above its base level",
        ));
    }
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let (width, going) = (mm(&stair.width)?, mm(&stair.going)?);
    if width <= 1. || going <= 1. {
        return Err(invalid("Stair width and going must be positive"));
    }
    let start = [mm(&stair.start[0])?, mm(&stair.start[1])?];
    let riser = rise / stair.risers as f64;
    let (s, c) = stair.direction_deg.to_radians().sin_cos();
    let d1 = [c, s];
    let left = |d: [f64; 2]| [-d[1], d[0]];
    let at = |p: [f64; 2], d: [f64; 2], k: f64| [p[0] + d[0] * k, p[1] + d[1] * k];
    // A piece's frame origin is the right edge of its centerline start.
    let piece = |centre: [f64; 2], d: [f64; 2], profile: Vec<[f64; 2]>, role: &str| StairPiece {
        origin: at(centre, left(d), -width / 2.),
        along: d,
        width_mm: width,
        base_z_mm: base.elevation_mm,
        profile,
        role: role.into(),
    };
    let mut pieces = vec![];
    let mut notes = vec![];
    let (walking, run) = match stair.form {
        StairForm::Straight => {
            let n = stair.risers;
            pieces.push(piece(
                start,
                d1,
                flight_profile(n, riser, going, 0., going),
                "stair.flight.0",
            ));
            let end = at(start, d1, n as f64 * going);
            (vec![start, end], n as f64 * going)
        }
        StairForm::QuarterTurn { turn } | StairForm::HalfTurn { turn } => {
            let (n1, half) = (
                stair.risers / 2,
                matches!(stair.form, StairForm::HalfTurn { .. }),
            );
            let n2 = stair.risers - n1;
            let t = if turn == Turn::Left {
                left(d1)
            } else {
                [d1[1], -d1[0]]
            };
            let landing = n1 as f64 * riser;
            notes.push(format!("Landing at {landing:.0} mm above the base level"));
            // Flight 1 ends in a landing one width deep.
            pieces.push(piece(
                start,
                d1,
                flight_profile(n1, riser, going, 0., width),
                "stair.flight.0",
            ));
            let landing_start = at(start, d1, (n1 - 1) as f64 * going);
            let landing_mid = at(landing_start, d1, width / 2.);
            let platform =
                |depth: f64| vec![[0., 0.], [0., landing], [depth, landing], [depth, 0.]];
            let (centre2, d2) = if half {
                // The landing extends one width across to the return flight.
                let beside = at(landing_start, t, width);
                pieces.push(piece(beside, d1, platform(width), "stair.landing"));
                (beside, [-d1[0], -d1[1]])
            } else {
                (at(landing_mid, t, width / 2.), t)
            };
            pieces.push(piece(
                centre2,
                d2,
                flight_profile(n2, riser, going, landing, going),
                "stair.flight.1",
            ));
            let end = at(centre2, d2, n2 as f64 * going);
            let turn_point = if half {
                at(landing_mid, t, width / 2.)
            } else {
                landing_mid
            };
            (
                vec![start, landing_start, turn_point, centre2, end],
                (n1 - 1) as f64 * going + width + n2 as f64 * going,
            )
        }
    };
    let measure = 2. * riser + going;
    if !(550. ..=700.).contains(&measure) {
        notes.push(format!(
            "Step measure 2R+G = {measure:.0} mm lies outside the common 550–700 mm comfort range (geometric note only)"
        ));
    }
    Ok(ResolvedStair {
        pieces,
        check: StairCheck {
            rise_mm: rise,
            riser_mm: riser,
            going_mm: going,
            step_measure_mm: measure,
            run_mm: run,
            notes,
        },
        walking_line: walking,
    })
}

// ---- Transactions ----------------------------------------------------------

fn insert_extension(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    label: &str,
    name: &str,
    definition: Definition,
    inputs: Vec<Link>,
) -> Result<(EntityId, Prepared), CommandError> {
    revision(session, expected_revision)?;
    if name.trim().is_empty() || name.len() > 256 {
        return Err(error("Name must contain 1–256 bytes"));
    }
    let entity = Entity::new(name, EntityData::Extension { definition, inputs });
    let id = entity.id;
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits: vec![Mutation::Insert(entity)],
    })?;
    Ok((id, prepared))
}
fn replace_extension(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    label: &str,
    id: EntityId,
    definition: Definition,
    inputs: Vec<Link>,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    let mut entity = session
        .document()
        .get(id)
        .ok_or_else(|| error("Entity is missing"))?
        .clone();
    entity.data = EntityData::Extension { definition, inputs };
    session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits: vec![Mutation::Replace(entity)],
    })
}
pub fn prepare_level(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    value: &LevelDefinition,
) -> Result<(EntityId, Prepared), CommandError> {
    value.validate()?;
    insert_extension(
        session,
        expected_revision,
        actor,
        "Create level",
        name,
        definition(LEVEL_KIND, value)?,
        value.inputs(),
    )
}
/// Edit a level. Every slab, pedestal, stair and wall hosted on it rebuilds.
pub fn prepare_level_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    value: &LevelDefinition,
) -> Result<Prepared, CommandError> {
    value.validate()?;
    read_level(session.document(), id)?;
    replace_extension(
        session,
        expected_revision,
        actor,
        "Edit level",
        id,
        definition(LEVEL_KIND, value)?,
        value.inputs(),
    )
}
pub fn prepare_envelope(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    value: &EnvelopeDefinition,
) -> Result<(EntityId, Prepared), CommandError> {
    value.validate()?;
    if !matches!(
        session.document().get(value.target).map(|e| &e.data),
        Some(EntityData::Part { .. } | EntityData::Instance { .. })
    ) {
        return Err(error("Choose a placed Part or Instance for the envelope"));
    }
    insert_extension(
        session,
        expected_revision,
        actor,
        "Add service envelope",
        name,
        definition(ENVELOPE_KIND, value)?,
        value.inputs(),
    )
}
pub fn prepare_envelope_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    value: &EnvelopeDefinition,
) -> Result<Prepared, CommandError> {
    value.validate()?;
    read_envelope(session.document(), id)?;
    replace_extension(
        session,
        expected_revision,
        actor,
        "Edit service envelope",
        id,
        definition(ENVELOPE_KIND, value)?,
        value.inputs(),
    )
}
/// A new slab, pedestal or stair Part.
pub struct ArchitectureRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub definition: Definition,
    pub inputs: Vec<Link>,
    pub material: Option<EntityId>,
}
fn create(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    label: &str,
    r: ArchitectureRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    for id in [r.feature, r.body] {
        if id.is_nil() || session.document().get(id).is_some() || r.feature == r.body {
            return Err(error("Identities must be new, distinct and non-nil"));
        }
    }
    if r.name.trim().is_empty() || r.name.len() > 1000 {
        return Err(error("Name must contain 1–1000 bytes"));
    }
    let mut feature = Entity::new(
        format!("{} feature", r.name),
        EntityData::Feature {
            definition: r.definition,
            inputs: r.inputs,
            suppressed: false,
        },
    );
    feature.id = r.feature;
    let mut body = Entity::new(
        r.name,
        EntityData::Part {
            features: vec![r.feature],
            material: r.material,
            tags: BTreeSet::new(),
        },
    );
    body.id = r.body;
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits: vec![Mutation::Insert(feature), Mutation::Insert(body)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature: r.feature,
            body: r.body,
        }],
    })
}
#[allow(clippy::too_many_arguments)]
fn edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    label: &str,
    feature: EntityId,
    definition: Definition,
    inputs: Vec<Link>,
    body: EntityId,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut entity = session
        .document()
        .get(feature)
        .ok_or_else(|| error("Feature is missing"))?
        .clone();
    entity.data = EntityData::Feature {
        definition,
        inputs,
        suppressed: false,
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody { feature, body }],
    })
}
/// Create a slab or pedestal; `slab.output.body` names the new Part.
pub fn prepare_slab(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    slab: &SlabDefinition,
    material: Option<EntityId>,
) -> Result<ModelPrepared, CommandError> {
    slab.validate()?;
    resolve_slab(session.document(), session.parameters(), slab)?;
    create(
        session,
        expected_revision,
        actor,
        "Create slab",
        ArchitectureRequest {
            feature: EntityId::new(),
            body: slab.output.body,
            name: name.into(),
            definition: definition(SLAB_TYPE, slab)?,
            inputs: slab.inputs(),
            material,
        },
    )
}
pub fn prepare_slab_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    slab: &SlabDefinition,
) -> Result<ModelPrepared, CommandError> {
    let current = read_slab(session.document(), feature)?;
    if current.output != slab.output {
        return Err(error("A slab edit keeps its output Part"));
    }
    slab.validate()?;
    resolve_slab(session.document(), session.parameters(), slab)?;
    edit(
        session,
        expected_revision,
        actor,
        "Edit slab",
        feature,
        definition(SLAB_TYPE, slab)?,
        slab.inputs(),
        slab.output.body,
    )
}
pub fn prepare_stair(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    stair: &StairDefinition,
    material: Option<EntityId>,
) -> Result<ModelPrepared, CommandError> {
    stair.validate()?;
    resolve_stair(session.document(), session.parameters(), stair)?;
    create(
        session,
        expected_revision,
        actor,
        "Create stair",
        ArchitectureRequest {
            feature: EntityId::new(),
            body: stair.output.body,
            name: name.into(),
            definition: definition(STAIR_TYPE, stair)?,
            inputs: stair.inputs(),
            material,
        },
    )
}
pub fn prepare_stair_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    stair: &StairDefinition,
) -> Result<ModelPrepared, CommandError> {
    let current = read_stair(session.document(), feature)?;
    if current.output != stair.output {
        return Err(error("A stair edit keeps its output Part"));
    }
    stair.validate()?;
    resolve_stair(session.document(), session.parameters(), stair)?;
    edit(
        session,
        expected_revision,
        actor,
        "Edit stair",
        feature,
        definition(STAIR_TYPE, stair)?,
        stair.inputs(),
        stair.output.body,
    )
}
