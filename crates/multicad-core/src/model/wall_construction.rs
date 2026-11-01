//! Versioned construction records in the existing wall-type catalog. Schema 1
//! remains the uniform wall wire format; schema 2 stores the layer stack itself.
use super::{
    WALL_TYPE_KIND, WallLayerReference, WallLayerStack, WallTypeDefinition,
    common::{number, parameter_links, revision},
    source::{check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link, Mutation},
    parameters::Unit,
};
use serde::{Deserialize, Serialize};

pub const LAYERED_WALL_TYPE_SCHEMA: u32 = 2;
pub const LAYERED_WALL_TYPE_CAPABILITY: &str = "architecture.wall_type.2";
pub const LAYERED_WALL_CAPABILITY: &str = "architecture.wall.2";
pub const LAYERED_WALL_SCHEMA: u32 = 2;

fn is_false(value: &bool) -> bool {
    !*value
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallLayerPlacement {
    /// None inherits the type's reference boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<WallLayerReference>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flipped: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWallLayerConstruction {
    pub stack: super::ResolvedWallLayerStack,
    pub bands: Vec<super::WallLayerBand>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayeredWallTypeDefinition {
    pub layers: WallLayerStack,
    pub height: Dimension,
    #[serde(default)]
    pub reference: WallLayerReference,
}
impl LayeredWallTypeDefinition {
    pub fn resolve_layers(
        &self,
        document: &Document,
        parameters: &crate::parameters::EvaluatedParameters,
        placement: &WallLayerPlacement,
    ) -> Result<ResolvedWallLayerConstruction, DocumentError> {
        self.validate()?;
        let stack = self.layers.resolve(document, parameters)?;
        let bands = stack.bands(
            placement.reference.unwrap_or(self.reference),
            placement.flipped,
        )?;
        Ok(ResolvedWallLayerConstruction { stack, bands })
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.layers.validate()?;
        check_dimension(&self.height, Unit::Millimeter)?;
        if matches!(&self.height, Dimension::Literal {value,..} if *value <= 0.) {
            return Err(invalid("Wall type height must be positive"));
        }
        if let WallLayerReference::Boundary { layer, .. } = self.reference
            && !self.layers.layers.iter().any(|item| item.id == layer)
        {
            return Err(invalid(
                "The wall type reference layer is missing; choose another boundary",
            ));
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: WALL_TYPE_KIND.into(),
            schema_version: LAYERED_WALL_TYPE_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = self.layers.inputs();
        for link in parameter_links(vec![&self.height]) {
            if !links.contains(&link) {
                links.push(link);
            }
        }
        links
    }
    fn validate_values(&self, session: &Session) -> Result<(), DocumentError> {
        self.layers
            .resolve(session.document(), session.parameters())?;
        if number(&self.height, Unit::Millimeter, session.parameters())? <= 0. {
            return Err(invalid("Wall type height must be positive"));
        }
        Ok(())
    }
}

/// A single canonical catalog, with explicit legacy compatibility. Uniform-only
/// APIs continue to reject a layered record rather than flattening its source.
#[derive(Clone, Debug, PartialEq)]
pub enum WallTypeConstruction {
    Uniform(WallTypeDefinition),
    Layered(LayeredWallTypeDefinition),
}
pub fn read_wall_construction(
    document: &Document,
    id: EntityId,
) -> Result<WallTypeConstruction, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose a wall construction type"));
    };
    if definition.type_id != WALL_TYPE_KIND {
        return Err(invalid("Choose a wall construction type"));
    }
    match definition.schema_version {
        1 => super::read_wall_type(document, id).map(WallTypeConstruction::Uniform),
        LAYERED_WALL_TYPE_SCHEMA => {
            let value: LayeredWallTypeDefinition = serde_json::from_value(definition.data.clone())
                .map_err(|e| invalid(e.to_string()))?;
            value.validate()?;
            let expected = value.inputs();
            if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link))
            {
                return Err(invalid(
                    "Layered wall construction and canonical input links disagree",
                ));
            }
            for layer in &value.layers.layers {
                if let Some(material) = layer.construction.material()
                    && !matches!(
                        document.get(material).map(|e| &e.data),
                        Some(EntityData::Material { .. })
                    )
                {
                    return Err(invalid(format!(
                        "Material for wall layer {} is missing or is not a material",
                        layer.name
                    )));
                }
            }
            Ok(WallTypeConstruction::Layered(value))
        }
        _ => Err(invalid("Unsupported wall construction schema")),
    }
}
pub fn prepare_layered_wall_type(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    value: &LayeredWallTypeDefinition,
) -> Result<(EntityId, Prepared), CommandError> {
    revision(session, expected_revision)?;
    value.validate_values(session)?;
    super::wall::insert_extension(
        session,
        expected_revision,
        actor,
        "Create layered wall type",
        name,
        value.definition()?,
        value.inputs(),
    )
}
pub fn prepare_layered_wall_type_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    value: &LayeredWallTypeDefinition,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    if !matches!(
        read_wall_construction(session.document(), id)?,
        WallTypeConstruction::Layered(_)
    ) {
        return Err(invalid(
            "Choose a layered wall type; uniform type conversion must be explicit",
        )
        .into());
    }
    value.validate_values(session)?;
    let mut entity = session
        .document()
        .get(id)
        .ok_or(DocumentError::Missing(id))?
        .clone();
    entity.data = EntityData::Extension {
        definition: value.definition()?,
        inputs: value.inputs(),
    };
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit layered wall type".into(),
        edits: vec![Mutation::Replace(entity)],
    })
}
