//! Architectural walls with reusable wall and opening types. A wall is a
//! model feature owning a polyline path on a horizontal plane, a linked wall
//! type, explicit instance overrides and its door/window openings. Types are
//! document extensions; editing a type updates every wall that does not
//! override the changed value. Geometry here is pure 2D planning shared by the
//! native evaluator and plan symbols; the kernel builds the exact solid.
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

pub const WALL_TYPE: &str = "architecture.wall";
pub const WALL_SCHEMA: u32 = 1;
pub const WALL_CAPABILITY: &str = "architecture.wall.1";
pub const WALL_TYPE_KIND: &str = "architecture.wall_type";
pub const WALL_TYPE_CAPABILITY: &str = "architecture.wall_type.1";
pub const OPENING_TYPE_KIND: &str = "architecture.opening_type";
pub const OPENING_TYPE_CAPABILITY: &str = "architecture.opening_type.1";
/// Required by documents whose walls carry vertical cutouts, so a build that
/// does not know them refuses the file instead of misreading the wall.
pub const WALL_CUTOUT_CAPABILITY: &str = "architecture.wall_cutout.1";
pub const MAX_WALL_POINTS: usize = 256;
pub const MAX_WALL_OPENINGS: usize = 256;
/// A miter longer than this many wall thicknesses is reported, not built.
pub const MITER_LIMIT: f64 = 8.;

/// Which side of the authored path the wall occupies, looking along it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Justification {
    #[default]
    Center,
    Left,
    Right,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallTypeDefinition {
    pub thickness: Dimension,
    pub height: Dimension,
    #[serde(default)]
    pub justification: Justification,
    /// Material assigned to new walls of this type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<EntityId>,
}
/// Which jamb carries the hinge and to which side of the path a door opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hinge {
    Start,
    End,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Swing {
    Left,
    Right,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpeningKind {
    Door { hinge: Hinge, swing: Swing },
    Window,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpeningTypeDefinition {
    pub kind: OpeningKind,
    pub width: Dimension,
    pub height: Dimension,
    pub sill: Dimension,
}
/// One opening in one wall segment. `offset` runs along the segment from its
/// start point to the opening's start jamb. Unset overrides use the type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opening {
    pub id: EntityId,
    pub opening_type: EntityId,
    pub segment: usize,
    pub offset: Dimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sill: Option<Dimension>,
}
/// The wall face a partial-depth vertical cutout (a chase) is cut from,
/// looking along the segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallFace {
    #[default]
    Left,
    Right,
}
/// A full-height rectangular cutout in one wall segment, from the wall base
/// to its top. Without a depth it passes through the whole thickness (a
/// gap); with a depth it is a vertical chase cut into one face.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerticalCutout {
    pub id: EntityId,
    pub segment: usize,
    /// Along the segment from its start point to the cutout's start side.
    pub offset: Dimension,
    pub width: Dimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Dimension>,
    #[serde(default)]
    pub face: WallFace,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallDefinition {
    pub output: BodyOutput,
    pub wall_type: EntityId,
    /// Path vertices in world X/Y millimeters on the plane z = `base_z`.
    pub path: Vec<[Dimension; 2]>,
    #[serde(default)]
    pub closed: bool,
    /// Optional hosting level; `base_z` is then measured from its elevation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<EntityId>,
    pub base_z: Dimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<Justification>,
    #[serde(default)]
    pub openings: Vec<Opening>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<VerticalCutout>,
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
impl WallTypeDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        check_dimension(&self.thickness, Unit::Millimeter)?;
        check_dimension(&self.height, Unit::Millimeter)
    }
    fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(vec![&self.thickness, &self.height]);
        links.extend(self.material.map(|m| Link::new(m, "material")));
        links
    }
}
impl OpeningTypeDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        for d in [&self.width, &self.height, &self.sill] {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    fn inputs(&self) -> Vec<Link> {
        parameter_links(vec![&self.width, &self.height, &self.sill])
    }
}
impl WallDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() || self.wall_type.is_nil() {
            return Err(invalid("A wall needs an output Part and a wall type"));
        }
        let min = if self.closed { 3 } else { 2 };
        if self.path.len() < min || self.path.len() > MAX_WALL_POINTS {
            return Err(invalid(format!(
                "A {} wall path needs {min}–{MAX_WALL_POINTS} points",
                if self.closed { "closed" } else { "open" }
            )));
        }
        if self.openings.len() + self.cutouts.len() > MAX_WALL_OPENINGS {
            return Err(invalid("Too many wall openings"));
        }
        let segments = self.segment_count();
        let mut ids = BTreeSet::new();
        let placed = self
            .openings
            .iter()
            .map(|o| (o.id, o.segment, "Opening"))
            .chain(self.cutouts.iter().map(|c| (c.id, c.segment, "Cutout")));
        for (id, segment, what) in placed {
            if id.is_nil() || !ids.insert(id) {
                return Err(invalid("Wall openings need distinct identities"));
            }
            if segment >= segments {
                return Err(invalid(format!(
                    "{what} refers to segment {} but the wall has {segments}",
                    segment + 1
                )));
            }
        }
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    /// Whether a door, window or vertical cutout of this wall has `id`.
    pub fn has_opening(&self, id: EntityId) -> bool {
        self.openings.iter().any(|o| o.id == id) || self.cutouts.iter().any(|c| c.id == id)
    }
    pub fn segment_count(&self) -> usize {
        if self.closed {
            self.path.len()
        } else {
            self.path.len().saturating_sub(1)
        }
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut out: Vec<&Dimension> = self.path.iter().flatten().collect();
        out.push(&self.base_z);
        out.extend(self.thickness.iter().chain(&self.height));
        for o in &self.openings {
            out.push(&o.offset);
            out.extend(o.width.iter().chain(&o.height).chain(&o.sill));
        }
        for c in &self.cutouts {
            out.extend([&c.offset, &c.width]);
            out.extend(c.depth.iter());
        }
        out
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        definition(WALL_TYPE, self)
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        let value: Self = parse(d, WALL_TYPE)?;
        value.validate()?;
        Ok(value)
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(self.dimensions());
        links.push(Link::new(self.wall_type, "wall_type"));
        links.extend(self.level.map(|l| Link::new(l, "level")));
        let types: BTreeSet<_> = self.openings.iter().map(|o| o.opening_type).collect();
        links.extend(
            types
                .into_iter()
                .map(|t| Link::new(t, format!("opening_type/{t}"))),
        );
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
pub fn read_wall_type(
    document: &Document,
    id: EntityId,
) -> Result<WallTypeDefinition, DocumentError> {
    let value: WallTypeDefinition =
        parse(&extension(document, id, WALL_TYPE_KIND)?, WALL_TYPE_KIND)?;
    value.validate()?;
    if let Some(m) = value.material
        && !matches!(
            document.get(m).map(|e| &e.data),
            Some(EntityData::Material { .. })
        )
    {
        return Err(invalid("Wall type material is missing"));
    }
    Ok(value)
}
pub fn read_opening_type(
    document: &Document,
    id: EntityId,
) -> Result<OpeningTypeDefinition, DocumentError> {
    let value: OpeningTypeDefinition = parse(
        &extension(document, id, OPENING_TYPE_KIND)?,
        OPENING_TYPE_KIND,
    )?;
    value.validate()?;
    Ok(value)
}
pub fn wall_types(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| matches!(&e.data, EntityData::Extension{definition,..} if definition.type_id==WALL_TYPE_KIND))
}
pub fn opening_types(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| matches!(&e.data, EntityData::Extension{definition,..} if definition.type_id==OPENING_TYPE_KIND))
}
pub fn walls(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature{definition,..} if definition.type_id==WALL_TYPE),
    )
}
pub fn read_wall(document: &Document, feature: EntityId) -> Result<WallDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a wall feature"));
    };
    let wall = WallDefinition::from_definition(definition)?;
    let expected = wall.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid("Wall source and canonical input links disagree"));
    }
    if document.get(wall.wall_type).is_some() {
        read_wall_type(document, wall.wall_type)?;
    }
    for opening in &wall.openings {
        if document.get(opening.opening_type).is_some() {
            read_opening_type(document, opening.opening_type)?;
        }
    }
    if let Some(entity) = document.get(wall.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Wall output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("Wall output Part must contain its wall feature"));
        }
    }
    Ok(wall)
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for e in wall_types(document) {
        read_wall_type(document, e.id)?;
    }
    for e in opening_types(document) {
        read_opening_type(document, e.id)?;
    }
    for e in walls(document) {
        read_wall(document, e.id)?;
    }
    Ok(())
}

