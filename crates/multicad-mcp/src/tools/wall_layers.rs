//! Source-owned layered construction tools. Wall creation/editing uses the same
//! canonical transaction as the native editor and checks exact geometry before commit.
use super::{Quantity, Request, ToolResult, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    constraint_sketch::{Dimension, binding::DimensionInput},
    document::{Document, EntityId},
    model::{
        self, LayeredWallTypeDefinition, WallLayer, WallLayerKind, WallLayerReference,
        WallLayerSide, WallLayerStack,
    },
    parameters::{BoundExpression, EvaluatedParameters, Unit},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerKindArg {
    Solid {
        thickness: Quantity,
        material: String,
    },
    Cavity {
        thickness: Quantity,
    },
    Coating {
        material: String,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerArg {
    /// Retain the returned ID when editing a layer. Omit only for a new layer.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    pub construction: LayerKindArg,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayerSideArg {
    Outside,
    Inside,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerReferenceArg {
    Outside,
    Center,
    Inside,
    Boundary { layer: String, side: LayerSideArg },
}
impl LayerReferenceArg {
    pub(super) fn value(&self) -> Result<WallLayerReference, ToolError> {
        Ok(match self {
            Self::Outside => WallLayerReference::Outside,
            Self::Center => WallLayerReference::Center,
            Self::Inside => WallLayerReference::Inside,
            Self::Boundary { layer, side } => WallLayerReference::Boundary {
                layer: id(layer)?,
                side: match side {
                    LayerSideArg::Outside => WallLayerSide::Outside,
                    LayerSideArg::Inside => WallLayerSide::Inside,
                },
            },
        })
    }
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WallEndArg {
    Start,
    End,
}
#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JunctionStyleArg {
    #[default]
    FlushButt,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WallJunctionArg {
    pub end: WallEndArg,
    /// Active direct architectural wall feature, not its Part ID.
    pub host_wall: String,
    pub host_segment: usize,
    /// Omit for a new join; retained joins keep their recorded host topology.
    #[serde(default)]
    pub host_point_count: Option<usize>,
    #[serde(default)]
    pub host_closed: Option<bool>,
    #[serde(default)]
    pub style: JunctionStyleArg,
}
impl WallJunctionArg {
    fn value(
        &self,
        document: &Document,
        previous: Option<&model::WallJunction>,
    ) -> Result<model::WallJunction, ToolError> {
        let host_wall = id(&self.host_wall)?;
        let host = model::read_wall(document, host_wall)?;
        let previous =
            previous.filter(|j| j.host_wall == host_wall && j.host_segment == self.host_segment);
        Ok(model::WallJunction {
            end: match self.end {
                WallEndArg::Start => model::WallEnd::Start,
                WallEndArg::End => model::WallEnd::End,
            },
            host_wall,
            host_segment: self.host_segment,
            host_point_count: self
                .host_point_count
                .or_else(|| previous.map(|j| j.host_point_count))
                .unwrap_or(host.path.len()),
            host_closed: self
                .host_closed
                .or_else(|| previous.map(|j| j.host_closed))
                .unwrap_or(host.closed),
            style: match self.style {
                JunctionStyleArg::FlushButt => model::WallJunctionStyle::FlushButt,
            },
        })
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerPlacementArg {
    /// Omit or null to inherit the construction reference.
    #[serde(default)]
    pub reference: Option<LayerReferenceArg>,
    #[serde(default)]
    pub flipped: bool,
    /// Omit to retain existing joins; an explicit empty list removes them.
    /// Only perpendicular flush-butt T joins are supported, with no wrapping.
    #[serde(default)]
    pub junctions: Option<Vec<WallJunctionArg>>,
}
impl LayerPlacementArg {
    pub(super) fn value(
        &self,
        previous: Option<&model::WallLayerPlacement>,
        document: &Document,
    ) -> Result<model::WallLayerPlacement, ToolError> {
        if self.junctions.as_ref().is_some_and(|j| j.len() > 2) {
            return Err(ToolError::invalid("A wall has at most two joined ends"));
        }
        Ok(model::WallLayerPlacement {
            reference: self
                .reference
                .as_ref()
                .map(LayerReferenceArg::value)
                .transpose()?,
            flipped: self.flipped,
            junctions: self
                .junctions
                .as_ref()
                .map(|j| {
                    j.iter()
                        .map(|junction| {
                            junction.value(
                                document,
                                previous.and_then(|p| {
                                    p.junctions.iter().find(|j| {
                                        matches!(
                                            (junction.end, j.end),
                                            (WallEndArg::Start, model::WallEnd::Start)
                                                | (WallEndArg::End, model::WallEnd::End)
                                        )
                                    })
                                }),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_else(|| previous.map(|p| p.junctions.clone()).unwrap_or_default()),
        })
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerConstructionArg {
    /// Ordered Outside to Inside. Cavities have real width; coatings have zero volume.
    pub layers: Vec<LayerArg>,
    pub height: Quantity,
    #[serde(default)]
    pub reference: Option<LayerReferenceArg>,
}
fn dimension(
    q: &Quantity,
    previous: Option<&Dimension>,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<Dimension, ToolError> {
    match q.input()? {
        DimensionInput::Literal { .. } => Ok(q.dimension(document, evaluated)?.0),
        DimensionInput::Expression { text, unit } => {
            let value = Dimension::Expression {
                expression: BoundExpression::bind(
                    document,
                    &text,
                    &unit,
                    previous.and_then(Dimension::expression),
                )?,
            };
            value
                .value_in(Unit::Millimeter, Some(evaluated))
                .map_err(ToolError::invalid)?;
            Ok(value)
        }
    }
}
impl LayerConstructionArg {
    pub(super) fn value(
        &self,
        document: &Document,
        evaluated: &EvaluatedParameters,
        previous: Option<&LayeredWallTypeDefinition>,
    ) -> Result<LayeredWallTypeDefinition, ToolError> {
        if self.layers.len() > model::MAX_WALL_LAYERS {
            return Err(ToolError::invalid(
                "Wall construction layer budget exceeded",
            ));
        }
        let mut layers = Vec::new();
        for layer in &self.layers {
            let identity = layer.id.as_deref().map(id).transpose()?.unwrap_or_default();
            let previous =
                previous.and_then(|ty| ty.layers.layers.iter().find(|l| l.id == identity));
            let thickness = |q: &Quantity| {
                dimension(
                    q,
                    previous.and_then(|l| l.construction.thickness()),
                    document,
                    evaluated,
                )
            };
            layers.push(WallLayer {
                id: identity,
                name: layer.name.clone(),
                role: layer.role.clone(),
                construction: match &layer.construction {
                    LayerKindArg::Solid {
                        thickness: q,
                        material,
                    } => WallLayerKind::Solid {
                        thickness: thickness(q)?,
                        material: id(material)?,
                    },
                    LayerKindArg::Cavity { thickness: q } => WallLayerKind::Cavity {
                        thickness: thickness(q)?,
                    },
                    LayerKindArg::Coating { material } => WallLayerKind::Coating {
                        material: id(material)?,
                    },
                },
            });
        }
        let value = LayeredWallTypeDefinition {
            layers: WallLayerStack { layers },
            height: dimension(
                &self.height,
                previous.map(|p| &p.height),
                document,
                evaluated,
            )?,
            reference: self
                .reference
                .as_ref()
                .map(LayerReferenceArg::value)
                .transpose()?
                .unwrap_or_default(),
        };
        value.validate()?;
        value.layers.resolve(document, evaluated)?;
        Ok(value)
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedConstructionArg {
    pub name: String,
    pub construction: LayerConstructionArg,
}
impl SharedConstructionArg {
    pub(super) fn value(
        &self,
        ty: EntityId,
        document: &Document,
        evaluated: &EvaluatedParameters,
    ) -> Result<model::WallConstructionEdit, ToolError> {
        let model::WallTypeConstruction::Layered(previous) =
            model::read_wall_construction(document, ty)?
        else {
            return Err(ToolError::invalid(
                "Shared layer editing requires a layered construction",
            ));
        };
        Ok(model::WallConstructionEdit {
            id: ty,
            name: self.name.clone(),
            value: self
                .construction
                .value(document, evaluated, Some(&previous))?,
        })
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLayeredWallTypeArgs {
    pub expected_revision: u64,
    pub name: String,
    pub construction: LayerConstructionArg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditLayeredWallTypeArgs {
    pub expected_revision: u64,
    pub wall_type: String,
    pub name: String,
    pub construction: LayerConstructionArg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectWallConstructionArgs {
    pub wall_type: String,
}
fn quantity_json(value: &Dimension) -> Value {
    match value {
        Dimension::Literal { value, unit } => json!({"value":value,"unit":unit}),
        Dimension::Expression { expression } => {
            json!({"expression":expression.text,"unit":expression.unit})
        }
    }
}
pub(super) fn construction_json(value: &LayeredWallTypeDefinition) -> Value {
    let layers:Vec<_>=value.layers.layers.iter().map(|layer|json!({"id":layer.id,"name":layer.name,"role":layer.role,"construction":match &layer.construction {
        WallLayerKind::Solid {thickness,material}=>json!({"kind":"solid","thickness":quantity_json(thickness),"material":material}),
        WallLayerKind::Cavity {thickness}=>json!({"kind":"cavity","thickness":quantity_json(thickness)}),
        WallLayerKind::Coating {material}=>json!({"kind":"coating","material":material}),
    }})).collect();
    json!({"layers":layers,"height":quantity_json(&value.height),"reference":value.reference})
}
pub(super) fn create_layered_wall_type(
    host: &dyn Host,
    call: &Request,
    args: CreateLayeredWallTypeArgs,
) -> ToolResult {
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let value = args
            .construction
            .value(session.document(), session.parameters(), None)?;
        let (ty, prepared) = model::prepare_layered_wall_type(
            session,
            args.expected_revision,
            call.actor.clone(),
            &args.name,
            &value,
        )?;
        let revision = super::commit(target, &call, prepared)?;
        Ok(
            json!({"revision":revision,"wall_type":ty,"construction":construction_json(&value),"geometry":"Source definition only; native geometry is checked when a wall uses it."}),
        )
    })?
}
pub(super) fn edit_layered_wall_type(
    host: &dyn Host,
    call: &Request,
    args: EditLayeredWallTypeArgs,
) -> ToolResult {
    let ty = id(&args.wall_type)?;
    let call_owned = call.clone();
    let (prepared, revision, users) = with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let session = target.session();
        let edit = SharedConstructionArg {
            name: args.name,
            construction: args.construction,
        }
        .value(ty, session.document(), session.parameters())?;
        let users = model::wall_construction_users(session.document(), ty)?;
        let prepared = model::prepare_wall_construction_edit(
            session,
            args.expected_revision,
            call_owned.actor.clone(),
            &edit,
        )?;
        Ok::<_, ToolError>((prepared, args.expected_revision, users))
    })??;
    compile(prepared.document(), revision, call)?;
    let call = call.clone();
    let committed = with(host, move |target| super::commit(target, &call, prepared))??;
    Ok(
        json!({"revision":committed,"wall_type":ty,"affected_walls":users.into_iter().map(|u|u.feature).collect::<Vec<_>>()}),
    )
}
pub(super) fn inspect_wall_construction(
    host: &dyn Host,
    _: &Request,
    args: InspectWallConstructionArgs,
) -> ToolResult {
    let ty = id(&args.wall_type)?;
    let (document, revision, evaluated) = snapshot(host)?;
    let users = model::wall_construction_users(&document, ty)?;
    match model::read_wall_construction(&document, ty)? {
        model::WallTypeConstruction::Uniform(value) => Ok(
            json!({"revision":revision,"wall_type":ty,"kind":"uniform","definition":value,"affected_walls":users.into_iter().map(|u|u.feature).collect::<Vec<_>>()}),
        ),
        model::WallTypeConstruction::Layered(value) => {
            let stack = value.layers.resolve(&document, &evaluated)?;
            Ok(
                json!({"revision":revision,"wall_type":ty,"kind":"layered","construction":construction_json(&value),"definition":value,"thickness_mm":stack.thickness_mm,"affected_walls":users.into_iter().map(|u|u.feature).collect::<Vec<_>>() }),
            )
        }
    }
}
