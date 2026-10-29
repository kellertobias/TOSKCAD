//! Weldment members: straight tube and profile stock members placed along a
//! path segment. Each member is a model feature producing its own Part from
//! profile stock, so members of one frame are never fused into one body.
//! A member path is either an explicit point pair or one line of a sketch;
//! a sketch line keeps its identity across sketch edits, so the member and
//! anything that names it survive path changes.
//!
//! The section is placed in a member frame: the axis runs from the path
//! start to its end, the section height (y) follows a reference direction
//! and the width (x) is y × axis. An anchor selects which point of the
//! section bounding box lies on the path, and a rotation turns the section
//! about the axis. The native evaluator builds the solids and measures cut
//! records from them.
mod connection;
mod prepare;
mod resolve;
mod weld;
pub use connection::*;
pub use prepare::*;
pub use resolve::*;
pub use weld::*;

use super::{
    common::parameter_links,
    source::{BodyOutput, check_dimension, invalid},
};
use crate::{
    constraint_sketch::{CurveKind, Dimension, source::Source},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link},
    materials::{self, LibraryDefinition, Stock, StockKind},
    parameters::Unit,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MEMBER_TYPE: &str = "weldment.member";
pub const MEMBER_SCHEMA: u32 = 1;
pub const MEMBER_CAPABILITY: &str = "weldment.member.1";
/// Members created in one frame transaction.
pub const MAX_FRAME_MEMBERS: usize = 256;

/// Where a member runs. Both forms are straight segments.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemberPath {
    /// World points in millimeters.
    Points {
        start: [Dimension; 3],
        end: [Dimension; 3],
    },
    /// One non-construction line of a constrained sketch, by its stable
    /// curve identity. The sketch normal is the section height reference.
    SketchLine { sketch: EntityId, line: EntityId },
}
/// Which point of the section bounding box lies on the path, along one
/// section axis: the minimum edge, the middle or the maximum edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisAnchor {
    Min,
    #[default]
    Center,
    Max,
}
impl AxisAnchor {
    pub fn fraction(self) -> f64 {
        match self {
            Self::Min => 0.,
            Self::Center => 0.5,
            Self::Max => 1.,
        }
    }
}
fn centered(anchor: &[AxisAnchor; 2]) -> bool {
    *anchor == [AxisAnchor::Center; 2]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberDefinition {
    pub output: BodyOutput,
    /// Profile stock supplying the section, material and bar length.
    pub stock: EntityId,
    pub path: MemberPath,
    /// Section anchor along the width (x) and height (y).
    #[serde(default, skip_serializing_if = "centered")]
    pub anchor: [AxisAnchor; 2],
    /// Section rotation about the member axis, counterclockwise looking from
    /// the path end back toward its start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Dimension>,
    /// Extra length beyond the path start; negative trims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extend_start: Option<Dimension>,
    /// Extra length beyond the path end; negative trims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extend_end: Option<Dimension>,
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

impl MemberDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() {
            return Err(invalid("A member needs an output Part"));
        }
        if self.stock.is_nil() {
            return Err(invalid("A member needs profile stock"));
        }
        if let MemberPath::SketchLine { sketch, line } = &self.path
            && (sketch.is_nil() || line.is_nil())
        {
            return Err(invalid("A sketch-line member needs its sketch and line"));
        }
        for d in self.lengths() {
            check_dimension(d, Unit::Millimeter)?;
        }
        if let Some(r) = &self.rotation {
            check_dimension(r, Unit::Degree)?;
        }
        Ok(())
    }
    fn lengths(&self) -> Vec<&Dimension> {
        let mut out = vec![];
        if let MemberPath::Points { start, end } = &self.path {
            out.extend(start.iter().chain(end));
        }
        out.extend(&self.extend_start);
        out.extend(&self.extend_end);
        out
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        definition(MEMBER_TYPE, self)
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        let value: Self = parse(d, MEMBER_TYPE)?;
        value.validate()?;
        Ok(value)
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut dims = self.lengths();
        dims.extend(&self.rotation);
        let mut links = parameter_links(dims);
        links.push(Link::new(self.stock, "stock"));
        if let MemberPath::SketchLine { sketch, .. } = &self.path {
            links.push(Link::new(*sketch, "path"));
        }
        links
    }
}

pub fn members(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == MEMBER_TYPE),
    )
}
/// The member feature producing a Part, if it is a member.
pub fn member_feature(document: &Document, part: EntityId) -> Option<EntityId> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return None;
    };
    features.iter().copied().find(|f| {
        matches!(document.get(*f).map(|e| &e.data), Some(EntityData::Feature { definition, .. }) if definition.type_id == MEMBER_TYPE)
    })
}
pub fn read_member(
    document: &Document,
    feature: EntityId,
) -> Result<MemberDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a weldment member feature"));
    };
    let member = MemberDefinition::from_definition(definition)?;
    let expected = member.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid("Member source and canonical input links disagree"));
    }
    if let Some(entity) = document.get(member.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Member output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid(
                "Member output Part must contain its member feature",
            ));
        }
    }
    Ok(member)
}
/// Profile stock: a stock library entry with a profile specification.
pub fn read_profile_stock(document: &Document, id: EntityId) -> Result<Stock, DocumentError> {
    let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
    match materials::read(entity)? {
        Some(LibraryDefinition::Stock(
            stock @ Stock {
                specification: StockKind::Profile { .. },
                ..
            },
        )) => Ok(stock),
        _ => Err(invalid(format!(
            "{} is not profile stock; choose a tube, bar or angle stock definition",
            entity.name
        ))),
    }
}
/// The non-construction lines of a sketch, in source order. These are the
/// segments a frame path can turn into members.
pub fn sketch_path_lines(
    document: &Document,
    sketch: EntityId,
) -> Result<Vec<EntityId>, DocumentError> {
    let (source, ..) = Source::read(document, sketch)?;
    Ok(source
        .sketch
        .curves
        .iter()
        .filter(|c| !c.construction && matches!(c.kind, CurveKind::Line { .. }))
        .map(|c| c.id)
        .collect())
}
/// Structural check of a sketch-line path; the native evaluator solves it.
pub fn check_sketch_line(
    document: &Document,
    sketch: EntityId,
    line: EntityId,
) -> Result<(), DocumentError> {
    let (source, ..) = Source::read(document, sketch)?;
    match source.sketch.curves.iter().find(|c| c.id == line) {
        Some(c) if c.construction => Err(invalid(
            "A construction line is not a member path; use a solid line",
        )),
        Some(c) if matches!(c.kind, CurveKind::Line { .. }) => Ok(()),
        Some(_) => Err(invalid(
            "Members follow straight lines only; arcs, circles and splines are not supported paths",
        )),
        None => Err(invalid(
            "The member's sketch line no longer exists; choose another path",
        )),
    }
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    let mut lines = BTreeSet::new();
    for e in members(document) {
        let member = read_member(document, e.id)?;
        if let MemberPath::SketchLine { sketch, line } = member.path
            && !lines.insert((sketch, line))
        {
            return Err(invalid(format!(
                "Sketch line {line} carries more than one weldment member"
            )));
        }
    }
    validate_connections(document)?;
    validate_welds(document)
}
