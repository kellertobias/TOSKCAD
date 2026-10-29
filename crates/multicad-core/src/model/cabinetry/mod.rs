//! Cabinetry panels, carcasses and face-to-face stacks. A panel is a model
//! feature producing one Part from panel stock: a rectangle placed on a
//! principal plane, a sketch region, or one role of a carcass. A carcass is a
//! document extension holding the parametric cabinet envelope; each of its
//! panels is an ordinary editable Part whose geometry follows the carcass. A
//! stack records face-to-face layers that keep their own Parts and stock.
//!
//! The modeled solid is the finished panel, edge banding included. Core cut
//! dimensions subtract banding, add banding allowances and any panel
//! oversize, and are reported along and across the grain.
mod joint;
mod prepare;
mod resolve;
pub use joint::*;
pub use prepare::*;
pub use resolve::*;

use super::{
    common::parameter_links,
    source::{BodyOutput, ProfileSource, check_dimension, invalid, validate_region},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link},
    materials::{self, LibraryDefinition, Stock, StockKind},
    parameters::Unit,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PANEL_TYPE: &str = "cabinetry.panel";
pub const PANEL_SCHEMA: u32 = 1;
pub const PANEL_CAPABILITY: &str = "cabinetry.panel.1";
pub const CARCASS_KIND: &str = "cabinetry.carcass";
pub const CARCASS_CAPABILITY: &str = "cabinetry.carcass.1";
pub const STACK_KIND: &str = "cabinetry.stack";
pub const STACK_CAPABILITY: &str = "cabinetry.stack.1";
pub const MAX_SHELVES: usize = 64;
pub const MAX_STACK_LAYERS: usize = 32;

