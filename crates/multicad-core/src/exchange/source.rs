use super::*;
use crate::{
    document::{Definition, Entity, Link},
    materials::{TagQuery, checked_tags},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScopeRoot {
    Project,
    Assembly { assembly: EntityId },
    Explicit { entities: Vec<EntityId> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub root: ScopeRoot,
    pub tags: Option<TagQuery>,
}
impl Default for Scope {
    fn default() -> Self {
        Self {
            root: ScopeRoot::Project,
            tags: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRef {
    pub book: EntityId,
    pub page: EntityId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pages {
    None,
    Project,
    Selected { pages: Vec<PageRef> },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Millimeter,
    Centimeter,
    Meter,
    Inch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HierarchyMode {
    SeparateParts,
    PreserveAssembly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameHandling {
    FileNamesOnly,
    ProductNames,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AidPolicy {
    IncludeAll,
    ExcludeRemovable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshFormat {
    Obj,
    Stl,
    ThreeMf,
    Glb,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshQuality {
    pub chord_deviation_mm: f64,
    pub angular_deviation_deg: f64,
}
impl Default for MeshQuality {
    fn default() -> Self {
        Self {
            chord_deviation_mm: 0.1,
            angular_deviation_deg: 15.,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    pub cut: String,
    pub bend: String,
    pub construction: String,
    pub annotation: String,
}
impl Default for Layers {
    fn default() -> Self {
        Self {
            cut: "CUT".into(),
            bend: "BEND".into(),
            construction: "CONSTRUCTION".into(),
            annotation: "ANNOTATION".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecipeOutput {
    Step {
        units: LengthUnit,
        hierarchy: HierarchyMode,
        names: NameHandling,
        aids: AidPolicy,
    },
    Mesh {
        format: MeshFormat,
        units: LengthUnit,
        hierarchy: HierarchyMode,
        names: NameHandling,
        aids: AidPolicy,
        quality: MeshQuality,
    },
    PlanarDxf {
        units: LengthUnit,
        layers: Layers,
        aids: AidPolicy,
    },
    DrawingDxf {
        units: LengthUnit,
        layers: Layers,
    },
    Pdf,
}
impl RecipeOutput {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Step { .. } => "step",
            Self::Mesh { format, .. } => match format {
                MeshFormat::Obj => "obj",
                MeshFormat::Stl => "stl",
                MeshFormat::ThreeMf => "3mf",
                MeshFormat::Glb => "glb",
            },
            Self::PlanarDxf { .. } | Self::DrawingDxf { .. } => "dxf",
            Self::Pdf => "pdf",
        }
    }
    pub fn uses_pages(&self) -> bool {
        matches!(self, Self::DrawingDxf { .. } | Self::Pdf)
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if let Self::Mesh { quality, .. } = self
            && (!quality.chord_deviation_mm.is_finite()
                || quality.chord_deviation_mm <= 0.
                || !quality.angular_deviation_deg.is_finite()
                || quality.angular_deviation_deg <= 0.
                || quality.angular_deviation_deg > 180.)
        {
            return Err(DocumentError::Invalid("Invalid mesh quality".into()));
        }
        if let Self::PlanarDxf { layers, .. } | Self::DrawingDxf { layers, .. } = self
            && [
                &layers.cut,
                &layers.bend,
                &layers.construction,
                &layers.annotation,
            ]
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 255 || s.chars().any(char::is_control))
        {
            return Err(DocumentError::Invalid(
                "Invalid manufacturing layer names".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub id: EntityId,
    pub name: String,
    pub output: RecipeOutput,
}
impl Recipe {
    pub fn step(name: impl Into<String>) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            output: RecipeOutput::Step {
                units: LengthUnit::Millimeter,
                hierarchy: HierarchyMode::SeparateParts,
                names: NameHandling::FileNamesOnly,
                aids: AidPolicy::IncludeAll,
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RevisionRule {
    ContentHash,
    Label(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingRule {
    pub template: String,
    pub revision: RevisionRule,
}
impl Default for NamingRule {
    fn default() -> Self {
        Self {
            template: "{part}_{id}_{occurrence}_{recipe}_{revision}".into(),
            revision: RevisionRule::ContentHash,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDefinition {
    pub scope: Scope,
    pub pages: Pages,
    pub recipes: Vec<Recipe>,
    pub naming: NamingRule,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planar_faces: Vec<PlanarFace>,
}
impl Default for SetDefinition {
    fn default() -> Self {
        Self {
            scope: Default::default(),
            pages: Pages::None,
            recipes: vec![Recipe::step("STEP")],
            naming: Default::default(),
            planar_faces: vec![],
        }
    }
}
impl SetDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let ids_valid = |v: &[EntityId]| {
            !v.is_empty()
                && v.len() <= 4096
                && v.iter().all(|i| !i.is_nil())
                && v.iter().collect::<BTreeSet<_>>().len() == v.len()
        };
        match &self.scope.root {
            ScopeRoot::Assembly { assembly } if assembly.is_nil() => {
                return Err(DocumentError::Invalid("Assembly ID is nil".into()));
            }
            ScopeRoot::Explicit { entities } if !ids_valid(entities) => {
                return Err(DocumentError::Invalid(
                    "Select 1–4096 distinct source IDs".into(),
                ));
            }
            _ => {}
        }
        if let Some(q) = &self.scope.tags {
            checked_tags(&q.all)?;
            checked_tags(&q.any)?;
            checked_tags(&q.none)?;
            if q.all.is_empty() && q.any.is_empty() && q.none.is_empty() {
                return Err(DocumentError::Invalid(
                    "Empty tag query; choose the whole scope explicitly instead".into(),
                ));
            }
        }
        if let Pages::Selected { pages } = &self.pages
            && (pages.is_empty()
                || pages.len() > 1024
                || pages.iter().any(|p| p.book.is_nil() || p.page.is_nil())
                || pages.iter().collect::<BTreeSet<_>>().len() != pages.len())
        {
            return Err(DocumentError::Invalid(
                "Select distinct drawing page identities".into(),
            ));
        }
        if self.recipes.is_empty()
            || self.recipes.len() > 16
            || self
                .recipes
                .iter()
                .any(|r| r.id.is_nil() || r.name.trim().is_empty() || r.name.len() > 128)
            || self
                .recipes
                .iter()
                .map(|r| r.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.recipes.len()
        {
            return Err(DocumentError::Invalid(
                "Use 1–16 named recipes with distinct IDs".into(),
            ));
        }
        for r in &self.recipes {
            r.output.validate()?;
        }
        if self.planar_faces.len() > MAX_PLANAR_BINDINGS {
            return Err(DocumentError::Invalid(
                "At most 128 explicit planar face bindings are supported".into(),
            ));
        }
        let mut bindings = BTreeSet::new();
        let mut references = BTreeSet::new();
        for face in &self.planar_faces {
            face.validate()?;
            if !self
                .recipes
                .iter()
                .any(|r| r.id == face.recipe && matches!(r.output, RecipeOutput::PlanarDxf { .. }))
                || !bindings.insert((face.recipe, &face.subject))
                || !references.insert(face.reference)
            {
                return Err(DocumentError::Invalid("Planar face bindings need a matching DXF recipe and unique subject/reference identities".into()));
            }
        }
        if self.naming.template.is_empty()
            || self.naming.template.len() > 512
            || self
                .naming
                .template
                .chars()
                .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
        {
            return Err(DocumentError::Invalid(
                "Naming rule must be a filename template without path separators".into(),
            ));
        }
        let mut rest = self.naming.template.as_str();
        while let Some(i) = rest.find('{') {
            if rest[..i].contains('}') {
                return Err(DocumentError::Invalid("Unmatched naming brace".into()));
            }
            rest = &rest[i + 1..];
            let Some(end) = rest.find('}') else {
                return Err(DocumentError::Invalid("Unclosed naming placeholder".into()));
            };
            if ![
                "project",
                "set",
                "part",
                "id",
                "occurrence",
                "recipe",
                "revision",
                "page",
            ]
            .contains(&&rest[..end])
            {
                return Err(DocumentError::Invalid(format!(
                    "Unknown naming placeholder {}",
                    &rest[..end]
                )));
            }
            rest = &rest[end + 1..];
        }
        if rest.contains('}') {
            return Err(DocumentError::Invalid("Unmatched naming brace".into()));
        }
        if let RevisionRule::Label(s) = &self.naming.revision
            && (s.trim().is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        {
            return Err(DocumentError::Invalid(
                "Revision label must contain 1–128 printable bytes".into(),
            ));
        }
        Ok(())
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = vec![];
        match &self.scope.root {
            ScopeRoot::Project => {}
            ScopeRoot::Assembly { assembly } => links.push(Link::new(*assembly, "scope_assembly")),
            ScopeRoot::Explicit { entities } => {
                links.extend(entities.iter().map(|id| Link::new(*id, "scope_source")))
            }
        };
        if let Pages::Selected { pages } = &self.pages {
            let books: BTreeSet<_> = pages.iter().map(|p| p.book).collect();
            links.extend(books.into_iter().map(|id| Link::new(id, "drawing_book")));
        }
        links
    }
    pub fn entity(&self, id: EntityId, name: impl Into<String>) -> Result<Entity, DocumentError> {
        self.entity_with_geometry(id, name, &[])
    }
    pub fn entity_with_geometry(
        &self,
        id: EntityId,
        name: impl Into<String>,
        geometry: &[Link],
    ) -> Result<Entity, DocumentError> {
        self.validate()?;
        let mut entity = Entity::new(
            name,
            EntityData::ExportSet {
                definition: Definition {
                    type_id: TYPE_ID.into(),
                    schema_version: if self.planar_faces.is_empty() { 1 } else { 2 },
                    data: serde_json::to_value(self)
                        .map_err(|e| DocumentError::Invalid(e.to_string()))?,
                },
                inputs: super::planar::checked_inputs(self, geometry)?,
            },
        );
        entity.id = id;
        Ok(entity)
    }
}
pub fn read(document: &Document, id: EntityId) -> Result<SetDefinition, ExportError> {
    let entity = document
        .get(id)
        .ok_or_else(|| ExportError::new(ErrorKind::Missing, Some(id), "Export set is missing"))?;
    let EntityData::ExportSet { definition, inputs } = &entity.data else {
        return Err(ExportError::new(
            ErrorKind::Invalid,
            Some(id),
            "Selected entity is not an export set",
        ));
    };
    if definition.type_id != TYPE_ID || ![1, 2].contains(&definition.schema_version) {
        return Err(ExportError::new(
            ErrorKind::Unsupported,
            Some(id),
            "Unsupported export-set provider or schema",
        ));
    }
    let value: SetDefinition = serde_json::from_value(definition.data.clone())
        .map_err(|e| ExportError::new(ErrorKind::Invalid, Some(id), e.to_string()))?;
    value.validate()?;
    if definition.schema_version == 1 && !value.planar_faces.is_empty() {
        return Err(ExportError::new(
            ErrorKind::Invalid,
            Some(id),
            "Explicit planar faces require export-set schema 2",
        ));
    }
    let geometry = inputs
        .iter()
        .filter(|l| l.role == PLANAR_FACE_ROLE)
        .cloned()
        .collect::<Vec<_>>();
    if *inputs != super::planar::checked_inputs(&value, &geometry)? {
        return Err(ExportError::new(
            ErrorKind::Invalid,
            Some(id),
            "Export-set reference links differ from saved scope",
        ));
    }
    Ok(value)
}