// ---- Resolution and planning ---------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedOpening {
    pub id: EntityId,
    pub kind: OpeningKind,
    pub segment: usize,
    pub offset_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub sill_mm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWallCutout {
    pub id: EntityId,
    pub segment: usize,
    pub offset_mm: f64,
    pub width_mm: f64,
    /// Depth into the wall from `face`; equal to the thickness when through.
    pub depth_mm: f64,
    pub face: WallFace,
    pub through: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWall {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
    pub base_z_mm: f64,
    pub thickness_mm: f64,
    pub height_mm: f64,
    pub justification: Justification,
    pub openings: Vec<ResolvedOpening>,
    pub cutouts: Vec<ResolvedWallCutout>,
}
pub fn resolve_wall(
    document: &Document,
    parameters: &EvaluatedParameters,
    wall: &WallDefinition,
) -> Result<ResolvedWall, DocumentError> {
    let ty = read_wall_type(document, wall.wall_type)?;
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let thickness_mm = mm(wall.thickness.as_ref().unwrap_or(&ty.thickness))?;
    let height_mm = mm(wall.height.as_ref().unwrap_or(&ty.height))?;
    if thickness_mm <= 1e-3 || height_mm <= 1e-3 {
        return Err(invalid("Wall thickness and height must be positive"));
    }
    let points = wall
        .path
        .iter()
        .map(|[x, y]| Ok([mm(x)?, mm(y)?]))
        .collect::<Result<Vec<_>, DocumentError>>()?;
    let mut openings = vec![];
    for o in &wall.openings {
        let t = read_opening_type(document, o.opening_type)?;
        openings.push(ResolvedOpening {
            id: o.id,
            kind: t.kind,
            segment: o.segment,
            offset_mm: mm(&o.offset)?,
            width_mm: mm(o.width.as_ref().unwrap_or(&t.width))?,
            height_mm: mm(o.height.as_ref().unwrap_or(&t.height))?,
            sill_mm: mm(o.sill.as_ref().unwrap_or(&t.sill))?,
        });
    }
    let mut cutouts = vec![];
    for c in &wall.cutouts {
        let depth_mm = match &c.depth {
            Some(d) => mm(d)?,
            None => thickness_mm,
        };
        cutouts.push(ResolvedWallCutout {
            id: c.id,
            segment: c.segment,
            offset_mm: mm(&c.offset)?,
            width_mm: mm(&c.width)?,
            depth_mm,
            face: c.face,
            through: c.depth.is_none() || depth_mm >= thickness_mm - 1e-6,
        });
    }
    let level_z = match wall.level {
        Some(level) => super::resolve_level(document, parameters, level)?.elevation_mm,
        None => 0.,
    };
    Ok(ResolvedWall {
        points,
        closed: wall.closed,
        base_z_mm: level_z + mm(&wall.base_z)?,
        thickness_mm,
        height_mm,
        justification: wall.justification.unwrap_or(ty.justification),
        openings,
        cutouts,
    })
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn scale(a: [f64; 2], s: f64) -> [f64; 2] {
    [a[0] * s, a[1] * s]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn length(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

/// One straight wall segment: start, unit direction, left normal, length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start: [f64; 2],
    pub direction: [f64; 2],
    pub normal: [f64; 2],
    pub length_mm: f64,
}
/// A closed loop of the wall outline with a stable role per edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub points: Vec<[f64; 2]>,
    pub roles: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WallPlan {
    pub segments: Vec<Segment>,
    /// Signed offsets of the left and right faces from the path.
    pub left_mm: f64,
    pub right_mm: f64,
    pub outer: Loop,
    pub holes: Vec<Loop>,
}
fn signed_area(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|i| cross(points[i], points[(i + 1) % points.len()]))
        .sum::<f64>()
        / 2.
}
fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (r, s) = (sub(b, a), sub(d, c));
    let den = cross(r, s);
    if den.abs() < 1e-12 {
        return false;
    }
    let t = cross(sub(c, a), s) / den;
    let u = cross(sub(c, a), r) / den;
    t > 1e-9 && t < 1. - 1e-9 && u > 1e-9 && u < 1. - 1e-9
}
fn simple(points: &[[f64; 2]]) -> bool {
    let n = points.len();
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if segments_intersect(
                points[i],
                points[(i + 1) % n],
                points[j],
                points[(j + 1) % n],
            ) {
                return false;
            }
        }
    }
    true
}

