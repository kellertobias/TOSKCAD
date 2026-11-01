//! Plant semantics decorate pinned symbols; dimensions and placement remain in
//! their existing parameter/instance source, never a second drawing document.
use crate::{
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    symbols,
};
use serde::{Deserialize, Serialize};
mod prepare;
pub use prepare::{
    PlantEditRequest, PlantPlaceRequest, planar_move, prepare_edit, prepare_edit_request,
    prepare_place,
};
mod preset;
pub use preset::generic_definition;
pub const TEMPLATE_TYPE: &str = "architecture.plant_template";
pub const INSTANCE_TYPE: &str = "architecture.plant_instance";
pub const CAPABILITY: &str = "architecture.plant.1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlantKind {
    Tree,
    Shrub,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeBasis {
    #[default]
    PlannedMature,
    Current,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantTemplate {
    pub component: EntityId,
    pub kind: PlantKind,
    pub height_parameter: EntityId,
    pub spread_parameter: EntityId,
    pub plan: EntityId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantInstance {
    /// The canonical symbol instance source record, not another placement.
    pub symbol_instance: EntityId,
    pub size_basis: SizeBasis,
    pub label: Option<String>,
    pub plan_visible: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPlant {
    pub instance: EntityId,
    pub template: PlantTemplate,
    pub plan: EntityId,
    pub height_mm: f64,
    pub spread_mm: f64,
    pub size_basis: SizeBasis,
    pub label: Option<String>,
    pub plan_visible: bool,
}
fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
fn encode<T: Serialize>(kind: &str, value: &T) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: kind.into(),
        schema_version: 1,
        data: serde_json::to_value(value).map_err(|e| invalid(e.to_string()))?,
    })
}
fn decode<T: serde::de::DeserializeOwned>(
    source: &Definition,
    kind: &str,
) -> Result<T, DocumentError> {
    if source.type_id != kind || source.schema_version != 1 {
        return Err(invalid("Unsupported planting source/version"));
    }
    serde_json::from_value(source.data.clone()).map_err(|e| invalid(e.to_string()))
}
fn exact_links(actual: &[Link], expected: &[Link]) -> Result<(), DocumentError> {
    if actual.len() != expected.len() || expected.iter().any(|link| !actual.contains(link)) {
        return Err(invalid(
            "Planting source links do not match its typed payload",
        ));
    }
    Ok(())
}
impl PlantTemplate {
    pub fn links(&self) -> Vec<Link> {
        [
            (self.component, "component"),
            (self.height_parameter, "height"),
            (self.spread_parameter, "spread"),
            (self.plan, "plan"),
        ]
        .into_iter()
        .map(|(id, role)| Link::new(id, role))
        .collect()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        let ids = [
            self.component,
            self.height_parameter,
            self.spread_parameter,
            self.plan,
        ];
        if ids.iter().any(EntityId::is_nil)
            || ids
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != 4
        {
            return Err(invalid(
                "Plant template requires distinct component, dimension and plan identities",
            ));
        }
        encode(TEMPLATE_TYPE, self)
    }
    pub fn from_definition(source: &Definition) -> Result<Self, DocumentError> {
        let value: Self = decode(source, TEMPLATE_TYPE)?;
        value.definition()?;
        Ok(value)
    }
    pub fn entity(&self, id: EntityId) -> Result<Entity, DocumentError> {
        let mut entity = Entity::new(
            "Plant template",
            EntityData::Extension {
                definition: self.definition()?,
                inputs: self.links(),
            },
        );
        entity.id = id;
        Ok(entity)
    }
    fn check_document(&self, document: &Document) -> Result<(), DocumentError> {
        if !matches!(
            document.get(self.component).map(|e| &e.data),
            Some(EntityData::Part { .. } | EntityData::Assembly { .. })
        ) {
            return Err(invalid(
                "Plant template component is not a Part or assembly",
            ));
        }
        for id in [self.height_parameter, self.spread_parameter] {
            if !matches!(
                document.get(id).map(|e| &e.data),
                Some(EntityData::Parameter { unit, .. }) if Unit::parse(unit).is_some_and(|u| u.dimension() == Unit::Millimeter.dimension())
            ) {
                return Err(invalid(
                    "Plant dimensions must refer to canonical length parameters",
                ));
            }
        }
        if !matches!(
            document.get(self.plan).map(|e| &e.data),
            Some(EntityData::Sketch { .. })
        ) {
            return Err(invalid("Plant plan must be a canonical sketch"));
        }
        Ok(())
    }
}
impl PlantInstance {
    pub fn links(&self) -> Vec<Link> {
        vec![Link::new(self.symbol_instance, "symbol_instance")]
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.symbol_instance.is_nil()
            || self.label.as_ref().is_some_and(|label| {
                label.trim().is_empty()
                    || label.chars().count() > 200
                    || label.chars().any(char::is_control)
            })
        {
            return Err(invalid("Plant source or label is invalid"));
        }
        encode(INSTANCE_TYPE, self)
    }
    pub fn from_definition(source: &Definition) -> Result<Self, DocumentError> {
        let value: Self = decode(source, INSTANCE_TYPE)?;
        value.definition()?;
        Ok(value)
    }
    pub fn entity(&self, id: EntityId) -> Result<Entity, DocumentError> {
        let mut entity = Entity::new(
            "Plant instance settings",
            EntityData::Extension {
                definition: self.definition()?,
                inputs: self.links(),
            },
        );
        entity.id = id;
        Ok(entity)
    }
}
pub fn template_for(
    document: &Document,
    component: EntityId,
) -> Result<Option<PlantTemplate>, DocumentError> {
    let mut found = None;
    for entity in document.entities() {
        if let EntityData::Extension { definition, inputs } = &entity.data
            && definition.type_id == TEMPLATE_TYPE
        {
            let value = PlantTemplate::from_definition(definition)?;
            exact_links(inputs, &value.links())?;
            if value.component == component {
                if found.is_some() {
                    return Err(invalid("A component has duplicate plant template sources"));
                }
                value.check_document(document)?;
                found = Some(value);
            }
        }
    }
    Ok(found)
}
/// Reuse one checked definition/settings index for a whole drawing or browser.
pub struct Resolver<'a> {
    document: &'a Document,
    evaluated: &'a EvaluatedParameters,
    templates: std::collections::BTreeMap<EntityId, Option<PlantTemplate>>,
    settings: std::collections::BTreeMap<EntityId, PlantInstance>,
}
impl<'a> Resolver<'a> {
    pub fn new(
        document: &'a Document,
        evaluated: &'a EvaluatedParameters,
    ) -> Result<Self, DocumentError> {
        let mut templates = std::collections::BTreeMap::new();
        for (id, definition) in symbols::definitions(document)? {
            let template = template_for(&definition.template, definition.root)?;
            if template.as_ref().is_some_and(|template| {
                definition.plan.as_ref().map(|p| p.sketch) != Some(template.plan)
            }) {
                return Err(invalid(
                    "Plant definition and its linked plan do not share one source",
                ));
            }
            templates.insert(id, template);
        }
        let mut settings = std::collections::BTreeMap::new();
        for entity in document.entities() {
            if let EntityData::Extension { definition, inputs } = &entity.data
                && definition.type_id == INSTANCE_TYPE
            {
                let value = PlantInstance::from_definition(definition)?;
                exact_links(inputs, &value.links())?;
                let outer = symbols::read_instance(document, value.symbol_instance)?.instance;
                if settings.insert(outer, value).is_some() {
                    return Err(invalid("A plant instance has duplicate settings"));
                }
            }
        }
        Ok(Self {
            document,
            evaluated,
            templates,
            settings,
        })
    }
    pub fn resolve(&self, instance: EntityId) -> Result<Option<ResolvedPlant>, DocumentError> {
        let symbol = symbols::read_instance(self.document, instance)?;
        let Some(template) = self
            .templates
            .get(&symbol.definition)
            .and_then(Option::as_ref)
        else {
            return Ok(None);
        };
        let dimension = |id| {
            self.evaluated
                .value(symbol.ids[&id], Unit::Millimeter)
                .map_err(|e| invalid(e.to_string()))
        };
        let (height_mm, spread_mm) = (
            dimension(template.height_parameter)?,
            dimension(template.spread_parameter)?,
        );
        if [height_mm, spread_mm]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v > 1e7)
        {
            return Err(invalid(
                "Plant height and canopy/spread must be positive and within modeling limits",
            ));
        }
        let settings = self.settings.get(&symbol.instance);
        Ok(Some(ResolvedPlant {
            instance: symbol.instance,
            plan: symbol.ids[&template.plan],
            template: template.clone(),
            height_mm,
            spread_mm,
            size_basis: settings.map_or(SizeBasis::PlannedMature, |s| s.size_basis),
            label: settings.and_then(|s| s.label.clone()),
            plan_visible: settings.is_none_or(|s| s.plan_visible),
        }))
    }
}
pub fn resolve(
    document: &Document,
    evaluated: &EvaluatedParameters,
    instance: EntityId,
) -> Result<Option<ResolvedPlant>, DocumentError> {
    Resolver::new(document, evaluated)?.resolve(instance)
}
pub fn validate(document: &Document) -> Result<(), DocumentError> {
    let mut components = std::collections::BTreeSet::new();
    let mut instances = std::collections::BTreeSet::new();
    for entity in document.entities() {
        if let EntityData::Extension { definition, inputs } = &entity.data {
            match definition.type_id.as_str() {
                TEMPLATE_TYPE => {
                    let value = PlantTemplate::from_definition(definition)?;
                    exact_links(inputs, &value.links())?;
                    value.check_document(document)?;
                    if !components.insert(value.component) {
                        return Err(invalid("A component has duplicate plant template sources"));
                    }
                }
                INSTANCE_TYPE => {
                    let value = PlantInstance::from_definition(definition)?;
                    exact_links(inputs, &value.links())?;
                    if !matches!(document.get(value.symbol_instance).map(|e| &e.data), Some(EntityData::Extension { definition, .. }) if definition.type_id == symbols::INSTANCE_TYPE)
                    {
                        return Err(invalid(
                            "Plant settings require a canonical symbol ownership record",
                        ));
                    }
                    let symbol = symbols::read_instance(document, value.symbol_instance)?;
                    let definition = symbols::read_definition(document, symbol.definition)?;
                    let template = template_for(&definition.template, definition.root)?
                        .ok_or_else(|| invalid("Plant settings require a plant symbol"))?;
                    if definition.plan.as_ref().map(|p| p.sketch) != Some(template.plan) {
                        return Err(invalid(
                            "Plant definition and its linked plan do not share one source",
                        ));
                    }
                    if !instances.insert(value.symbol_instance) {
                        return Err(invalid("A plant instance has duplicate settings"));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}
pub fn required_capability(document: &Document) -> bool {
    let direct = |doc: &Document| {
        doc.entities().any(|e|matches!(&e.data,EntityData::Extension{definition,..} if matches!(definition.type_id.as_str(),TEMPLATE_TYPE|INSTANCE_TYPE)))
    };
    direct(document)
        || symbols::definitions(document).is_ok_and(|definitions| {
            definitions
                .iter()
                .any(|(_, definition)| direct(&definition.template))
        })
}
