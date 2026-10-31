//! Sheet-metal parts. A sheet part is a model feature producing one Part from
//! sheet stock: a base sheet (a rectangle on a principal plane, a sketch
//! region, or a contour captured from a converted constant-thickness solid)
//! of the stock thickness. The base face is the stationary face; the folded
//! solid and its developed flat pattern are two evaluations of the same
//! feature, so they share one Part and feature identity.
//!
//! Thickness comes from the stock. The inside bend radius and K-factor come
//! from the stock unless the part overrides them; both values stay separate
//! editable inputs and are never universal constants.
mod edges;
mod hardware;
mod prepare;
mod resolve;
pub use edges::*;
pub use hardware::*;
pub use prepare::*;
pub use resolve::*;

use super::{
    cabinetry::PanelPlane,
    common::parameter_links,
    source::{BodyOutput, ProfileSource, check_dimension, invalid, validate_region},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link},
    manufacturing::PlanarLoop,
    materials::{self, LibraryDefinition, Stock, StockKind},
    parameters::Unit,
};
use serde::{Deserialize, Serialize};

pub const SHEET_TYPE: &str = "sheet_metal.part";
pub const SHEET_SCHEMA: u32 = 1;
pub const SHEET_CAPABILITY: &str = "sheet_metal.part.1";
/// Captured contour budget for converted bases.
pub const MAX_CONTOUR_LOOPS: usize = 64;
pub const MAX_CONTOUR_EDGES: usize = 4096;
pub const MAX_FLANGES: usize = 64;
pub const MAX_CUTOUTS: usize = 256;