impl ResolvedWall {
    pub fn segments(&self) -> Result<Vec<Segment>, DocumentError> {
        let n = self.points.len();
        let count = if self.closed { n } else { n - 1 };
        (0..count)
            .map(|i| {
                let (a, b) = (self.points[i], self.points[(i + 1) % n]);
                let d = sub(b, a);
                let l = length(d);
                if l < 1e-3 {
                    return Err(invalid(format!("Wall segment {} has no length", i + 1)));
                }
                let dir = scale(d, 1. / l);
                Ok(Segment {
                    start: a,
                    direction: dir,
                    normal: [-dir[1], dir[0]],
                    length_mm: l,
                })
            })
            .collect()
    }
    fn offsets(&self) -> (f64, f64) {
        let t = self.thickness_mm;
        match self.justification {
            Justification::Center => (t / 2., -t / 2.),
            Justification::Left => (t, 0.),
            Justification::Right => (0., -t),
        }
    }
    /// The vertex where the offset lines of segments `a` and `b` meet.
    fn join(
        a: &Segment,
        b: &Segment,
        offset: f64,
        thickness: f64,
        index: usize,
    ) -> Result<[f64; 2], DocumentError> {
        let pa = add(a.start, scale(a.normal, offset));
        let pb = add(b.start, scale(b.normal, offset));
        let den = cross(a.direction, b.direction);
        if den.abs() < 1e-9 {
            if a.direction[0] * b.direction[0] + a.direction[1] * b.direction[1] < 0. {
                return Err(invalid(format!(
                    "Wall path reverses at point {}; split it into separate walls",
                    index + 1
                )));
            }
            return Ok(pb);
        }
        let t = cross(sub(pb, pa), b.direction) / den;
        let p = add(pa, scale(a.direction, t));
        let corner = add(b.start, scale(b.normal, offset));
        if length(sub(p, corner)) > MITER_LIMIT * thickness {
            return Err(invalid(format!(
                "Wall joint at point {} is too sharp to miter; widen the angle",
                index + 1
            )));
        }
        Ok(p)
    }
    /// The closed outline of the wall in plan, with stable edge roles.
    pub fn plan(&self) -> Result<WallPlan, DocumentError> {
        let segments = self.segments()?;
        let (left, right) = self.offsets();
        let n = segments.len();
        let side = |offset: f64| -> Result<Vec<[f64; 2]>, DocumentError> {
            if self.closed {
                (0..n)
                    .map(|i| {
                        Self::join(
                            &segments[(i + n - 1) % n],
                            &segments[i],
                            offset,
                            self.thickness_mm,
                            i,
                        )
                    })
                    .collect()
            } else {
                let mut pts = vec![add(segments[0].start, scale(segments[0].normal, offset))];
                for i in 1..n {
                    pts.push(Self::join(
                        &segments[i - 1],
                        &segments[i],
                        offset,
                        self.thickness_mm,
                        i,
                    )?);
                }
                let last = segments[n - 1];
                pts.push(add(
                    add(last.start, scale(last.direction, last.length_mm)),
                    scale(last.normal, offset),
                ));
                Ok(pts)
            }
        };
        let (lp, rp) = (side(left)?, side(right)?);
        let (outer, holes) = if self.closed {
            let role = |s: &str| {
                (0..n)
                    .map(|i| format!("wall.seg.{i}.{s}"))
                    .collect::<Vec<_>>()
            };
            let l = Loop {
                points: lp,
                roles: role("left"),
            };
            let r = Loop {
                points: rp,
                roles: role("right"),
            };
            let (o, h) = if signed_area(&l.points).abs() >= signed_area(&r.points).abs() {
                (l, r)
            } else {
                (r, l)
            };
            for lp in [&o, &h] {
                if !simple(&lp.points) || signed_area(&lp.points).abs() < 1e-6 {
                    return Err(invalid(
                        "Closed wall outline self-intersects; widen the loop or thin the wall",
                    ));
                }
            }
            (o, vec![h])
        } else {
            let mut points = lp.clone();
            let mut roles: Vec<String> = (0..n).map(|i| format!("wall.seg.{i}.left")).collect();
            roles.push("wall.end".into());
            points.extend(rp.iter().rev().copied());
            roles.extend((0..n).rev().map(|i| format!("wall.seg.{i}.right")));
            roles.push("wall.start".into());
            if !simple(&points) {
                return Err(invalid(
                    "Wall outline self-intersects; the path crosses or doubles back on itself",
                ));
            }
            (Loop { points, roles }, vec![])
        };
        Ok(WallPlan {
            segments,
            left_mm: left,
            right_mm: right,
            outer,
            holes,
        })
    }
    /// Check every opening against its segment and wall; joints keep one
    /// thickness of solid wall on each side.
    pub fn check_openings(&self, plan: &WallPlan) -> Result<(), DocumentError> {
        let n = plan.segments.len();
        let mut spans: Vec<(usize, f64, f64)> = vec![];
        for (k, o) in self.openings.iter().enumerate() {
            let seg = plan.segments[o.segment];
            let label = format!("Opening {}", k + 1);
            if o.width_mm <= 1e-3 || o.height_mm <= 1e-3 || o.sill_mm < 0. {
                return Err(invalid(format!(
                    "{label} needs a positive width and height and a non-negative sill"
                )));
            }
            let start_margin = if self.closed || o.segment > 0 {
                self.thickness_mm
            } else {
                0.
            };
            let end_margin = if self.closed || o.segment + 1 < n {
                self.thickness_mm
            } else {
                0.
            };
            if o.offset_mm < start_margin - 1e-9
                || o.offset_mm + o.width_mm > seg.length_mm - end_margin + 1e-9
            {
                return Err(invalid(format!(
                    "{label} does not fit segment {} ({:.1} mm long, keeping {:.1} mm of wall at joints)",
                    o.segment + 1,
                    seg.length_mm,
                    self.thickness_mm
                )));
            }
            if o.sill_mm + o.height_mm >= self.height_mm - 1e-6 {
                return Err(invalid(format!(
                    "{label} reaches the wall top; lower it or raise the wall so material remains above"
                )));
            }
            for (s, a, b) in &spans {
                if *s == o.segment
                    && o.offset_mm < *b - 1e-9
                    && o.offset_mm + o.width_mm > *a + 1e-9
                {
                    return Err(invalid(format!("{label} overlaps another opening")));
                }
            }
            spans.push((o.segment, o.offset_mm, o.offset_mm + o.width_mm));
        }
        self.check_cutouts(plan, &mut spans)
    }
    /// Vertical cutouts run from the base to the top, so they must fit their
    /// segment like an opening, overlap nothing, and leave one connected wall.
    fn check_cutouts(
        &self,
        plan: &WallPlan,
        spans: &mut Vec<(usize, f64, f64)>,
    ) -> Result<(), DocumentError> {
        let n = plan.segments.len();
        let mut interior_through = 0;
        for (k, c) in self.cutouts.iter().enumerate() {
            let seg = plan.segments[c.segment];
            let label = format!("Vertical cutout {}", k + 1);
            if c.width_mm <= 1e-3 || c.depth_mm <= 1e-3 {
                return Err(invalid(format!("{label} needs a positive width and depth")));
            }
            if c.depth_mm > self.thickness_mm + 1e-6 {
                return Err(invalid(format!(
                    "{label} is {:.1} mm deep but the wall is {:.1} mm thick; leave the depth empty to cut through",
                    c.depth_mm, self.thickness_mm
                )));
            }
            let at_start = !self.closed && c.segment == 0;
            let at_end = !self.closed && c.segment + 1 == n;
            let start_margin = if at_start { 0. } else { self.thickness_mm };
            let end_margin = if at_end { 0. } else { self.thickness_mm };
            let (a, b) = (c.offset_mm, c.offset_mm + c.width_mm);
            if a < start_margin - 1e-9 || b > seg.length_mm - end_margin + 1e-9 {
                return Err(invalid(format!(
                    "{label} does not fit segment {} ({:.1} mm long, keeping {:.1} mm of wall at joints)",
                    c.segment + 1,
                    seg.length_mm,
                    self.thickness_mm
                )));
            }
            for (s, sa, sb) in spans.iter() {
                if *s == c.segment && a < *sb - 1e-9 && b > *sa + 1e-9 {
                    return Err(invalid(format!(
                        "{label} overlaps another opening or cutout"
                    )));
                }
            }
            spans.push((c.segment, a, b));
            if c.through {
                let touches_start = at_start && a <= 1e-9;
                let touches_end = at_end && b >= seg.length_mm - 1e-9;
                if touches_start && touches_end {
                    return Err(invalid(format!("{label} removes the whole wall")));
                }
                if !touches_start && !touches_end {
                    interior_through += 1;
                }
            }
        }
        // An open wall falls apart at its first interior through-cut; a closed
        // room stays one piece with a single gap.
        let allowed = usize::from(self.closed);
        if interior_through > allowed {
            return Err(invalid(if self.closed {
                "Two full-depth vertical cutouts split this room wall into separate pieces; give one a chase depth or draw separate walls"
            } else {
                "A full-depth vertical cutout inside an open wall splits it into two pieces; give it a chase depth, move it to a wall end or draw two walls"
            }));
        }
        Ok(())
    }
    /// The plan rectangle of a vertical cutout: along-segment span and
    /// across-wall span (signed offsets from the path, low then high).
    pub fn cutout_span(
        &self,
        plan: &WallPlan,
        cutout: &ResolvedWallCutout,
    ) -> ([f64; 2], [f64; 2]) {
        let along = [cutout.offset_mm, cutout.offset_mm + cutout.width_mm];
        let across = if cutout.through {
            [plan.right_mm, plan.left_mm]
        } else {
            match cutout.face {
                WallFace::Left => [plan.left_mm - cutout.depth_mm, plan.left_mm],
                WallFace::Right => [plan.right_mm, plan.right_mm + cutout.depth_mm],
            }
        };
        (along, across)
    }
    /// Plan symbols in world X/Y: door leaves and swing arcs, window panes.
    pub fn plan_symbols(&self, plan: &WallPlan) -> PlanSymbols {
        let mut out = PlanSymbols::default();
        for o in &self.openings {
            let seg = plan.segments[o.segment];
            let at = |along: f64, across: f64| {
                add(
                    add(seg.start, scale(seg.direction, along)),
                    scale(seg.normal, across),
                )
            };
            let (a, b) = (o.offset_mm, o.offset_mm + o.width_mm);
            match &o.kind {
                OpeningKind::Door { hinge, swing } => {
                    let face = match swing {
                        Swing::Left => plan.left_mm,
                        Swing::Right => plan.right_mm,
                    };
                    let sign = if matches!(swing, Swing::Left) {
                        1.
                    } else {
                        -1.
                    };
                    let (pivot_along, free_along) = match hinge {
                        Hinge::Start => (a, b),
                        Hinge::End => (b, a),
                    };
                    let pivot = at(pivot_along, face);
                    let open = at(pivot_along, face + sign * o.width_mm);
                    let closed = at(free_along, face);
                    out.lines.push([pivot, open]);
                    let angle = |p: [f64; 2]| (p[1] - pivot[1]).atan2(p[0] - pivot[0]);
                    let (s, mut e) = (angle(closed), angle(open));
                    // The swing is the quarter turn between closed and open.
                    while e - s > std::f64::consts::PI {
                        e -= std::f64::consts::TAU;
                    }
                    while s - e > std::f64::consts::PI {
                        e += std::f64::consts::TAU;
                    }
                    out.arcs.push((pivot, o.width_mm, s.min(e), s.max(e)));
                }
                OpeningKind::Window => {
                    let mid = (plan.left_mm + plan.right_mm) / 2.;
                    let gap = (plan.left_mm - plan.right_mm).abs() / 6.;
                    for across in [mid - gap, mid + gap] {
                        out.lines.push([at(a, across), at(b, across)]);
                    }
                    for along in [a, b] {
                        out.lines
                            .push([at(along, plan.left_mm), at(along, plan.right_mm)]);
                    }
                }
            }
        }
        out
    }
}
/// Plan graphics in world X/Y millimeters. Arcs are (center, radius, start
/// rad, end rad), counterclockwise.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanSymbols {
    pub lines: Vec<[[f64; 2]; 2]>,
    pub arcs: Vec<([f64; 2], f64, f64, f64)>,
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
        return Err(error("Type name must contain 1–256 bytes"));
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
pub fn prepare_wall_type(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    value: &WallTypeDefinition,
) -> Result<(EntityId, Prepared), CommandError> {
    value.validate()?;
    insert_extension(
        session,
        expected_revision,
        actor,
        "Create wall type",
        name,
        definition(WALL_TYPE_KIND, value)?,
        value.inputs(),
    )
}
pub fn prepare_opening_type(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    value: &OpeningTypeDefinition,
) -> Result<(EntityId, Prepared), CommandError> {
    value.validate()?;
    insert_extension(
        session,
        expected_revision,
        actor,
        "Create opening type",
        name,
        definition(OPENING_TYPE_KIND, value)?,
        value.inputs(),
    )
}
/// Edit a wall type. Every wall of this type follows unless it overrides the
/// changed value; walls still using the previous type material follow a
/// material change, and walls with another material keep it.
pub fn prepare_wall_type_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    value: &WallTypeDefinition,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    value.validate()?;
    let document = session.document();
    let previous = read_wall_type(document, id)?;
    let mut entity = document
        .get(id)
        .ok_or_else(|| error("Wall type is missing"))?
        .clone();
    entity.data = EntityData::Extension {
        definition: definition(WALL_TYPE_KIND, value)?,
        inputs: value.inputs(),
    };
    let mut edits = vec![Mutation::Replace(entity)];
    if previous.material != value.material {
        for wall in walls(document) {
            let w = read_wall(document, wall.id)?;
            if w.wall_type != id {
                continue;
            }
            if let Some(mut part) = document.get(w.output.body).cloned()
                && let EntityData::Part { material, .. } = &mut part.data
                && *material == previous.material
            {
                *material = value.material;
                edits.push(Mutation::Replace(part));
            }
        }
    }
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit wall type".into(),
        edits,
    })
}
pub fn prepare_opening_type_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    value: &OpeningTypeDefinition,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    value.validate()?;
    read_opening_type(session.document(), id)?;
    let mut entity = session
        .document()
        .get(id)
        .ok_or_else(|| error("Opening type is missing"))?
        .clone();
    entity.data = EntityData::Extension {
        definition: definition(OPENING_TYPE_KIND, value)?,
        inputs: value.inputs(),
    };
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit opening type".into(),
        edits: vec![Mutation::Replace(entity)],
    })
}
pub struct WallRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub wall: WallDefinition,
}
impl WallRequest {
    pub fn new(name: impl Into<String>, mut wall: WallDefinition) -> Self {
        let body = EntityId::new();
        wall.output.body = body;
        Self {
            feature: EntityId::new(),
            body,
            name: name.into(),
            wall,
        }
    }
}
/// Create a wall Part carrying its type's material. The source is checked
/// against its types and parameters; a native evaluator builds the solid.
pub fn prepare_wall(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &WallRequest,
) -> Result<ModelPrepared, CommandError> {
    prepare_wall_change(
        session,
        expected_revision,
        actor,
        &[],
        &WallChange::Create(request),
    )
}
pub fn prepare_wall_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    wall: &WallDefinition,
) -> Result<ModelPrepared, CommandError> {
    prepare_wall_change(
        session,
        expected_revision,
        actor,
        &[],
        &WallChange::Edit { feature, wall },
    )
}

