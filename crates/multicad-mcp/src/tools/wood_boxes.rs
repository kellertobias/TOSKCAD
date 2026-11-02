use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{commands::Session, model::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Face {
    Left,
    Right,
    Front,
    Back,
    Bottom,
    Top,
}
impl From<Face> for WoodBoxFace {
    fn from(value: Face) -> Self {
        match value {
            Face::Left => Self::Left,
            Face::Right => Self::Right,
            Face::Front => Self::Front,
            Face::Back => Self::Back,
            Face::Bottom => Self::Bottom,
            Face::Top => Self::Top,
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelIdentity {
    pub face: Face,
    pub feature: String,
    pub part: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Identities {
    pub group: String,
    pub panels: Vec<PanelIdentity>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WoodBoxArgs {
    pub expected_revision: u64,
    /// Existing recipe or proposed recipe UUID returned by Preview.
    pub recipe: Option<String>,
    pub name: String,
    /// Shared panel stock supplies both material and thickness.
    pub stock: String,
    pub width: Quantity,
    pub depth: Quantity,
    pub height: Quantity,
    /// Any nonempty subset of the six named faces; duplicates are refused.
    pub faces: Vec<Face>,
    pub origin: Option<[Quantity; 3]>,
    pub rotation: Option<Quantity>,
    /// Reuse Preview identities when creating. Existing recipes retain theirs.
    pub identities: Option<Identities>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WoodBoxInspectArgs {
    pub recipe: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WoodBoxRemoveArgs {
    pub expected_revision: u64,
    pub recipe: String,
}
fn request(args: WoodBoxArgs, session: &Session) -> Result<WoodBoxRequest, ToolError> {
    let recipe = args
        .recipe
        .as_deref()
        .map(id)
        .transpose()?
        .unwrap_or_default();
    let q = |value: &Quantity| {
        value
            .dimension(session.document(), session.parameters())
            .map(|d| d.0)
    };
    let mut definition = if session.document().get(recipe).is_some() {
        read_wood_box(session.document(), recipe)?
    } else {
        WoodBoxDefinition::new(id(&args.stock)?, length(1.), length(1.), length(1.))
    };
    definition.stock = id(&args.stock)?;
    definition.width = q(&args.width)?;
    definition.depth = q(&args.depth)?;
    definition.height = q(&args.height)?;
    let face_count = args.faces.len();
    definition.faces = args.faces.into_iter().map(Into::into).collect();
    if definition.faces.len() != face_count {
        return Err(ToolError::invalid("Each face may be listed only once"));
    }
    if let Some(origin) = args.origin {
        definition.origin = [q(&origin[0])?, q(&origin[1])?, q(&origin[2])?];
    }
    if let Some(rotation) = args.rotation {
        definition.rotation = q(&rotation)?;
    }
    if let Some(identities) = args.identities {
        let mut panels = std::collections::BTreeMap::new();
        for panel in identities.panels {
            if panels
                .insert(
                    panel.face.into(),
                    WoodBoxPanelIds {
                        feature: id(&panel.feature)?,
                        body: id(&panel.part)?,
                    },
                )
                .is_some()
            {
                return Err(ToolError::invalid("Duplicate face identity"));
            }
        }
        definition.group = id(&identities.group)?;
        definition.panels = panels;
    }
    Ok(WoodBoxRequest {
        id: recipe,
        name: args.name,
        definition,
    })
}
fn description(
    document: &multicad_core::document::Document,
    recipe: multicad_core::document::EntityId,
) -> Result<Value, ToolError> {
    let source = read_wood_box(document, recipe)?;
    let parameters = multicad_core::parameters::evaluate(document)?;
    let layout = resolve_wood_box(document, &parameters, &source)?;
    let panels: Vec<_> = source
        .panels
        .iter()
        .map(|(face, ids)| json!({"face":face,"feature":ids.feature,"part":ids.body}))
        .collect();
    Ok(
        json!({"recipe":recipe,"definition":source,"identities":{"group":source.group,"panels":panels},
        "outside_mm":layout.outside_mm,"usable_inner_mm":layout.clear_mm,"clear_origin_mm":layout.clear_origin_mm,
        "actual_world_bounds_mm":layout.actual_bounds,"thickness_mm":layout.thickness_mm,
        "corner_policy":"Sides span full depth and height; front/back fit between sides; top/bottom fit between vertical faces"}),
    )
}
fn finish(
    host: &dyn Host,
    call: &Request,
    revision: u64,
    change: WoodBoxChange,
    apply: bool,
) -> ToolResult {
    let recipe = change.target();
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(prepare_wood_box(
            target.session(),
            revision,
            owned.actor.clone(),
            &change,
        )?)
    })??;
    let model = compile(prepared.model.prepared.document(), revision, call)?;
    let mut panels = vec![];
    for (face, output) in &prepared.panels {
        let solid = model
            .solid(output.body)
            .ok_or_else(|| ToolError::invalid("Native wood box panel is missing"))?;
        panels.push(
            json!({"face":face,"part":output.body,"feature":output.feature,
            "volume_mm3":solid.volume_mm3().map_err(|e| ToolError::invalid(e.to_string()))?,"solid_count":solid.solid_count().map_err(|e| ToolError::invalid(e.to_string()))?}),
        );
    }
    let source = if prepared.model.prepared.document().get(recipe).is_some() {
        Some(description(prepared.model.prepared.document(), recipe)?)
    } else {
        None
    };
    let owned = call.clone();
    let revision = if apply {
        with(host, move |target| {
            commit(target, &owned, prepared.model.prepared)
        })??
    } else {
        revision
    };
    Ok(
        json!({"revision":revision,"committed":apply,"recipe":recipe,"box":source,"panels":panels,"native_geometry_checked":true}),
    )
}
fn set(host: &dyn Host, call: &Request, args: WoodBoxArgs, apply: bool) -> ToolResult {
    let revision = args.expected_revision;
    let request = with(host, move |target| {
        revision_guard(target, revision)?;
        request(args, target.session())
    })??;
    finish(
        host,
        call,
        revision,
        WoodBoxChange::Set(Box::new(request)),
        apply,
    )
}
pub(super) fn set_wood_box(host: &dyn Host, call: &Request, args: WoodBoxArgs) -> ToolResult {
    set(host, call, args, true)
}
pub(super) fn preview_wood_box(host: &dyn Host, call: &Request, args: WoodBoxArgs) -> ToolResult {
    set(host, call, args, false)
}
pub(super) fn remove_wood_box(
    host: &dyn Host,
    call: &Request,
    args: WoodBoxRemoveArgs,
) -> ToolResult {
    finish(
        host,
        call,
        args.expected_revision,
        WoodBoxChange::Remove(id(&args.recipe)?),
        true,
    )
}
pub(super) fn inspect_wood_box(
    host: &dyn Host,
    _: &Request,
    args: WoodBoxInspectArgs,
) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let ids = if let Some(recipe) = args.recipe {
        vec![id(&recipe)?]
    } else {
        wood_boxes(&document).map(|entity| entity.id).collect()
    };
    let boxes = ids
        .into_iter()
        .map(|id| description(&document, id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"revision":revision,"boxes":boxes}))
}
