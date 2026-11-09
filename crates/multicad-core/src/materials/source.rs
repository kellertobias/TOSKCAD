use crate::document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link};
use crate::project_file::ContentHash;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub color_rgba: [u8; 4],
    pub roughness: f64,
    pub metallic: f64,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            color_rgba: [137, 174, 199, 255],
            roughness: 0.6,
            metallic: 0.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HatchPattern {
    None,
    Lines,
    Cross,
    Alternating,
    Solid,
    Dots,
    SmallCrosses,
}
impl HatchPattern {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Lines,
        Self::Alternating,
        Self::Cross,
        Self::Solid,
        Self::Dots,
        Self::SmallCrosses,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No hatch",
            Self::Lines => "Parallel lines",
            Self::Alternating => "Alternating solid and dotted lines",
            Self::Cross => "Crosshatch",
            Self::Solid => "Solid fill",
            Self::Dots => "Dots",
            Self::SmallCrosses => "Small crosses",
        }
    }
}
/// Paper-space distances in millimeters; angle measured counterclockwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HatchStyle {
    pub pattern: HatchPattern,
    pub angle_deg: f64,
    pub spacing_mm: f64,
    pub line_width_mm: f64,
}
impl Default for HatchStyle {
    fn default() -> Self {
        Self {
            pattern: HatchPattern::Lines,
            angle_deg: 45.0,
            spacing_mm: 3.0,
            line_width_mm: 0.18,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grain {
    None,
    AlongLength,
    AcrossLength,
}
/// Cross-section family of profile stock. Width and height are the section
/// bounding box; `wall_mm` is the tube wall, or the leg thickness of an angle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileShape {
    /// Square or rectangular hollow section (SHS/RHS) with a wall, or a
    /// solid flat/square bar without one.
    #[default]
    Rectangular,
    /// Circular hollow section (CHS) with a wall, or a round bar without one.
    /// Width and height are both the outside diameter.
    Round,
    /// Angle (L) section: legs of width and height, wall = leg thickness.
    Angle,
    /// Standard item Line 8, article 0.0.026.03, with four open grooves.
    #[serde(rename = "item_line_8_40x40")]
    ItemLine8_40x40,
}

pub const ITEM_LINE_8_PROFILE_CAPABILITY: &str = "materials.profile.item_line_8_40x40.1";
impl ProfileShape {
    pub fn is_rectangular(&self) -> bool {
        *self == Self::Rectangular
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StockKind {
    Panel {
        thickness_mm: f64,
        length_mm: f64,
        width_mm: f64,
        grain: Grain,
    },
    Sheet {
        thickness_mm: f64,
        length_mm: f64,
        width_mm: f64,
        inside_bend_radius_mm: f64,
        k_factor: f64,
    },
    Profile {
        designation: String,
        length_mm: f64,
        width_mm: f64,
        height_mm: f64,
        wall_mm: Option<f64>,
        #[serde(default, skip_serializing_if = "ProfileShape::is_rectangular")]
        shape: ProfileShape,
    },
    Print {
        process: String,
        nozzle_mm: Option<f64>,
        layer_height_mm: f64,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stock {
    pub material: EntityId,
    pub specification: StockKind,
}
/// Acknowledged source content, not a frozen copy. Changed definitions remain
/// live and produce a diagnostic until the user explicitly reassigns them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryRef {
    pub id: EntityId,
    pub content: ContentHash,
}
impl LibraryRef {
    pub fn capture(document: &Document, id: EntityId) -> Result<Self, DocumentError> {
        let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
        Ok(Self {
            id,
            content: fingerprint(entity)?,
        })
    }
}
pub fn fingerprint(entity: &Entity) -> Result<ContentHash, DocumentError> {
    // Names/descriptions do not change manufacturing content or tag identity.
    serde_json::to_vec(&entity.data)
        .map(|v| ContentHash::of(&v))
        .map_err(|e| DocumentError::Invalid(e.to_string()))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialStyle {
    pub material: EntityId,
    pub appearance: Option<LibraryRef>,
    pub hatch: Option<LibraryRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartProperties {
    pub part: EntityId,
    pub material: Option<LibraryRef>,
    pub stock: Option<LibraryRef>,
    pub appearance: Option<LibraryRef>,
    pub hatch: Option<LibraryRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum LibraryDefinition {
    Appearance(Appearance),
    Hatch(HatchStyle),
    Stock(Stock),
    MaterialStyle(MaterialStyle),
    PartProperties(PartProperties),
}
impl LibraryDefinition {
    pub fn type_id(&self) -> &'static str {
        match self {
            Self::Appearance(_) => "materials.appearance",
            Self::Hatch(_) => "materials.hatch",
            Self::Stock(_) => "materials.stock",
            Self::MaterialStyle(_) => "materials.material_style",
            Self::PartProperties(_) => "materials.part_properties",
        }
    }
    pub fn links(&self) -> Vec<Link> {
        let mut links = Vec::new();
        let mut add = |id, role| links.push(Link::new(id, role));
        match self {
            Self::Stock(s) => add(s.material, "material"),
            Self::MaterialStyle(s) => {
                add(s.material, "material");
                for (r, role) in [(&s.appearance, "appearance"), (&s.hatch, "hatch")] {
                    if let Some(r) = r {
                        add(r.id, role);
                    }
                }
            }
            Self::PartProperties(s) => {
                add(s.part, "part");
                for (r, role) in [
                    (&s.material, "material"),
                    (&s.stock, "stock"),
                    (&s.appearance, "appearance"),
                    (&s.hatch, "hatch"),
                ] {
                    if let Some(r) = r {
                        add(r.id, role);
                    }
                }
            }
            _ => {}
        };
        links
    }
    pub fn entity(&self, id: EntityId, name: impl Into<String>) -> Result<Entity, DocumentError> {
        self.validate()?;
        let mut entity = Entity::new(
            name,
            EntityData::Extension {
                definition: Definition {
                    type_id: self.type_id().into(),
                    schema_version: 1,
                    data: serde_json::to_value(self)
                        .map_err(|e| DocumentError::Invalid(e.to_string()))?,
                },
                inputs: self.links(),
            },
        );
        entity.id = id;
        Ok(entity)
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let unit = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        let valid = match self {
            Self::Appearance(a) => unit(a.roughness) && unit(a.metallic),
            Self::Hatch(h) => {
                h.angle_deg.is_finite()
                    && positive(h.spacing_mm)
                    && positive(h.line_width_mm)
                    && h.line_width_mm <= h.spacing_mm
            }
            Self::Stock(s) => {
                !s.material.is_nil()
                    && match &s.specification {
                        StockKind::Panel {
                            thickness_mm,
                            length_mm,
                            width_mm,
                            ..
                        } => [*thickness_mm, *length_mm, *width_mm]
                            .into_iter()
                            .all(positive),
                        StockKind::Sheet {
                            thickness_mm,
                            length_mm,
                            width_mm,
                            inside_bend_radius_mm,
                            k_factor,
                        } => {
                            [*thickness_mm, *length_mm, *width_mm]
                                .into_iter()
                                .all(positive)
                                && inside_bend_radius_mm.is_finite()
                                && *inside_bend_radius_mm >= 0.0
                                && unit(*k_factor)
                        }
                        StockKind::Profile {
                            designation,
                            length_mm,
                            width_mm,
                            height_mm,
                            wall_mm,
                            shape,
                        } => {
                            !designation.trim().is_empty()
                                && designation.len() <= 256
                                && [*length_mm, *width_mm, *height_mm]
                                    .into_iter()
                                    .all(positive)
                                && wall_mm.is_none_or(|v| {
                                    positive(v) && v * 2.0 <= width_mm.min(*height_mm)
                                })
                                && match shape {
                                    ProfileShape::Rectangular => true,
                                    ProfileShape::Round => width_mm == height_mm,
                                    ProfileShape::Angle => wall_mm.is_some(),
                                    ProfileShape::ItemLine8_40x40 => {
                                        *width_mm == 40.
                                            && *height_mm == 40.
                                            && wall_mm.is_none()
                                            && *length_mm <= 6000.
                                    }
                                }
                        }
                        StockKind::Print {
                            process,
                            nozzle_mm,
                            layer_height_mm,
                        } => {
                            !process.trim().is_empty()
                                && process.len() <= 256
                                && positive(*layer_height_mm)
                                && nozzle_mm.is_none_or(positive)
                        }
                    }
            }
            Self::MaterialStyle(s) => !s.material.is_nil(),
            Self::PartProperties(s) => !s.part.is_nil(),
        };
        if !valid || self.links().iter().any(|l| l.target.is_nil()) {
            return Err(DocumentError::Invalid(format!(
                "Invalid {} definition",
                self.type_id()
            )));
        }
        Ok(())
    }
}
pub fn read(entity: &Entity) -> Result<Option<LibraryDefinition>, DocumentError> {
    let EntityData::Extension { definition, inputs } = &entity.data else {
        return Ok(None);
    };
    if !is_type(&definition.type_id) {
        return Ok(None);
    }
    if definition.schema_version != 1 {
        return Err(DocumentError::Invalid(format!(
            "Unsupported {} schema {}",
            definition.type_id, definition.schema_version
        )));
    }
    let value: LibraryDefinition = serde_json::from_value(definition.data.clone())
        .map_err(|e| DocumentError::Invalid(format!("Library {}: {e}", entity.id)))?;
    value.validate()?;
    if value.type_id() != definition.type_id || *inputs != value.links() {
        return Err(DocumentError::Invalid(format!(
            "Library {} definition/link mismatch",
            entity.id
        )));
    }
    Ok(Some(value))
}
fn is_type(id: &str) -> bool {
    matches!(
        id,
        "materials.appearance"
            | "materials.hatch"
            | "materials.stock"
            | "materials.material_style"
            | "materials.part_properties"
    )
}
pub fn required_capability(document: &Document) -> bool {
    document.entities().any(
        |e| matches!(&e.data,EntityData::Extension{definition,..} if is_type(&definition.type_id))
            || matches!(&e.data,EntityData::Material{section_hatch:Some(h),..} if h.type_id=="materials.hatch.inline"),
    )
}

pub fn requires_item_line_8_profile(document: &Document) -> bool {
    let slotted_stock = |data: &serde_json::Value| {
        data.pointer("/value/specification/shape")
            .and_then(serde_json::Value::as_str)
            == Some("item_line_8_40x40")
    };
    document.entities().any(|entity| match &entity.data {
        EntityData::Extension { definition, .. } if definition.type_id == "materials.stock" => {
            slotted_stock(&definition.data)
        }
        EntityData::Extension { definition, .. }
            if definition.type_id == crate::symbols::DEFINITION_TYPE =>
        {
            definition
                .data
                .pointer("/template/entities")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|entities| {
                    entities.values().any(|entity| {
                        entity
                            .pointer("/data/definition/type_id")
                            .and_then(serde_json::Value::as_str)
                            == Some("materials.stock")
                            && entity
                                .pointer("/data/definition/data")
                                .is_some_and(slotted_stock)
                    })
                })
        }
        _ => false,
    })
}

pub fn validate(document: &Document) -> Result<(), DocumentError> {
    let mut owners = BTreeSet::new();
    for entity in document.entities() {
        if let EntityData::Material {
            section_hatch: Some(hatch),
            ..
        } = &entity.data
            && hatch.type_id == "materials.hatch.inline"
        {
            if hatch.schema_version != 1 {
                return Err(DocumentError::Invalid(
                    "Unsupported inline section hatch schema".into(),
                ));
            }
            let style: HatchStyle = serde_json::from_value(hatch.data.clone())
                .map_err(|e| DocumentError::Invalid(e.to_string()))?;
            LibraryDefinition::Hatch(style).validate()?;
        }
        let owner = match read(entity)? {
            Some(LibraryDefinition::MaterialStyle(s)) => Some((0, s.material)),
            Some(LibraryDefinition::PartProperties(s)) => Some((1, s.part)),
            _ => None,
        };
        if let Some(owner) = owner
            && !owners.insert(owner)
        {
            return Err(DocumentError::Invalid(format!(
                "Duplicate material association for {}",
                owner.1
            )));
        }
    }
    Ok(())
}

/// Metadata ownership participates in cache invalidation only. These reverse
/// owner links are excluded from canonical dependencies and topological ordering.
pub(crate) fn invalidation_edges(
    document: &Document,
) -> Result<Vec<(EntityId, EntityId)>, DocumentError> {
    let mut edges = Vec::new();
    for entity in document.entities() {
        let owner = match read(entity)? {
            Some(LibraryDefinition::MaterialStyle(style)) => Some(style.material),
            Some(LibraryDefinition::PartProperties(properties)) => Some(properties.part),
            _ => None,
        };
        if let Some(owner) = owner {
            edges.push((owner, entity.id));
        }
    }
    Ok(edges)
}