/// A captured plane: `u` and `v` are orthonormal; the sheet thickness runs
/// along u × v from the base face.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetFrame {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
}
impl SheetFrame {
    pub fn normal(&self) -> [f64; 3] {
        resolve::cross(self.u, self.v)
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let finite = self
            .origin
            .iter()
            .chain(&self.u)
            .chain(&self.v)
            .all(|v| v.is_finite() && v.abs() <= 1e9);
        if !finite
            || (dot(self.u, self.u) - 1.).abs() > 1e-9
            || (dot(self.v, self.v) - 1.).abs() > 1e-9
            || dot(self.u, self.v).abs() > 1e-9
        {
            return Err(invalid("A sheet frame needs finite orthonormal axes"));
        }
        Ok(())
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SheetBase {
    /// A rectangle on a principal plane with its minimum corner at `origin`.
    Rectangle {
        plane: PanelPlane,
        origin: [Dimension; 3],
        length: Dimension,
        width: Dimension,
    },
    /// A solved sketch region; the base face lies on the sketch plane.
    Profile { profile: ProfileSource },
    /// Exact base loops captured from a converted constant-thickness solid,
    /// in `frame` coordinates. The source Part does not drive the sheet.
    Contour {
        frame: SheetFrame,
        loops: Vec<PlanarLoop>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        converted_from: Option<EntityId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetDefinition {
    pub output: BodyOutput,
    /// Sheet stock: material, thickness and default bend values.
    pub stock: EntityId,
    pub base: SheetBase,
    /// Material on the other side of the base face: thickness along −(u × v).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reverse: bool,
    /// Inside bend radius for this part; the stock value when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_radius: Option<Dimension>,
    /// Neutral-axis position as a fraction of thickness (ratio); the stock
    /// value when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_factor: Option<Dimension>,
    /// Flanges, hems and jogs on straight edges of the base outline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flanges: Vec<Flange>,
    /// Through-cuts in the base or a flange leg, in that face's coordinates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<Cutout>,
    /// Mounting patterns of attached hardware; each cuts receiving holes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fasteners: Vec<MountingPattern>,
}

/// Which way a flange bends from the base: `Up` toward the material side
/// (along the thickness direction), `Down` away from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BendDirection {
    #[default]
    Up,
    Down,
}
/// The forming intent of a flange. Each kind keeps its own editable values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlangeKind {
    /// One bend of `angle` (0–180°, exclusive of 0) followed by a straight leg.
    Bend { angle: Dimension },
    /// A 180° fold whose leg lies `gap` from the base face; the inside
    /// radius is half the gap.
    Hem { gap: Dimension },
    /// Two opposite bends of `angle` that offset the leg by `offset`,
    /// measured between the base face and the leg's matching face.
    Jog { angle: Dimension, offset: Dimension },
}
/// A rectangular relief cut into the base at each inset end of a flange:
/// `width` along the edge beyond the flange end, `depth` into the base.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relief {
    pub width: Dimension,
    pub depth: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flange {
    /// Index of a straight edge in the base outer loop. Rectangle edges are
    /// 0: v = 0, 1: u = length, 2: v = width, 3: u = 0.
    pub edge: u32,
    pub kind: FlangeKind,
    #[serde(default)]
    pub direction: BendDirection,
    /// Straight leg beyond the last bend.
    pub length: Dimension,
    /// Inside radius for this flange's bends; the part value when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_radius: Option<Dimension>,
    /// Gaps from the edge's start and end to the flange, for corners and
    /// seams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_inset: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_inset: Option<Dimension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relief: Option<Relief>,
}
impl Flange {
    fn lengths(&self) -> Vec<&Dimension> {
        let mut out = vec![&self.length];
        match &self.kind {
            FlangeKind::Hem { gap } => out.push(gap),
            FlangeKind::Jog { offset, .. } => out.push(offset),
            FlangeKind::Bend { .. } => {}
        }
        out.extend(&self.inside_radius);
        out.extend(&self.start_inset);
        out.extend(&self.end_inset);
        if let Some(r) = &self.relief {
            out.extend([&r.width, &r.depth]);
        }
        out
    }
    fn angles(&self) -> Vec<&Dimension> {
        match &self.kind {
            FlangeKind::Bend { angle } | FlangeKind::Jog { angle, .. } => vec![angle],
            FlangeKind::Hem { .. } => vec![],
        }
    }
}
/// The face a cutout goes through. Base coordinates are the base frame's
/// (u, v); flange coordinates run along the edge from the flange start (x)
/// and along the leg from the end of its last bend (y).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CutoutFace {
    Base,
    Flange { index: u32 },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum CutoutShape {
    Circle {
        center: [Dimension; 2],
        diameter: Dimension,
    },
    Rectangle {
        center: [Dimension; 2],
        size: [Dimension; 2],
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cutout {
    pub face: CutoutFace,
    pub shape: CutoutShape,
}
impl Cutout {
    fn lengths(&self) -> Vec<&Dimension> {
        match &self.shape {
            CutoutShape::Circle { center, diameter } => vec![&center[0], &center[1], diameter],
            CutoutShape::Rectangle { center, size } => {
                vec![&center[0], &center[1], &size[0], &size[1]]
            }
        }
    }
}

pub(super) fn definition<T: Serialize>(
    type_id: &str,
    data: &T,
) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: type_id.into(),
        schema_version: SHEET_SCHEMA,
        data: serde_json::to_value(data).map_err(|e| invalid(e.to_string()))?,
    })
}

fn validate_loops(loops: &[PlanarLoop]) -> Result<(), DocumentError> {
    if loops.is_empty()
        || loops.len() > MAX_CONTOUR_LOOPS
        || loops.iter().filter(|l| l.outer).count() != 1
        || !loops[0].outer
        || loops.iter().map(|l| l.edges.len()).sum::<usize>() > MAX_CONTOUR_EDGES
    {
        return Err(invalid(
            "A captured sheet contour needs one outer loop first and bounded holes",
        ));
    }
    for l in loops {
        if l.edges.is_empty() {
            return Err(invalid("Captured sheet loops cannot be empty"));
        }
        for (i, e) in l.edges.iter().enumerate() {
            e.validate(1e-6).map_err(invalid)?;
            let next = &l.edges[(i + 1) % l.edges.len()];
            let (_, end) = e.ends();
            let (start, _) = next.ends();
            if (end[0] - start[0]).hypot(end[1] - start[1]) > 1e-6 {
                return Err(invalid("A captured sheet loop is open"));
            }
        }
        let area = l.signed_area();
        if (area > 0.) != l.outer || area.abs() < 1e-9 {
            return Err(invalid(
                "Captured outer loops run counterclockwise and holes clockwise",
            ));
        }
    }
    Ok(())
}

impl SheetDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() || self.stock.is_nil() {
            return Err(invalid("A sheet part needs an output Part and sheet stock"));
        }
        match &self.base {
            SheetBase::Profile { profile } => {
                if profile.sketch.is_nil() {
                    return Err(invalid("A profile sheet needs a sketch"));
                }
                validate_region(&profile.region)?;
            }
            SheetBase::Contour {
                frame,
                loops,
                converted_from,
            } => {
                frame.validate()?;
                validate_loops(loops)?;
                if converted_from.is_some_and(|p| p.is_nil()) {
                    return Err(invalid("Converted source identity is nil"));
                }
            }
            SheetBase::Rectangle { .. } => {}
        }
        if self.flanges.len() > MAX_FLANGES || self.cutouts.len() > MAX_CUTOUTS {
            return Err(invalid(format!(
                "A sheet part holds at most {MAX_FLANGES} flanges and {MAX_CUTOUTS} cutouts"
            )));
        }
        let mut edges = std::collections::BTreeSet::new();
        for f in &self.flanges {
            if !edges.insert(f.edge) {
                return Err(invalid(format!(
                    "Base edge {} carries more than one flange",
                    f.edge
                )));
            }
        }
        if self.fasteners.len() > MAX_PATTERNS {
            return Err(invalid(format!(
                "A sheet part holds at most {MAX_PATTERNS} mounting patterns"
            )));
        }
        for p in &self.fasteners {
            p.validate(self.flanges.len())?;
        }
        for c in &self.cutouts {
            if let CutoutFace::Flange { index } = c.face
                && index as usize >= self.flanges.len()
            {
                return Err(invalid(format!("Cutout names missing flange {index}")));
            }
        }
        for d in self.lengths() {
            check_dimension(d, Unit::Millimeter)?;
        }
        for d in self.flanges.iter().flat_map(Flange::angles) {
            check_dimension(d, Unit::Degree)?;
        }
        if let Some(k) = &self.k_factor {
            check_dimension(k, Unit::Ratio)?;
        }
        Ok(())
    }
    fn lengths(&self) -> Vec<&Dimension> {
        let mut out = vec![];
        if let SheetBase::Rectangle {
            origin,
            length,
            width,
            ..
        } = &self.base
        {
            out.extend(origin);
            out.extend([length, width]);
        }
        out.extend(&self.inside_radius);
        out.extend(self.flanges.iter().flat_map(Flange::lengths));
        out.extend(self.cutouts.iter().flat_map(Cutout::lengths));
        out.extend(self.fasteners.iter().flat_map(MountingPattern::dimensions));
        out
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut out = self.lengths();
        out.extend(&self.k_factor);
        out.extend(self.flanges.iter().flat_map(Flange::angles));
        out
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        definition(SHEET_TYPE, self)
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != SHEET_TYPE || d.schema_version != SHEET_SCHEMA {
            return Err(invalid(format!(
                "Expected {SHEET_TYPE} schema {SHEET_SCHEMA}"
            )));
        }
        let value: Self =
            serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        value.validate()?;
        Ok(value)
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(self.dimensions());
        links.push(Link::new(self.stock, "stock"));
        if let SheetBase::Profile { profile } = &self.base {
            links.push(Link::new(profile.sketch, "profile"));
        }
        links
    }
}

