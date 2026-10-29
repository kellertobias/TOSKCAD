//! Attached sheet-metal hardware. A mounting pattern on a sheet part names a
//! hardware preset, the face it attaches to and its points; the sheet cuts
//! the receiving holes from the pattern. Each purchased item of a pattern
//! (screws, washers, nuts, standoffs, inserts) is its own Part produced by a
//! `sheet_metal.hardware` feature that follows the sheet.
//!
//! Built-in metric dimensions are nominal values from the named standards.
//! Inserts and other catalogue items carry the designation and source the
//! user gives. No torque, strength or installation data is stored.
use super::*;

pub const HARDWARE_TYPE: &str = "sheet_metal.hardware";
pub const HARDWARE_SCHEMA: u32 = 1;
pub const HARDWARE_CAPABILITY: &str = "sheet_metal.hardware.1";
pub const MAX_PATTERNS: usize = 32;
pub const MAX_PATTERN_POINTS: usize = 64;

/// Metric coarse thread sizes with nominal dimensions from ISO 273 (medium
/// clearance hole), ISO 4762 (socket head cap screw), ISO 7089 (plain
/// washer) and ISO 4032 (hex nut).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Thread {
    M3,
    M4,
    M5,
    M6,
}
/// Nominal dimensions of one thread size, in millimeters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadData {
    pub diameter: f64,
    /// ISO 273 medium clearance hole.
    pub clearance: f64,
    /// ISO 4762 head diameter and height.
    pub head: [f64; 2],
    /// ISO 7089 inner diameter, outer diameter and thickness.
    pub washer: [f64; 3],
    /// ISO 4032 width across flats and height.
    pub nut: [f64; 2],
}
impl Thread {
    pub fn data(self) -> ThreadData {
        match self {
            Self::M3 => ThreadData {
                diameter: 3.,
                clearance: 3.4,
                head: [5.5, 3.],
                washer: [3.2, 7., 0.5],
                nut: [5.5, 2.4],
            },
            Self::M4 => ThreadData {
                diameter: 4.,
                clearance: 4.5,
                head: [7., 4.],
                washer: [4.3, 9., 0.8],
                nut: [7., 3.2],
            },
            Self::M5 => ThreadData {
                diameter: 5.,
                clearance: 5.5,
                head: [8.5, 5.],
                washer: [5.3, 10., 1.],
                nut: [8., 4.7],
            },
            Self::M6 => ThreadData {
                diameter: 6.,
                clearance: 6.6,
                head: [10., 6.],
                washer: [6.4, 12., 1.6],
                nut: [10., 5.2],
            },
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::M3 => "M3",
            Self::M4 => "M4",
            Self::M5 => "M5",
            Self::M6 => "M6",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum HardwarePreset {
    /// An ISO 4762 socket head cap screw through an ISO 273 clearance hole,
    /// with optional ISO 7089 washers and an ISO 4032 nut.
    Screw {
        thread: Thread,
        length: Dimension,
        #[serde(default)]
        washers: bool,
        #[serde(default)]
        nut: bool,
    },
    /// A hex standoff on the face, fastened through a clearance hole. Its
    /// length and width across flats are user values, not a standard.
    Standoff {
        thread: Thread,
        length: Dimension,
        across_flats: Dimension,
    },
    /// A catalogue insert or captive fastener: every dimension and the
    /// supported sheet thickness range come from the named source.
    Insert {
        designation: String,
        source: String,
        hole_diameter: Dimension,
        body_diameter: Dimension,
        height: Dimension,
        min_sheet: Dimension,
        max_sheet: Dimension,
    },
}
/// Which side of the face the hardware's head or body sits on: the material
/// side (along the face's thickness direction) or the base side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareSide {
    #[default]
    Material,
    Base,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountingPattern {
    pub preset: HardwarePreset,
    /// The face the hardware attaches to, in the same coordinates as
    /// cutouts.
    pub face: CutoutFace,
    #[serde(default)]
    pub side: HardwareSide,
    pub points: Vec<[Dimension; 2]>,
}
/// One purchased item of a mounting pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareItem {
    Screw,
    Washer,
    Nut,
    Standoff,
    Insert,
}
impl HardwareItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Screw => "screw",
            Self::Washer => "washer",
            Self::Nut => "nut",
            Self::Standoff => "standoff",
            Self::Insert => "insert",
        }
    }
}
impl HardwarePreset {
    pub fn items(&self) -> Vec<HardwareItem> {
        match self {
            Self::Screw { washers, nut, .. } => {
                let mut out = vec![HardwareItem::Screw];
                if *washers {
                    out.push(HardwareItem::Washer);
                }
                if *nut {
                    out.push(HardwareItem::Nut);
                }
                out
            }
            Self::Standoff { .. } => vec![HardwareItem::Standoff],
            Self::Insert { .. } => vec![HardwareItem::Insert],
        }
    }
    fn lengths(&self) -> Vec<&Dimension> {
        match self {
            Self::Screw { length, .. } => vec![length],
            Self::Standoff {
                length,
                across_flats,
                ..
            } => vec![length, across_flats],
            Self::Insert {
                hole_diameter,
                body_diameter,
                height,
                min_sheet,
                max_sheet,
                ..
            } => vec![hole_diameter, body_diameter, height, min_sheet, max_sheet],
        }
    }
    fn validate(&self) -> Result<(), DocumentError> {
        if let Self::Insert {
            designation,
            source,
            ..
        } = self
            && [designation, source]
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 256)
        {
            return Err(invalid(
                "An insert needs a designation and a catalogue source of 1–256 bytes",
            ));
        }
        for d in self.lengths() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
}
impl MountingPattern {
    pub(super) fn validate(&self, flanges: usize) -> Result<(), DocumentError> {
        if self.points.is_empty() || self.points.len() > MAX_PATTERN_POINTS {
            return Err(invalid(format!(
                "A mounting pattern needs 1–{MAX_PATTERN_POINTS} points"
            )));
        }
        if let CutoutFace::Flange { index } = self.face
            && index as usize >= flanges
        {
            return Err(invalid(format!(
                "A mounting pattern names missing flange {index}"
            )));
        }
        self.preset.validate()?;
        for p in &self.points {
            for d in p {
                check_dimension(d, Unit::Millimeter)?;
            }
        }
        Ok(())
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        let mut out = self.preset.lengths();
        out.extend(self.points.iter().flatten());
        out
    }
}