/// A wall, door or window type created in the same transaction as a wall.
#[derive(Clone, Debug, PartialEq)]
pub enum NewTypeKind {
    Wall(WallTypeDefinition),
    Opening(OpeningTypeDefinition),
}
#[derive(Clone, Debug, PartialEq)]
pub struct NewType {
    pub id: EntityId,
    pub name: String,
    pub kind: NewTypeKind,
}
impl NewType {
    pub fn new(name: impl Into<String>, kind: NewTypeKind) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            kind,
        }
    }
    pub(super) fn entity(&self) -> Result<Entity, CommandError> {
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(error("Type name must contain 1–256 bytes"));
        }
        let data = match &self.kind {
            NewTypeKind::Wall(v) => {
                v.validate()?;
                EntityData::Extension {
                    definition: definition(WALL_TYPE_KIND, v)?,
                    inputs: v.inputs(),
                }
            }
            NewTypeKind::Opening(v) => {
                v.validate()?;
                EntityData::Extension {
                    definition: definition(OPENING_TYPE_KIND, v)?,
                    inputs: v.inputs(),
                }
            }
        };
        let mut entity = Entity::new(self.name.clone(), data);
        entity.id = self.id;
        Ok(entity)
    }
}
pub enum WallChange<'a> {
    Create(&'a WallRequest),
    Edit {
        feature: EntityId,
        wall: &'a WallDefinition,
    },
}
/// Create or edit a wall together with any new types it uses, as one
/// transaction. The wall is planned against the candidate including the new
/// types; a native evaluator still builds and checks the solid.
pub fn prepare_wall_change(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    new_types: &[NewType],
    change: &WallChange<'_>,
) -> Result<ModelPrepared, CommandError> {
    prepare_wall_change_labeled(session, expected_revision, actor, new_types, change, None)
}
/// As [`prepare_wall_change`], with an optional history label replacing
/// "Create wall" or "Edit wall".
pub(super) fn prepare_wall_change_labeled(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    new_types: &[NewType],
    change: &WallChange<'_>,
    history_label: Option<&str>,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut edits = vec![];
    for t in new_types {
        if session.document().get(t.id).is_some() {
            return Err(error("New type identities must be new"));
        }
        edits.push(Mutation::Insert(t.entity()?));
    }
    let candidate = session.document().edited(&edits)?;
    let parameters = crate::parameters::evaluate(&candidate)?;
    let (wall, output, label) = match change {
        WallChange::Create(request) => {
            if request.wall.output.body != request.body {
                return Err(error("Wall output must name the requested Part"));
            }
            (
                &request.wall,
                CreatedBody {
                    feature: request.feature,
                    body: request.body,
                },
                "Create wall",
            )
        }
        WallChange::Edit { feature, wall } => {
            if read_wall(&candidate, *feature)?.output != wall.output {
                return Err(error("A wall edit keeps its output Part"));
            }
            (
                *wall,
                CreatedBody {
                    feature: *feature,
                    body: wall.output.body,
                },
                "Edit wall",
            )
        }
    };
    // Validate first: planning indexes segments by the openings' numbers.
    wall.validate()?;
    let resolved = resolve_wall(&candidate, &parameters, wall)?;
    resolved.check_openings(&resolved.plan()?)?;
    match change {
        WallChange::Create(request) => {
            for id in [request.feature, request.body] {
                if id.is_nil() || candidate.get(id).is_some() || request.feature == request.body {
                    return Err(error("Wall identities must be new, distinct and non-nil"));
                }
            }
            if request.name.trim().is_empty() || request.name.len() > 1000 {
                return Err(error("Wall name must contain 1–1000 bytes"));
            }
            let mut feature = Entity::new(
                format!("{} feature", request.name),
                EntityData::Feature {
                    definition: wall.definition()?,
                    inputs: wall.inputs(),
                    suppressed: false,
                },
            );
            feature.id = request.feature;
            let mut body = Entity::new(
                request.name.clone(),
                EntityData::Part {
                    features: vec![request.feature],
                    material: read_wall_type(&candidate, wall.wall_type)?.material,
                    tags: BTreeSet::new(),
                },
            );
            body.id = request.body;
            edits.extend([Mutation::Insert(feature), Mutation::Insert(body)]);
        }
        WallChange::Edit { feature, .. } => {
            let mut entity = candidate
                .get(*feature)
                .ok_or_else(|| error("Wall is missing"))?
                .clone();
            entity.data = EntityData::Feature {
                definition: wall.definition()?,
                inputs: wall.inputs(),
                suppressed: false,
            };
            edits.push(Mutation::Replace(entity));
        }
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: history_label.unwrap_or(label).into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![output],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wall(points: &[[f64; 2]], closed: bool, j: Justification) -> ResolvedWall {
        ResolvedWall {
            points: points.to_vec(),
            closed,
            base_z_mm: 0.,
            thickness_mm: 200.,
            height_mm: 2500.,
            justification: j,
            openings: vec![],
            cutouts: vec![],
        }
    }
    fn area(l: &Loop) -> f64 {
        signed_area(&l.points).abs()
    }
    #[test]
    fn open_l_shaped_wall_has_a_mitered_outline_with_roles() {
        let w = wall(
            &[[0., 0.], [4000., 0.], [4000., 3000.]],
            false,
            Justification::Center,
        );
        let plan = w.plan().unwrap();
        assert_eq!(plan.outer.points.len(), 6);
        assert_eq!(plan.outer.roles.len(), 6);
        assert!(plan.outer.roles.contains(&"wall.start".to_string()));
        // Centerline length 7000 × 200 thickness; a centered miter keeps the area.
        assert!(
            (area(&plan.outer) - 7000. * 200.).abs() < 1e-6,
            "{}",
            area(&plan.outer)
        );
        assert!(plan.outer.points.contains(&[4100., -100.]));
        assert!(plan.outer.points.contains(&[3900., 100.]));
    }
    #[test]
    fn closed_room_has_outer_and_inner_loops_for_any_winding() {
        for pts in [
            vec![[0., 0.], [5000., 0.], [5000., 4000.], [0., 4000.]],
            vec![[0., 0.], [0., 4000.], [5000., 4000.], [5000., 0.]],
        ] {
            let plan = wall(&pts, true, Justification::Center).plan().unwrap();
            assert!((area(&plan.outer) - 5200. * 4200.).abs() < 1e-6);
            assert!((area(&plan.holes[0]) - 4800. * 3800.).abs() < 1e-6);
        }
        // Left justification on a counterclockwise room builds inward.
        let plan = wall(
            &[[0., 0.], [5000., 0.], [5000., 4000.], [0., 4000.]],
            true,
            Justification::Left,
        )
        .plan()
        .unwrap();
        assert!((area(&plan.outer) - 5000. * 4000.).abs() < 1e-6);
    }
    #[test]
    fn unsupported_paths_are_diagnosed() {
        let reverse = wall(
            &[[0., 0.], [1000., 0.], [0., 0.]],
            false,
            Justification::Center,
        );
        assert!(reverse.plan().unwrap_err().to_string().contains("reverses"));
        let sharp = wall(
            &[[0., 0.], [3000., 0.], [0., 100.]],
            false,
            Justification::Center,
        );
        assert!(sharp.plan().unwrap_err().to_string().contains("too sharp"));
        let crossing = wall(
            &[[0., 0.], [3000., 0.], [3000., 3000.], [1500., -2000.]],
            false,
            Justification::Center,
        );
        assert!(
            crossing
                .plan()
                .unwrap_err()
                .to_string()
                .contains("self-intersects")
        );
        let zero = wall(&[[0., 0.], [0., 0.]], false, Justification::Center);
        assert!(zero.plan().is_err());
    }
    #[test]
    fn a_wall_and_its_new_types_commit_as_one_transaction() {
        let session = Session::new(Document::new("Types").unwrap()).unwrap();
        let ty = NewType::new(
            "Stud 100",
            NewTypeKind::Wall(WallTypeDefinition {
                thickness: super::super::length(100.),
                height: super::super::length(2400.),
                justification: Justification::Center,
                material: None,
            }),
        );
        let door = NewType::new(
            "Door",
            NewTypeKind::Opening(OpeningTypeDefinition {
                kind: OpeningKind::Door {
                    hinge: Hinge::End,
                    swing: Swing::Right,
                },
                width: super::super::length(800.),
                height: super::super::length(2000.),
                sill: super::super::length(0.),
            }),
        );
        let wall = WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty.id,
            path: vec![
                [super::super::length(0.), super::super::length(0.)],
                [super::super::length(3000.), super::super::length(0.)],
            ],
            closed: false,
            level: None,
            base_z: super::super::length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![Opening {
                id: EntityId::new(),
                opening_type: door.id,
                segment: 0,
                offset: super::super::length(1000.),
                width: None,
                height: None,
                sill: None,
            }],
            cutouts: vec![],
        };
        let request = WallRequest::new("Partition", wall);
        let prepared = prepare_wall_change(
            &session,
            0,
            crate::commands::Actor::local(crate::commands::Channel::Ui),
            &[ty.clone(), door.clone()],
            &WallChange::Create(&request),
        )
        .unwrap();
        let doc = prepared.prepared.document();
        assert!(read_wall_type(doc, ty.id).is_ok() && read_opening_type(doc, door.id).is_ok());
        assert_eq!(walls(doc).count(), 1);
        // An opening that does not fit is refused before anything is staged.
        let mut bad = read_wall(doc, prepared.outputs[0].feature).unwrap();
        bad.openings[0].offset = super::super::length(2500.);
        let mut s = session;
        s.commit(prepared.prepared).unwrap();
        let feature = walls(s.document()).next().unwrap().id;
        assert!(
            prepare_wall_change(
                &s,
                1,
                crate::commands::Actor::local(crate::commands::Channel::Ui),
                &[],
                &WallChange::Edit {
                    feature,
                    wall: &bad
                }
            )
            .is_err()
        );
    }
    #[test]
    fn openings_must_fit_their_segment_and_leave_wall_above() {
        let mut w = wall(
            &[[0., 0.], [4000., 0.], [4000., 3000.]],
            false,
            Justification::Center,
        );
        let door = |offset: f64, width: f64, height: f64| ResolvedOpening {
            id: EntityId::new(),
            kind: OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Left,
            },
            segment: 0,
            offset_mm: offset,
            width_mm: width,
            height_mm: height,
            sill_mm: 0.,
        };
        w.openings = vec![door(500., 900., 2100.)];
        let plan = w.plan().unwrap();
        w.check_openings(&plan).unwrap();
        w.openings = vec![door(3500., 900., 2100.)];
        assert!(
            w.check_openings(&plan)
                .unwrap_err()
                .to_string()
                .contains("does not fit")
        );
        w.openings = vec![door(500., 900., 2500.)];
        assert!(
            w.check_openings(&plan)
                .unwrap_err()
                .to_string()
                .contains("wall top")
        );
        w.openings = vec![door(500., 900., 2100.), door(1000., 900., 2100.)];
        assert!(
            w.check_openings(&plan)
                .unwrap_err()
                .to_string()
                .contains("overlaps")
        );
        // A door swing is a quarter arc at the hinge with the leaf open.
        w.openings = vec![door(500., 900., 2100.)];
        let symbols = w.plan_symbols(&plan);
        assert_eq!(symbols.lines, vec![[[500., 100.], [500., 1000.]]]);
        let (c, r, s, e) = symbols.arcs[0];
        assert_eq!((c, r), ([500., 100.], 900.));
        assert!((e - s - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }
}
