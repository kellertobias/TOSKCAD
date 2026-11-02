//! Typed external fence control over the same source transaction as the editor.
use super::{Quantity, Request, ToolResult, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    constraint_sketch::{Dimension, binding::DimensionInput},
    document::{Document, EntityId},
    model::*,
    parameters::{BoundExpression, EvaluatedParameters, Unit},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PostArg {
    Rectangular { width: Quantity, depth: Quantity },
    Round { diameter: Quantity },
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutArg {
    Equal,
    ShortLast,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShortArg {
    Reject,
    Cut,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelWidthArg {
    FitBay,
    Purchased {
        width: Quantity,
        short_bay: ShortArg,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InfillKindArg {
    Panel {
        thickness: Quantity,
        width: PanelWidthArg,
    },
    Rail {
        depth: Quantity,
    },
    Pickets {
        width: Quantity,
        depth: Quantity,
        maximum_gap: Quantity,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InfillArg {
    /// Retain the returned recipe identity on edits. Omit for a new recipe.
    pub id: Option<String>,
    pub name: String,
    pub material: String,
    pub bottom: Quantity,
    pub height: Quantity,
    pub normal_offset: Quantity,
    pub construction: InfillKindArg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConstructionArg {
    pub height: Quantity,
    pub post: PostArg,
    pub post_material: String,
    pub preferred_post_spacing: Quantity,
    pub layout: LayoutArg,
    pub infills: Vec<InfillArg>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FenceTypeArgs {
    pub expected_revision: u64,
    /// Omit to create; supplying an existing ID explicitly edits all its runs.
    pub fence_type: Option<String>,
    pub name: String,
    pub construction: ConstructionArg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FenceTypeInspectArgs {
    pub fence_type: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VertexArg {
    /// Preserve the returned ID when moving a corner. Omit only for a new corner.
    pub id: Option<String>,
    pub point: [Quantity; 2],
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PositionArg {
    pub start: String,
    pub end: String,
    pub ordinal: u32,
    pub offset: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedArg {
    pub name: String,
    pub construction: ConstructionArg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FenceArgs {
    pub expected_revision: u64,
    /// Omit to create a run; supply its canonical run ID to edit it.
    pub fence: Option<String>,
    pub name: String,
    pub fence_type: String,
    /// Complete replacement path, with retained IDs for retained corners.
    pub vertices: Vec<VertexArg>,
    pub closed: bool,
    /// Complete replacement context; null means the world plane.
    pub level: Option<String>,
    pub base_z: Quantity,
    /// Complete replacement overrides; null inherits the construction.
    pub post_height: Option<Quantity>,
    pub post_spacing: Option<Quantity>,
    /// Omitted/null retains existing overrides; [] explicitly restores automatic positions.
    pub post_positions: Option<Vec<PositionArg>>,
    /// Explicit shared scope; can repair this run's overrides atomically.
    pub edit_shared_construction: Option<SharedArg>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FenceInspectArgs {
    pub fence: String,
}

pub(super) fn dimension(
    q: &Quantity,
    previous: Option<&Dimension>,
    doc: &Document,
    params: &EvaluatedParameters,
) -> Result<Dimension, ToolError> {
    match q.input()? {
        DimensionInput::Literal { .. } => Ok(q.dimension(doc, params)?.0),
        DimensionInput::Expression { text, unit } => {
            let value = Dimension::Expression {
                expression: BoundExpression::bind(
                    doc,
                    &text,
                    &unit,
                    previous.and_then(Dimension::expression),
                )?,
            };
            value
                .value_in(Unit::Millimeter, Some(params))
                .map_err(ToolError::invalid)?;
            Ok(value)
        }
    }
}
impl ConstructionArg {
    fn value(
        &self,
        doc: &Document,
        params: &EvaluatedParameters,
        old: Option<&FenceConstruction>,
    ) -> Result<FenceConstruction, ToolError> {
        if self.infills.len() > MAX_FENCE_INFILL_RECIPES {
            return Err(ToolError::invalid("Fence infill recipe budget exceeded"));
        }
        let q = |q: &Quantity, p: Option<&Dimension>| dimension(q, p, doc, params);
        let post = match &self.post {
            PostArg::Rectangular { width, depth } => {
                let previous = old.and_then(|p| {
                    if let FencePostProfile::Rectangular { width, depth } = &p.post {
                        Some((width, depth))
                    } else {
                        None
                    }
                });
                FencePostProfile::Rectangular {
                    width: q(width, previous.map(|p| p.0))?,
                    depth: q(depth, previous.map(|p| p.1))?,
                }
            }
            PostArg::Round { diameter } => FencePostProfile::Round {
                diameter: q(
                    diameter,
                    old.and_then(|p| {
                        if let FencePostProfile::Round { diameter } = &p.post {
                            Some(diameter)
                        } else {
                            None
                        }
                    }),
                )?,
            },
        };
        let mut infills = Vec::new();
        for row in &self.infills {
            let identity = row.id.as_deref().map(id).transpose()?.unwrap_or_default();
            let previous = old.and_then(|p| p.infills.iter().find(|i| i.id == identity));
            let construction = match &row.construction {
                InfillKindArg::Panel { thickness, width } => {
                    let p = previous.and_then(|p| {
                        if let FenceInfillKind::Panel { thickness, width } = &p.construction {
                            Some((thickness, width))
                        } else {
                            None
                        }
                    });
                    FenceInfillKind::Panel {
                        thickness: q(thickness, p.map(|p| p.0))?,
                        width: match width {
                            PanelWidthArg::FitBay => FencePanelWidth::FitBay,
                            PanelWidthArg::Purchased { width, short_bay } => {
                                FencePanelWidth::Purchased {
                                    width: q(
                                        width,
                                        p.and_then(|p| {
                                            if let FencePanelWidth::Purchased { width, .. } = p.1 {
                                                Some(width)
                                            } else {
                                                None
                                            }
                                        }),
                                    )?,
                                    short_bay: match short_bay {
                                        ShortArg::Reject => FenceShortPanelPolicy::Reject,
                                        ShortArg::Cut => FenceShortPanelPolicy::Cut,
                                    },
                                }
                            }
                        },
                    }
                }
                InfillKindArg::Rail { depth } => FenceInfillKind::Rail {
                    depth: q(
                        depth,
                        previous.and_then(|p| {
                            if let FenceInfillKind::Rail { depth } = &p.construction {
                                Some(depth)
                            } else {
                                None
                            }
                        }),
                    )?,
                },
                InfillKindArg::Pickets {
                    width,
                    depth,
                    maximum_gap,
                } => {
                    let p = previous.and_then(|p| {
                        if let FenceInfillKind::Pickets {
                            width,
                            depth,
                            maximum_gap,
                        } = &p.construction
                        {
                            Some((width, depth, maximum_gap))
                        } else {
                            None
                        }
                    });
                    FenceInfillKind::Pickets {
                        width: q(width, p.map(|p| p.0))?,
                        depth: q(depth, p.map(|p| p.1))?,
                        maximum_gap: q(maximum_gap, p.map(|p| p.2))?,
                    }
                }
            };
            infills.push(FenceInfill {
                id: identity,
                name: row.name.clone(),
                material: id(&row.material)?,
                bottom: q(&row.bottom, previous.map(|p| &p.bottom))?,
                height: q(&row.height, previous.map(|p| &p.height))?,
                normal_offset: q(&row.normal_offset, previous.map(|p| &p.normal_offset))?,
                construction,
            });
        }
        let value = FenceConstruction {
            height: q(&self.height, old.map(|p| &p.height))?,
            post,
            post_material: id(&self.post_material)?,
            preferred_post_spacing: q(
                &self.preferred_post_spacing,
                old.map(|p| &p.preferred_post_spacing),
            )?,
            layout: match self.layout {
                LayoutArg::Equal => FenceBayLayout::Equal,
                LayoutArg::ShortLast => FenceBayLayout::ShortLast,
            },
            infills,
        };
        value.resolve(doc, params)?;
        Ok(value)
    }
}
/// Replace canonical dimension objects with the public unit-bearing Quantity shape.
/// All semantic IDs are retained, including recipe and path vertex identities.
pub(super) fn quantities(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            if map.contains_key("value") && map.contains_key("unit") {
                return Value::Object(map);
            }
            if let Some(expression) = map.get("expression").and_then(Value::as_object) {
                return json!({"expression":expression["text"],"unit":expression["unit"]});
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, quantities(v))).collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(quantities).collect()),
        other => other,
    }
}
pub(super) fn set_fence_type(host: &dyn Host, call: &Request, args: FenceTypeArgs) -> ToolResult {
    let revision = args.expected_revision;
    let owned = call.clone();
    let (prepared, ty, value) = with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let s = target.session();
        let ty = args.fence_type.as_deref().map(id).transpose()?;
        let old = ty.map(|ty| read_fence_type(s.document(), ty)).transpose()?;
        let value = args
            .construction
            .value(s.document(), s.parameters(), old.as_ref())?;
        let request = FenceTypeRequest {
            id: ty.unwrap_or_default(),
            name: args.name,
            construction: value.clone(),
        };
        let ty = request.id;
        let change = if old.is_some() {
            FenceChange {
                edit_type: Some(request),
                ..Default::default()
            }
        } else {
            FenceChange {
                new_type: Some(request),
                ..Default::default()
            }
        };
        let prepared =
            prepare_fence_change(s, args.expected_revision, owned.actor.clone(), &change)?;
        Ok::<_, ToolError>((prepared, ty, value))
    })??;
    if !prepared.runs.is_empty() {
        compile(prepared.model.prepared.document(), revision, call)?;
    }
    let runs = prepared.runs.clone();
    let owned = call.clone();
    let revision = with(host, move |target| {
        super::commit(target, &owned, prepared.model.prepared)
    })??;
    Ok(
        json!({"revision":revision,"fence_type":ty,"construction":quantities(serde_json::to_value(value).expect("fence serializes")),"affected_runs":runs,"geometry":if runs.is_empty(){"Source definition only; geometry is checked when a run uses it"}else{"Native preflight passed"}}),
    )
}
pub(super) fn inspect_fence_type(
    host: &dyn Host,
    _: &Request,
    args: FenceTypeInspectArgs,
) -> ToolResult {
    let (doc, revision, params) = snapshot(host)?;
    let ty = id(&args.fence_type)?;
    let value = read_fence_type(&doc, ty)?;
    value.resolve(&doc, &params)?;
    Ok(
        json!({"revision":revision,"fence_type":ty,"name":doc.get(ty).expect("read type").name,"construction":quantities(serde_json::to_value(value).expect("fence serializes")),"affected_runs":fence_type_users(&doc,ty)?}),
    )
}
pub(super) fn set_fence(host: &dyn Host, call: &Request, args: FenceArgs) -> ToolResult {
    fence_change(host, call, args, false)
}
pub(super) fn preview_fence(host: &dyn Host, call: &Request, args: FenceArgs) -> ToolResult {
    fence_change(host, call, args, true)
}
fn fence_change(host: &dyn Host, call: &Request, args: FenceArgs, preview: bool) -> ToolResult {
    if args.vertices.len() > MAX_FENCE_PATH_POINTS
        || args
            .post_positions
            .as_ref()
            .is_some_and(|p| p.len() > MAX_FENCE_MEMBERS)
    {
        return Err(ToolError::invalid("Fence path/post budget exceeded"));
    }
    let revision = args.expected_revision;
    let owned = call.clone();
    let (prepared, run) = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        let doc = s.document();
        let params = s.parameters();
        let run = args.fence.as_deref().map(id).transpose()?;
        let old = run.map(|r| read_fence_run(doc, r)).transpose()?;
        let ty = id(&args.fence_type)?;
        let q = |q: &Quantity, p: Option<&Dimension>| dimension(q, p, doc, params);
        let mut vertices = Vec::new();
        for vertex in args.vertices {
            let identity = vertex
                .id
                .as_deref()
                .map(id)
                .transpose()?
                .unwrap_or_default();
            let previous = old
                .as_ref()
                .and_then(|p| p.path.vertices.iter().find(|v| v.id == identity));
            vertices.push(FenceVertex {
                id: identity,
                point: [
                    q(&vertex.point[0], previous.map(|p| &p.point[0]))?,
                    q(&vertex.point[1], previous.map(|p| &p.point[1]))?,
                ],
            });
        }
        let positions = match args.post_positions {
            None => old
                .as_ref()
                .map_or_else(Vec::new, |p| p.post_positions.clone()),
            Some(rows) => rows
                .into_iter()
                .map(|p| {
                    let post = FencePostKey::Interior {
                        start: id(&p.start)?,
                        end: id(&p.end)?,
                        ordinal: p.ordinal,
                    };
                    let previous = old
                        .as_ref()
                        .and_then(|r| r.post_positions.iter().find(|p| p.post == post));
                    Ok(FencePostPosition {
                        post,
                        offset: q(&p.offset, previous.map(|p| &p.offset))?,
                    })
                })
                .collect::<Result<Vec<_>, ToolError>>()?,
        };
        let source = FenceRunDefinition {
            fence_type: ty,
            group: old.as_ref().map_or_else(EntityId::new, |p| p.group),
            level: args.level.as_deref().map(id).transpose()?,
            path: FencePath {
                vertices,
                closed: args.closed,
                base_z: q(&args.base_z, old.as_ref().map(|p| &p.path.base_z))?,
            },
            overrides: FenceInstanceOverrides {
                post_height: args
                    .post_height
                    .as_ref()
                    .map(|q_| {
                        q(
                            q_,
                            old.as_ref().and_then(|p| p.overrides.post_height.as_ref()),
                        )
                    })
                    .transpose()?,
                post_spacing: args
                    .post_spacing
                    .as_ref()
                    .map(|q_| {
                        q(
                            q_,
                            old.as_ref().and_then(|p| p.overrides.post_spacing.as_ref()),
                        )
                    })
                    .transpose()?,
            },
            post_positions: positions,
        };
        let request = FenceRunRequest {
            id: run.unwrap_or_default(),
            name: args.name,
            definition: source,
        };
        let run = request.id;
        let edit_type = args
            .edit_shared_construction
            .map(|edit| {
                Ok::<_, ToolError>(FenceTypeRequest {
                    id: ty,
                    name: edit.name,
                    construction: edit.construction.value(
                        doc,
                        params,
                        Some(&read_fence_type(doc, ty)?),
                    )?,
                })
            })
            .transpose()?;
        let change = FenceChange {
            edit_type,
            run: Some(if old.is_some() {
                FenceRunChange::Edit(request)
            } else {
                FenceRunChange::Create(request)
            }),
            ..Default::default()
        };
        Ok::<_, ToolError>((
            prepare_fence_change(s, revision, owned.actor.clone(), &change)?,
            run,
        ))
    })??;
    let model = compile(prepared.model.prepared.document(), revision, call)?;
    let mut report = layout_json(prepared.model.prepared.document(), run)?;
    report["native_geometry_checked"] = json!(true);
    let members=prepared.members.iter().map(|(run,key,body)|json!({"fence":run,"key":key,"feature":body.feature,"part":body.body})).collect::<Vec<_>>();
    let affected = prepared.runs.clone();
    let part_count = model.snapshot().bodies.len();
    let committed = if preview {
        call.check()?;
        revision
    } else {
        let owned = call.clone();
        with(host, move |target| {
            super::commit(target, &owned, prepared.model.prepared)
        })??
    };
    Ok(
        json!({"revision":committed,"status":if preview{"previewed"}else{"applied"},"fence":run,"affected_runs":affected,"members":members,"model_part_count":part_count,"layout":report}),
    )
}
fn layout_json(doc: &Document, run: EntityId) -> ToolResult {
    let params = multicad_core::parameters::evaluate(doc)?;
    let source = read_fence_run(doc, run)?;
    let layout = resolve_fence_run(doc, &params, &source)?;
    Ok(
        json!({"definition":quantities(serde_json::to_value(&source).expect("source serializes")),"posts":layout.posts.iter().map(|p|json!({"key":p.key,"position_mm":p.position_mm,"direction":p.direction,"overridden":p.overridden})).collect::<Vec<_>>(),"bays":layout.bays.iter().map(|b|json!({"key":b.key,"center_spacing_mm":b.center_spacing_mm,"clear_width_mm":b.clear_width_mm})).collect::<Vec<_>>(),"infill_count":layout.infills.len(),"native_geometry_checked":false}),
    )
}
pub(super) fn inspect_fence(host: &dyn Host, _: &Request, args: FenceInspectArgs) -> ToolResult {
    let (doc, revision, _) = snapshot(host)?;
    let run = id(&args.fence)?;
    let layout = layout_json(&doc, run)?;
    let members=fence_members(&doc,run)?.into_iter().map(|(key,feature,part)|json!({"key":key,"feature":feature,"part":part,"name":doc.get(part).expect("member Part").name})).collect::<Vec<_>>();
    Ok(json!({"revision":revision,"fence":run,"layout":layout,"members":members}))
}