pub fn sheet_parts(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == SHEET_TYPE),
    )
}
/// Sheet stock: a stock library entry with a sheet specification.
pub fn read_sheet_stock(document: &Document, id: EntityId) -> Result<Stock, DocumentError> {
    let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
    match materials::read(entity)? {
        Some(LibraryDefinition::Stock(
            stock @ Stock {
                specification: StockKind::Sheet { .. },
                ..
            },
        )) => Ok(stock),
        _ => Err(invalid(format!(
            "{} is not sheet stock; choose a sheet stock definition",
            entity.name
        ))),
    }
}
pub fn read_sheet(
    document: &Document,
    feature: EntityId,
) -> Result<SheetDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a sheet-metal feature"));
    };
    let sheet = SheetDefinition::from_definition(definition)?;
    let expected = sheet.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid("Sheet source and canonical input links disagree"));
    }
    if let Some(entity) = document.get(sheet.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Sheet output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("Sheet output Part must contain its sheet feature"));
        }
    }
    Ok(sheet)
}
/// The sheet feature producing a Part, if it is a sheet part.
pub fn sheet_feature(document: &Document, part: EntityId) -> Option<EntityId> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return None;
    };
    features.iter().copied().find(|f| {
        matches!(document.get(*f).map(|e| &e.data), Some(EntityData::Feature { definition, .. }) if definition.type_id == SHEET_TYPE)
    })
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for e in sheet_parts(document) {
        read_sheet(document, e.id)?;
    }
    validate_hardware(document)
}
