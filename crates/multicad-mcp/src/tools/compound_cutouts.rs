use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::EntityId,
    model::{self, *},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoxArg {
    /// Retain this UUID when editing a constituent box.
    pub id: Option<String>,
    pub center: [Quantity; 2],
    pub width: Quantity,
    pub depth: Quantity,
    #[serde(default)]
    pub rotation_deg: f64,
    /// Both vertical limits or neither (inherit the complete cutter extent).
    pub bottom: Option<Quantity>,
    pub top: Option<Quantity>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiverArg {
    pub part: String,
    /// Optional generated identity returned by Preview. Retain it for Apply.
    pub feature: Option<String>,
    /// Explicit input producer in this Part's history. Retain it on edits.
    pub producer: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompoundCutoutArgs {
    pub expected_revision: u64,
    /// Existing source UUID to edit, or the proposed UUID returned by Preview.
    pub cutout: Option<String>,
    pub name: String,
    pub boxes: Vec<BoxArg>,
    /// Explicit walls, slabs or both; all are checked before one atomic commit.
    pub receivers: Vec<ReceiverArg>,
    /// Both limits or neither (follow the complete chosen receiver height).
    pub bottom: Option<Quantity>,
    pub top: Option<Quantity>,
    pub from_level: Option<String>,
    pub to_level: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompoundCutoutInspectArgs {
    pub cutout: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompoundCutoutRemoveArgs {
    pub expected_revision: u64,
    pub cutout: String,
}

fn request(
    args: CompoundCutoutArgs,
    session: &multicad_core::commands::Session,
) -> Result<CompoundCutoutRequest, ToolError> {
    let feature = args
        .cutout
        .as_deref()
        .map(id)
        .transpose()?
        .unwrap_or_default();
    let previous = args
        .cutout
        .as_ref()
        .filter(|_| session.document().get(feature).is_some())
        .map(|_| read_compound_cutout(session.document(), feature))
        .transpose()?;
    let q = |value: &Quantity| {
        value
            .dimension(session.document(), session.parameters())
            .map(|value| value.0)
    };
    let z = |bottom: Option<Quantity>, top: Option<Quantity>| match (bottom, top) {
        (None, None) => Ok(None),
        (Some(bottom), Some(top)) => Ok(Some(CutoutZRange {
            bottom: q(&bottom)?,
            top: q(&top)?,
        })),
        _ => Err(ToolError::invalid(
            "Specify both bottom and top, or neither",
        )),
    };
    let boxes = args
        .boxes
        .into_iter()
        .map(|item| {
            Ok(CompoundCutoutBox {
                id: item.id.as_deref().map(id).transpose()?.unwrap_or_default(),
                rectangle: PlanRectangle {
                    center: [q(&item.center[0])?, q(&item.center[1])?],
                    width: q(&item.width)?,
                    depth: q(&item.depth)?,
                    rotation_deg: item.rotation_deg,
                },
                vertical: z(item.bottom, item.top)?,
            })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    let receivers = args
        .receivers
        .into_iter()
        .map(|item| {
            let target = BodyInput {
                body: id(&item.part)?,
                producer: id(&item.producer)?,
            };
            let retained = previous
                .as_ref()
                .and_then(|source| {
                    source
                        .receivers
                        .iter()
                        .find(|old| old.target.body == target.body)
                })
                .map(|old| old.feature);
            let supplied = item.feature.as_deref().map(id).transpose()?;
            if matches!((retained,supplied),(Some(old),Some(new)) if old!=new) {
                return Err(ToolError::invalid(
                    "Retained receiver feature identity changed",
                ));
            }
            let feature = retained.or(supplied).unwrap_or_default();
            Ok(CompoundCutoutReceiver { target, feature })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    let levels = match (args.from_level, args.to_level) {
        (None, None) => None,
        (Some(from), to) => Some(CutoutLevelRange {
            from: id(&from)?,
            to: to.as_deref().map(id).transpose()?,
        }),
        (None, Some(_)) => return Err(ToolError::invalid("to_level requires from_level")),
    };
    Ok(CompoundCutoutRequest {
        feature,
        name: args.name,
        definition: CompoundCutoutDefinition {
            boxes,
            receivers,
            extent: z(args.bottom, args.top)?.map_or(
                CompoundCutoutExtent::ThroughReceivers,
                CompoundCutoutExtent::Fixed,
            ),
            levels,
        },
    })
}
fn finish(
    host: &dyn Host,
    call: &Request,
    revision: u64,
    change: CompoundCutoutChange,
    apply: bool,
) -> ToolResult {
    let source = change.target();
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(prepare_compound_cutout(
            target.session(),
            revision,
            owned.actor.clone(),
            &change,
        )?)
    })??;
    let model = compile(prepared.prepared.document(), revision, call)?;
    let mut outputs = vec![];
    for output in &prepared.outputs {
        let shape = model
            .snapshot()
            .shapes
            .get(&output.feature)
            .ok_or_else(|| ToolError::invalid("Native compound receiver proof is missing"))?;
        outputs.push(json!({"part":output.body,"feature":output.feature,"volume_mm3":shape.volume_mm3,"solid_count":shape.solid_count}));
    }
    let definition = if prepared.prepared.document().get(source).is_some() {
        Some(read_compound_cutout(prepared.prepared.document(), source)?)
    } else {
        None
    };
    let owned = call.clone();
    let revision = if apply {
        with(host, move |target| {
            commit(target, &owned, prepared.prepared)
        })??
    } else {
        revision
    };
    Ok(
        json!({"revision":revision,"committed":apply,"cutout":source,"definition":definition,"receivers":outputs,"native_geometry_checked":true}),
    )
}
pub(super) fn set_compound_cutout(
    host: &dyn Host,
    call: &Request,
    args: CompoundCutoutArgs,
) -> ToolResult {
    let revision = args.expected_revision;
    let request = with(host, move |target| {
        revision_guard(target, revision)?;
        request(args, target.session())
    })??;
    finish(
        host,
        call,
        revision,
        CompoundCutoutChange::Set(Box::new(request)),
        true,
    )
}
pub(super) fn preview_compound_cutout(
    host: &dyn Host,
    call: &Request,
    args: CompoundCutoutArgs,
) -> ToolResult {
    let revision = args.expected_revision;
    let request = with(host, move |target| {
        revision_guard(target, revision)?;
        request(args, target.session())
    })??;
    finish(
        host,
        call,
        revision,
        CompoundCutoutChange::Set(Box::new(request)),
        false,
    )
}
pub(super) fn remove_compound_cutout(
    host: &dyn Host,
    call: &Request,
    args: CompoundCutoutRemoveArgs,
) -> ToolResult {
    finish(
        host,
        call,
        args.expected_revision,
        CompoundCutoutChange::Remove(id(&args.cutout)?),
        true,
    )
}
pub(super) fn inspect_compound_cutout(
    host: &dyn Host,
    _: &Request,
    args: CompoundCutoutInspectArgs,
) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let ids = args
        .cutout
        .as_deref()
        .map(|value| id(value).map(|id| vec![id]))
        .unwrap_or_else(|| {
            Ok(model::compound_cutouts(&document)
                .map(|entity| entity.id)
                .collect())
        })?;
    let cutouts = ids
        .into_iter()
        .map(|id: EntityId| {
            Ok(json!({"cutout":id,"definition":read_compound_cutout(&document,id)?}))
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    Ok(json!({"revision":revision,"cutouts":cutouts}))
}
