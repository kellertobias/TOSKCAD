use crate::document::{Definition, Document, DocumentError, Entity, EntityData, EntityId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const TYPE_ID: &str = "drawings.book";
pub const CAPABILITY: &str = "drawings.book.1";
pub const MAX_PAGES: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperSize {
    A4,
    A3,
    A2,
    A1,
    A0,
    Custom { width_mm: f64, height_mm: f64 },
}
impl PaperSize {
    pub fn dimensions(self) -> [f64; 2] {
        match self {
            Self::A4 => [210., 297.],
            Self::A3 => [297., 420.],
            Self::A2 => [420., 594.],
            Self::A1 => [594., 841.],
            Self::A0 => [841., 1189.],
            Self::Custom {
                width_mm,
                height_mm,
            } => [width_mm, height_mm],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Portrait,
    Landscape,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    FirstAngle,
    ThirdAngle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scale {
    pub numerator: u32,
    pub denominator: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSettings {
    pub paper: PaperSize,
    pub orientation: Orientation,
    pub projection: Projection,
    pub scale: Scale,
    pub margin_mm: f64,
    pub title_block: bool,
}
impl Default for PageSettings {
    fn default() -> Self {
        Self {
            paper: PaperSize::A4,
            orientation: Orientation::Landscape,
            projection: Projection::FirstAngle,
            scale: Scale {
                numerator: 1,
                denominator: 1,
            },
            margin_mm: 10.,
            title_block: true,
        }
    }
}
impl PageSettings {
    pub fn dimensions(&self) -> [f64; 2] {
        let [a, b] = self.paper.dimensions();
        match self.orientation {
            Orientation::Portrait => [a.min(b), a.max(b)],
            Orientation::Landscape => [a.max(b), a.min(b)],
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingInfo {
    pub title: String,
    pub author: String,
    pub project: String,
    pub revision: String,
    pub checker: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextNote {
    pub id: EntityId,
    pub position_mm: [f64; 2],
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: EntityId,
    pub number: String,
    pub settings: PageSettings,
    pub info: DrawingInfo,
    pub notes: Vec<TextNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<super::DrawingView>,
    /// Dimensions, notes, leaders, weld symbols, tables and drafting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<super::Annotation>,
}
impl Page {
    pub fn new(number: impl Into<String>) -> Self {
        Self {
            id: EntityId::new(),
            number: number.into(),
            settings: PageSettings::default(),
            info: DrawingInfo::default(),
            notes: vec![],
            views: vec![],
            annotations: vec![],
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingBook {
    pub pages: Vec<Page>,
}
impl DrawingBook {
    pub fn entity(&self, name: impl Into<String>) -> Result<Entity, DocumentError> {
        self.validate()?;
        Ok(Entity::new(
            name,
            EntityData::Drawing {
                definition: Definition {
                    type_id: TYPE_ID.into(),
                    schema_version: if super::has_views(self) { 2 } else { 1 },
                    data: serde_json::to_value(self).map_err(invalid)?,
                },
                inputs: super::view_inputs(self),
            },
        ))
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.pages.len() > MAX_PAGES {
            return Err(invalid("A drawing book supports at most 256 pages"));
        }
        let mut ids = BTreeSet::new();
        for (index, page) in self.pages.iter().enumerate() {
            if page.id.is_nil() || !ids.insert(page.id) {
                return Err(invalid("Duplicate or nil drawing page identity"));
            }
            text(&page.number, 64)?;
            if page.number.trim().is_empty() {
                return Err(invalid("Page number is required"));
            }
            for value in [
                &page.info.title,
                &page.info.author,
                &page.info.project,
                &page.info.revision,
                &page.info.checker,
            ] {
                text(value, 4096)?;
            }
            let [w, h] = page.settings.dimensions();
            let margin = page.settings.margin_mm;
            if [w, h]
                .iter()
                .any(|v| !v.is_finite() || !(50.0..=2000.).contains(v))
                || !margin.is_finite()
                || margin < 0.
                || margin * 2. + 20. > w.min(h)
                || page.settings.scale.numerator == 0
                || page.settings.scale.denominator == 0
            {
                return Err(invalid(
                    "Paper needs finite dimensions, printable margins and a positive scale",
                ));
            }
            if page.notes.len() > 1024 {
                return Err(invalid("Too many text notes on a drawing page"));
            }
            for note in &page.notes {
                if note.id.is_nil() || !ids.insert(note.id) {
                    return Err(invalid("Duplicate or nil drawing content identity"));
                }
                text(&note.text, 4096)?;
                if note.position_mm.iter().any(|v| !v.is_finite())
                    || note.position_mm[0] < margin
                    || note.position_mm[1] < margin
                    || note.position_mm[0] >= w - margin
                    || note.position_mm[1] >= h - margin
                {
                    return Err(invalid("Text note must start inside the printable page"));
                }
            }
            super::layout_page(page, index + 1, self.pages.len())?;
            if page.views.len() > 256 {
                return Err(invalid("Too many drawing views on a page"));
            }
            for view in &page.views {
                view.validate()?;
                if !ids.insert(view.id) {
                    return Err(invalid("Duplicate drawing content identity"));
                }
                if view.position_mm[0] < margin
                    || view.position_mm[0] >= w - margin
                    || view.position_mm[1] < margin
                    || view.position_mm[1] >= h - margin
                {
                    return Err(invalid(
                        "Drawing view anchor must be inside the printable page",
                    ));
                }
            }
            if page.annotations.len() > super::MAX_ANNOTATIONS {
                return Err(invalid("Too many annotations on a drawing page"));
            }
            for annotation in &page.annotations {
                annotation.validate(page)?;
                if !ids.insert(annotation.id) {
                    return Err(invalid("Duplicate drawing content identity"));
                }
            }
        }
        Ok(())
    }
}
pub fn read(document: &Document, id: EntityId) -> Result<DrawingBook, DocumentError> {
    let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
    let EntityData::Drawing { definition, inputs } = &entity.data else {
        return Err(invalid("Select a drawing book"));
    };
    if definition.type_id != TYPE_ID || !matches!(definition.schema_version, 1 | 2) {
        return Err(invalid("Drawing page provider/version is unavailable"));
    }
    let book: DrawingBook = serde_json::from_value(definition.data.clone()).map_err(invalid)?;
    book.validate()?;
    if (definition.schema_version == 1 && super::has_views(&book))
        || *inputs != super::view_inputs(&book)
    {
        return Err(invalid(
            "Drawing view schema or source dependencies do not match authored views",
        ));
    }
    Ok(book)
}
pub fn books(document: &Document) -> impl Iterator<Item = EntityId> + '_ {
    document.entities().filter(|entity|matches!(&entity.data,EntityData::Drawing{definition,..}if definition.type_id==TYPE_ID)).map(|entity|entity.id)
}
pub fn validate(document: &Document) -> Result<(), DocumentError> {
    let mut identities: BTreeSet<_> = document.entities().map(|entity| entity.id).collect();
    identities.insert(document.id());
    for id in books(document) {
        let book = read(document, id)?;
        for page in book.pages {
            for nested in std::iter::once(page.id)
                .chain(page.notes.iter().map(|note| note.id))
                .chain(page.views.iter().map(|view| view.id))
                .chain(page.annotations.iter().map(|a| a.id))
            {
                if !identities.insert(nested) {
                    return Err(invalid(
                        "Drawing page/content identity collides with another source identity",
                    ));
                }
            }
        }
    }
    Ok(())
}
pub fn required_capability(document: &Document) -> bool {
    books(document).next().is_some()
}
pub(super) fn invalid(error: impl std::fmt::Display) -> DocumentError {
    DocumentError::Invalid(error.to_string())
}
fn text(value: &str, max: usize) -> Result<(), DocumentError> {
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err(invalid(
            "Drawing text is too long or contains unsupported control characters",
        ))
    } else {
        Ok(())
    }
}