/// A resolved preset in millimeters, with its designation and source.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedPreset {
    Screw {
        thread: Thread,
        length_mm: f64,
        washers: bool,
        nut: bool,
    },
    Standoff {
        thread: Thread,
        length_mm: f64,
        across_flats_mm: f64,
    },
    Insert {
        designation: String,
        source: String,
        hole_mm: f64,
        body_mm: f64,
        height_mm: f64,
        sheet_range_mm: [f64; 2],
    },
}
impl ResolvedPreset {
    pub fn hole_mm(&self) -> f64 {
        match self {
            Self::Screw { thread, .. } | Self::Standoff { thread, .. } => thread.data().clearance,
            Self::Insert { hole_mm, .. } => *hole_mm,
        }
    }
    /// The designation and dimension source of one item.
    pub fn identity(&self, item: HardwareItem) -> (String, String) {
        match (self, item) {
            (
                Self::Screw {
                    thread, length_mm, ..
                },
                HardwareItem::Screw,
            ) => (
                format!("ISO 4762 {} × {length_mm}", thread.label()),
                "ISO 4762 nominal dimensions".into(),
            ),
            (Self::Screw { thread, .. }, HardwareItem::Washer) => (
                format!("ISO 7089 {}", thread.label()),
                "ISO 7089 nominal dimensions".into(),
            ),
            (Self::Screw { thread, .. }, HardwareItem::Nut) => (
                format!("ISO 4032 {}", thread.label()),
                "ISO 4032 nominal dimensions".into(),
            ),
            (
                Self::Standoff {
                    thread,
                    length_mm,
                    across_flats_mm,
                },
                _,
            ) => (
                format!(
                    "Hex standoff {} × {length_mm}, AF {across_flats_mm}",
                    thread.label()
                ),
                "User-specified dimensions".into(),
            ),
            (
                Self::Insert {
                    designation,
                    source,
                    ..
                },
                _,
            ) => (designation.clone(), source.clone()),
            (_, item) => (item.label().into(), "unmatched item".into()),
        }
    }
    /// How many of `item` one pattern point uses.
    pub fn per_point(&self, item: HardwareItem) -> usize {
        match (self, item) {
            (Self::Screw { nut, .. }, HardwareItem::Washer) => 1 + usize::from(*nut),
            _ => 1,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPattern {
    pub index: usize,
    pub preset: ResolvedPreset,
    pub face: CutoutFace,
    pub side: HardwareSide,
    pub points: Vec<[f64; 2]>,
}
pub(super) fn resolve_patterns(
    patterns: &[MountingPattern],
    mm: &dyn Fn(&Dimension) -> Result<f64, DocumentError>,
) -> Result<Vec<ResolvedPattern>, DocumentError> {
    let mut out = vec![];
    for (index, p) in patterns.iter().enumerate() {
        let what = |m: &str| invalid(format!("Mounting pattern {}: {m}", index + 1));
        let preset = match &p.preset {
            HardwarePreset::Screw {
                thread,
                length,
                washers,
                nut,
            } => ResolvedPreset::Screw {
                thread: *thread,
                length_mm: mm(length)?,
                washers: *washers,
                nut: *nut,
            },
            HardwarePreset::Standoff {
                thread,
                length,
                across_flats,
            } => ResolvedPreset::Standoff {
                thread: *thread,
                length_mm: mm(length)?,
                across_flats_mm: mm(across_flats)?,
            },
            HardwarePreset::Insert {
                designation,
                source,
                hole_diameter,
                body_diameter,
                height,
                min_sheet,
                max_sheet,
            } => ResolvedPreset::Insert {
                designation: designation.clone(),
                source: source.clone(),
                hole_mm: mm(hole_diameter)?,
                body_mm: mm(body_diameter)?,
                height_mm: mm(height)?,
                sheet_range_mm: [mm(min_sheet)?, mm(max_sheet)?],
            },
        };
        let positive = match &preset {
            ResolvedPreset::Screw { length_mm, .. } => *length_mm > 0.,
            ResolvedPreset::Standoff {
                thread,
                length_mm,
                across_flats_mm,
            } => *length_mm > 0. && *across_flats_mm > thread.data().clearance,
            ResolvedPreset::Insert {
                hole_mm,
                body_mm,
                height_mm,
                sheet_range_mm,
                ..
            } => {
                *hole_mm > 0.
                    && *body_mm > 0.
                    && *height_mm > 0.
                    && sheet_range_mm[0] >= 0.
                    && sheet_range_mm[1] >= sheet_range_mm[0]
            }
        };
        if !positive {
            return Err(what(
                "its sizes must be positive (a standoff wider than its hole, a valid sheet range)",
            ));
        }
        let points = p
            .points
            .iter()
            .map(|q| Ok([mm(&q[0])?, mm(&q[1])?]))
            .collect::<Result<_, DocumentError>>()?;
        out.push(ResolvedPattern {
            index,
            preset,
            face: p.face,
            side: p.side,
            points,
        });
    }
    Ok(out)
}

/// A hardware feature: one purchased item of one mounting pattern of a
/// sheet feature, as its own Part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareDefinition {
    pub output: BodyOutput,
    /// The sheet feature carrying the mounting pattern.
    pub sheet: EntityId,
    pub pattern: u32,
    pub item: HardwareItem,
}
impl HardwareDefinition {
    pub fn inputs(&self) -> Vec<Link> {
        vec![Link::new(self.sheet, "sheet")]
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.output.body.is_nil() || self.sheet.is_nil() {
            return Err(invalid("Hardware needs an output Part and a sheet feature"));
        }
        Ok(Definition {
            type_id: HARDWARE_TYPE.into(),
            schema_version: HARDWARE_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
}
pub fn hardware_features(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == HARDWARE_TYPE),
    )
}
pub fn read_hardware(
    document: &Document,
    feature: EntityId,
) -> Result<HardwareDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a hardware feature"));
    };
    if definition.type_id != HARDWARE_TYPE || definition.schema_version != HARDWARE_SCHEMA {
        return Err(invalid(format!(
            "Expected {HARDWARE_TYPE} schema {HARDWARE_SCHEMA}"
        )));
    }
    let value: HardwareDefinition =
        serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
    if inputs != &value.inputs() {
        return Err(invalid(
            "Hardware source and canonical input links disagree",
        ));
    }
    if let Some(entity) = document.get(value.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Hardware output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("Hardware output Part must contain its feature"));
        }
    }
    Ok(value)
}
/// Hardware features of a sheet feature: (pattern, item, feature, Part).
pub fn sheet_hardware(
    document: &Document,
    sheet: EntityId,
) -> Result<Vec<(u32, HardwareItem, EntityId, EntityId)>, DocumentError> {
    let mut out = vec![];
    for e in hardware_features(document) {
        let h = read_hardware(document, e.id)?;
        if h.sheet == sheet {
            out.push((h.pattern, h.item, e.id, h.output.body));
        }
    }
    out.sort();
    Ok(out)
}
pub(super) fn validate_hardware(document: &Document) -> Result<(), DocumentError> {
    let mut seen = std::collections::BTreeSet::new();
    for e in hardware_features(document) {
        let h = read_hardware(document, e.id)?;
        if !seen.insert((h.sheet, h.pattern, h.item)) {
            return Err(invalid(format!(
                "Mounting pattern {} has more than one {} Part",
                h.pattern + 1,
                h.item.label()
            )));
        }
        if document.get(h.sheet).is_some() {
            let sheet = read_sheet(document, h.sheet)?;
            let fits = sheet
                .fasteners
                .get(h.pattern as usize)
                .is_some_and(|p| p.preset.items().contains(&h.item));
            if !fits {
                return Err(invalid(format!(
                    "Hardware {} no longer matches mounting pattern {}",
                    h.item.label(),
                    h.pattern + 1
                )));
            }
        }
    }
    Ok(())
}