/// One banded edge: the band thickness is outside the core, and the allowance
/// is extra core material per banded edge, trimmed after banding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Band {
    pub thickness: Dimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance: Option<Dimension>,
}
/// Banding on the four edges of a panel rectangle. `u_start` is the edge at
/// u = 0, running along v; `v_start` is the edge at v = 0, running along u.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelEdges {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_start: Option<Band>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_end: Option<Band>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v_start: Option<Band>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v_end: Option<Band>,
}
impl PanelEdges {
    pub fn bands(&self) -> [Option<&Band>; 4] {
        [
            self.u_start.as_ref(),
            self.u_end.as_ref(),
            self.v_start.as_ref(),
            self.v_end.as_ref(),
        ]
    }
    pub fn is_empty(&self) -> bool {
        self.bands().iter().all(Option::is_none)
    }
}
/// The panel axis the grain runs along. `Free` allows either orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainAxis {
    #[default]
    U,
    V,
    Free,
}
/// A principal plane for a rectangular panel: (u, v) are (X, Y), (X, Z) or
/// (Y, Z). The thickness runs along u × v: +Z, −Y or +X.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelPlane {
    Xy,
    Xz,
    Yz,
}
impl PanelPlane {
    pub fn axes(self) -> ([f64; 3], [f64; 3]) {
        match self {
            Self::Xy => ([1., 0., 0.], [0., 1., 0.]),
            Self::Xz => ([1., 0., 0.], [0., 0., 1.]),
            Self::Yz => ([0., 1., 0.], [0., 0., 1.]),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelRole {
    LeftSide,
    RightSide,
    Top,
    Bottom,
    Back,
    Shelf { index: u32 },
}
impl PanelRole {
    pub fn label(self) -> String {
        match self {
            Self::LeftSide => "left side".into(),
            Self::RightSide => "right side".into(),
            Self::Top => "top".into(),
            Self::Bottom => "bottom".into(),
            Self::Back => "back".into(),
            Self::Shelf { index } => format!("shelf {}", index + 1),
        }
    }
}
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelSource {
    /// A rectangle on a principal plane with its minimum corner at `origin`.
    Rectangle {
        stock: EntityId,
        plane: PanelPlane,
        origin: [Dimension; 3],
        length: Dimension,
        width: Dimension,
    },
    /// A sketch region extruded by the stock thickness along the sketch
    /// normal. Grain axes are the sketch plane's u and v.
    Profile {
        stock: EntityId,
        profile: ProfileSource,
    },
    /// One panel of a carcass. Stock, grain and banding follow the carcass.
    Carcass { carcass: EntityId, role: PanelRole },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelDefinition {
    pub output: BodyOutput,
    pub source: PanelSource,
    /// Rectangle and profile panels only; carcass panels follow their role.
    #[serde(default)]
    pub grain: GrainAxis,
    /// Rectangle panels only; carcass panels take the carcass front band.
    #[serde(default, skip_serializing_if = "PanelEdges::is_empty")]
    pub edges: PanelEdges,
    /// Extra core material added to both cut dimensions, for trimming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oversize: Option<Dimension>,
}
/// How the carcass sides meet the top and bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Construction {
    /// Full-height sides; top and bottom fit between them.
    #[default]
    SidesFullHeight,
    /// Full-width top and bottom; sides fit between them.
    TopBottomFullWidth,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Back {
    None,
    /// Captured between the sides, top and bottom, `recess` in from the rear.
    Inset {
        stock: EntityId,
        recess: Dimension,
    },
    /// Applied over the rear edges of the sides, top and bottom.
    Overlay {
        stock: EntityId,
    },
}
impl Back {
    pub fn stock(&self) -> Option<EntityId> {
        match self {
            Self::None => None,
            Self::Inset { stock, .. } | Self::Overlay { stock } => Some(*stock),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShelfLayout {
    /// Shelves spaced so the interior gaps are equal.
    Even { count: u32 },
    /// Distances from the interior bottom to each shelf's underside.
    Offsets { offsets: Vec<Dimension> },
}
impl ShelfLayout {
    pub fn count(&self) -> usize {
        match self {
            Self::Even { count } => *count as usize,
            Self::Offsets { offsets } => offsets.len(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shelves {
    pub layout: ShelfLayout,
    /// Distance from the cabinet front to the shelf front edge.
    pub setback: Dimension,
    /// Gap between each shelf end and the side it sits beside.
    pub clearance: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarcassDefinition {
    /// World position of the cabinet's front-left-bottom corner.
    pub origin: [Dimension; 3],
    /// Rotation about +Z through `origin`. Unrotated, width runs along +X,
    /// depth along +Y (the front faces −Y) and height along +Z.
    pub rotation: Dimension,
    pub width: Dimension,
    pub height: Dimension,
    pub depth: Dimension,
    /// Panel stock for the sides, top, bottom and shelves.
    pub stock: EntityId,
    #[serde(default)]
    pub construction: Construction,
    pub back: Back,
    pub shelves: Shelves,
    /// Banding on the front edges of the sides, top, bottom and shelves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_band: Option<Band>,
    /// Optional model group holding the carcass panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<EntityId>,
}
/// Face-to-face layers, listed in stacking order. Each layer keeps its own
/// Part, material, stock and grain; the optional group holds them as one
/// finished assembly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackDefinition {
    pub layers: Vec<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adhesive: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<EntityId>,
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
fn band_dimensions(band: &Band) -> Vec<&Dimension> {
    std::iter::once(&band.thickness)
        .chain(&band.allowance)
        .collect()
}

impl PanelDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() {
            return Err(invalid("A panel needs an output Part"));
        }
        match &self.source {
            PanelSource::Rectangle { stock, .. } if stock.is_nil() => {
                return Err(invalid("A panel needs panel stock"));
            }
            PanelSource::Profile { stock, profile } => {
                if stock.is_nil() || profile.sketch.is_nil() {
                    return Err(invalid("A profile panel needs panel stock and a sketch"));
                }
                validate_region(&profile.region)?;
                if !self.edges.is_empty() {
                    return Err(invalid(
                        "Edge banding is supported on rectangle and carcass panels only",
                    ));
                }
            }
            PanelSource::Carcass { carcass, .. } => {
                if carcass.is_nil() {
                    return Err(invalid("A carcass panel needs its carcass"));
                }
                if !self.edges.is_empty() || self.grain != GrainAxis::U {
                    return Err(invalid(
                        "Carcass panels take their grain and banding from the carcass",
                    ));
                }
            }
            PanelSource::Rectangle { .. } => {}
        }
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut out = vec![];
        if let PanelSource::Rectangle {
            origin,
            length,
            width,
            ..
        } = &self.source
        {
            out.extend(origin);
            out.extend([length, width]);
        }
        for band in self.edges.bands().into_iter().flatten() {
            out.extend(band_dimensions(band));
        }
        out.extend(&self.oversize);
        out
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        definition(PANEL_TYPE, self)
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        let value: Self = parse(d, PANEL_TYPE)?;
        value.validate()?;
        Ok(value)
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(self.dimensions());
        match &self.source {
            PanelSource::Rectangle { stock, .. } => links.push(Link::new(*stock, "stock")),
            PanelSource::Profile { stock, profile } => {
                links.push(Link::new(*stock, "stock"));
                links.push(Link::new(profile.sketch, "profile"));
            }
            PanelSource::Carcass { carcass, .. } => links.push(Link::new(*carcass, "carcass")),
        }
        links
    }
}
impl CarcassDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.stock.is_nil() || self.back.stock().is_some_and(|s| s.is_nil()) {
            return Err(invalid("A carcass needs panel stock"));
        }
        if self.shelves.layout.count() > MAX_SHELVES {
            return Err(invalid(format!(
                "A carcass holds at most {MAX_SHELVES} shelves"
            )));
        }
        if self.group.is_some_and(|g| g.is_nil()) {
            return Err(invalid("Carcass group identity is nil"));
        }
        check_dimension(&self.rotation, Unit::Degree)?;
        for d in self.lengths() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    fn lengths(&self) -> Vec<&Dimension> {
        let mut out: Vec<&Dimension> = self.origin.iter().collect();
        out.extend([&self.width, &self.height, &self.depth]);
        if let Back::Inset { recess, .. } = &self.back {
            out.push(recess);
        }
        if let ShelfLayout::Offsets { offsets } = &self.shelves.layout {
            out.extend(offsets);
        }
        out.extend([&self.shelves.setback, &self.shelves.clearance]);
        if let Some(band) = &self.front_band {
            out.extend(band_dimensions(band));
        }
        out
    }
    /// The panel roles this carcass generates, in display order.
    pub fn roles(&self) -> Vec<PanelRole> {
        let mut roles = vec![
            PanelRole::LeftSide,
            PanelRole::RightSide,
            PanelRole::Top,
            PanelRole::Bottom,
        ];
        if !matches!(self.back, Back::None) {
            roles.push(PanelRole::Back);
        }
        roles.extend(
            (0..self.shelves.layout.count()).map(|index| PanelRole::Shelf {
                index: index as u32,
            }),
        );
        roles
    }
    pub fn stock_for(&self, role: PanelRole) -> Option<EntityId> {
        match role {
            PanelRole::Back => self.back.stock(),
            _ => Some(self.stock),
        }
    }
    fn inputs(&self) -> Vec<Link> {
        let mut dims = self.lengths();
        dims.push(&self.rotation);
        let mut links = parameter_links(dims);
        links.push(Link::new(self.stock, "stock"));
        if let Some(back) = self.back.stock() {
            links.push(Link::new(back, "back_stock"));
        }
        links
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(CARCASS_KIND, self)?,
            inputs: self.inputs(),
        })
    }
}
impl StackDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.layers.len() < 2 || self.layers.len() > MAX_STACK_LAYERS {
            return Err(invalid(format!(
                "A stack needs 2–{MAX_STACK_LAYERS} layers"
            )));
        }
        if self.layers.iter().any(EntityId::is_nil)
            || self.layers.iter().collect::<BTreeSet<_>>().len() != self.layers.len()
        {
            return Err(invalid("Stack layers must be distinct Parts"));
        }
        if self
            .adhesive
            .as_ref()
            .is_some_and(|a| a.trim().is_empty() || a.len() > 256)
        {
            return Err(invalid("Adhesive name must contain 1–256 bytes"));
        }
        if self.group.is_some_and(|g| g.is_nil()) {
            return Err(invalid("Stack group identity is nil"));
        }
        Ok(())
    }
    fn inputs(&self) -> Vec<Link> {
        self.layers
            .iter()
            .enumerate()
            .map(|(i, part)| Link::new(*part, format!("layer/{i}")))
            .collect()
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(STACK_KIND, self)?,
            inputs: self.inputs(),
        })
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
fn is_kind(entity: &Entity, kind: &str) -> bool {
    matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == kind)
}
pub fn carcasses(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| is_kind(e, CARCASS_KIND))
}
pub fn stacks(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| is_kind(e, STACK_KIND))
}
pub fn panels(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == PANEL_TYPE),
    )
}
/// Panel stock: a stock library entry with a panel specification.
pub fn read_panel_stock(document: &Document, id: EntityId) -> Result<Stock, DocumentError> {
    let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
    match materials::read(entity)? {
        Some(LibraryDefinition::Stock(
            stock @ Stock {
                specification: StockKind::Panel { .. },
                ..
            },
        )) => Ok(stock),
        _ => Err(invalid(format!(
            "{} is not panel stock; choose a panel stock definition",
            entity.name
        ))),
    }
}
pub fn read_carcass(document: &Document, id: EntityId) -> Result<CarcassDefinition, DocumentError> {
    let value: CarcassDefinition = parse(&extension(document, id, CARCASS_KIND)?, CARCASS_KIND)?;
    value.validate()?;
    Ok(value)
}
pub fn read_stack(document: &Document, id: EntityId) -> Result<StackDefinition, DocumentError> {
    let value: StackDefinition = parse(&extension(document, id, STACK_KIND)?, STACK_KIND)?;
    value.validate()?;
    Ok(value)
}
pub fn read_panel(
    document: &Document,
    feature: EntityId,
) -> Result<PanelDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a panel feature"));
    };
    let panel = PanelDefinition::from_definition(definition)?;
    let expected = panel.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid("Panel source and canonical input links disagree"));
    }
    if let Some(entity) = document.get(panel.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Panel output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("Panel output Part must contain its panel feature"));
        }
    }
    Ok(panel)
}
/// The panel feature producing a Part, if it is a panel.
pub fn panel_feature(document: &Document, part: EntityId) -> Option<EntityId> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return None;
    };
    features.iter().copied().find(|f| {
        matches!(document.get(*f).map(|e| &e.data), Some(EntityData::Feature { definition, .. }) if definition.type_id == PANEL_TYPE)
    })
}
/// Panel features generated by a carcass, by role.
pub fn carcass_panels(
    document: &Document,
    carcass: EntityId,
) -> Result<Vec<(PanelRole, EntityId, EntityId)>, DocumentError> {
    let mut out = vec![];
    for e in panels(document) {
        let panel = read_panel(document, e.id)?;
        if let PanelSource::Carcass { carcass: c, role } = panel.source
            && c == carcass
        {
            out.push((role, e.id, panel.output.body));
        }
    }
    out.sort();
    Ok(out)
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for e in carcasses(document) {
        read_carcass(document, e.id)?;
    }
    let mut roles = BTreeSet::new();
    for e in panels(document) {
        let panel = read_panel(document, e.id)?;
        if let PanelSource::Carcass { carcass, role } = panel.source {
            if !roles.insert((carcass, role)) {
                return Err(invalid(format!(
                    "Carcass {carcass} has more than one {} panel",
                    role.label()
                )));
            }
            if document.get(carcass).is_some() {
                read_carcass(document, carcass)?;
            }
        }
    }
    for e in stacks(document) {
        read_stack(document, e.id)?;
    }
    validate_panel_joints(document)
}
