use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::Placement,
    plants::{self, PlantKind, SizeBasis},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Tree,
    Shrub,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    PlannedMature,
    Current,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlantArgs {
    pub expected_revision: u64,
    /// Existing canonical symbol ownership record; omit to create a generic plant.
    pub record: Option<String>,
    pub kind: Option<Kind>,
    pub name: Option<String>,
    pub height: Option<Quantity>,
    pub spread: Option<Quantity>,
    pub position: Option<[Quantity; 3]>,
    pub rotation_deg: Option<f64>,
    pub size_basis: Option<Basis>,
    /// Null or omitted retains an existing label. Empty string removes it.
    pub label: Option<String>,
    pub plan_visible: Option<bool>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlantInspectArgs {
    pub record: String,
}
pub(super) fn inspect_plant(host: &dyn Host, _: &Request, args: PlantInspectArgs) -> ToolResult {
    let (doc, revision, parameters) = snapshot(host)?;
    let record = id(&args.record)?;
    let plant = plants::resolve(&doc, &parameters, record)?
        .ok_or_else(|| ToolError::invalid("Choose a plant instance"))?;
    let symbol = multicad_core::symbols::read_instance(&doc, record)?;
    let pose = multicad_core::model::resolve_instance_pose(&doc, plant.instance, &parameters)?;
    Ok(
        json!({"revision":revision,"record":record,"instance":plant.instance,"kind":plant.template.kind,"height_mm":plant.height_mm,"spread_mm":plant.spread_mm,"size_basis":plant.size_basis,"label":plant.label,"plan_visible":plant.plan_visible,"pose":pose,"pin":symbol.pin,"plan_sketch":plant.plan}),
    )
}
pub(super) fn set_plant(host: &dyn Host, call: &Request, args: PlantArgs) -> ToolResult {
    let (doc, revision, parameters) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            "Plant request is based on a stale revision",
        )
        .at_revision(revision));
    }
    if args
        .rotation_deg
        .is_some_and(|v| !v.is_finite() || v.abs() > 1e9)
    {
        return Err(ToolError::invalid(
            "Plant rotation must be finite and within modeling limits",
        ));
    }
    let record = args.record.as_deref().map(id).transpose()?;
    let previous = record
        .map(|id| plants::resolve(&doc, &parameters, id))
        .transpose()?
        .flatten();
    if record.is_some() && previous.is_none() {
        return Err(ToolError::invalid("Choose a plant instance"));
    }
    let kind = match args.kind {
        Some(Kind::Tree) => PlantKind::Tree,
        Some(Kind::Shrub) => PlantKind::Shrub,
        None => previous
            .as_ref()
            .map_or(PlantKind::Tree, |p| p.template.kind),
    };
    if previous.as_ref().is_some_and(|p| p.template.kind != kind) {
        return Err(ToolError::invalid(
            "An existing plant retains its pinned kind; place a new kind instead",
        ));
    }
    let mm = |q: &Quantity| -> Result<f64, ToolError> {
        q.dimension(&doc, &parameters)?
            .0
            .value_in(
                multicad_core::parameters::Unit::Millimeter,
                Some(&parameters),
            )
            .map_err(ToolError::invalid)
    };
    let height = args
        .height
        .as_ref()
        .map(mm)
        .transpose()?
        .unwrap_or_else(|| {
            previous
                .as_ref()
                .map_or(if kind == PlantKind::Tree { 6000. } else { 800. }, |p| {
                    p.height_mm
                })
        });
    let spread = args
        .spread
        .as_ref()
        .map(mm)
        .transpose()?
        .unwrap_or_else(|| {
            previous.as_ref().map_or(
                if kind == PlantKind::Tree {
                    4000.
                } else {
                    1000.
                },
                |p| p.spread_mm,
            )
        });
    let position = args
        .position
        .as_ref()
        .map(|p| Ok::<_, ToolError>([mm(&p[0])?, mm(&p[1])?, mm(&p[2])?]))
        .transpose()?;
    let basis = args
        .size_basis
        .map(|b| match b {
            Basis::PlannedMature => SizeBasis::PlannedMature,
            Basis::Current => SizeBasis::Current,
        })
        .unwrap_or_else(|| {
            previous
                .as_ref()
                .map_or(SizeBasis::PlannedMature, |p| p.size_basis)
        });
    let label = args
        .label
        .map(|s| if s.trim().is_empty() { None } else { Some(s) })
        .unwrap_or_else(|| previous.as_ref().and_then(|p| p.label.clone()));
    let visible = args
        .plan_visible
        .unwrap_or_else(|| previous.as_ref().is_none_or(|p| p.plan_visible));
    let actor = call.actor.clone();
    let (prepared, record, instance) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        if let Some(record) = record {
            let symbol = multicad_core::symbols::read_instance(session.document(), record)?;
            let pose = multicad_core::model::resolve_instance_pose(
                session.document(),
                symbol.instance,
                session.parameters(),
            )?;
            let move_by = if position.is_some() || args.rotation_deg.is_some() {
                plants::planar_move(
                    session.document(),
                    session.parameters(),
                    record,
                    position.unwrap_or(pose.translation_mm),
                    args.rotation_deg.unwrap_or_else(|| {
                        pose.rotation[1][0].atan2(pose.rotation[0][0]).to_degrees()
                    }),
                )?
            } else {
                None
            };
            let name = args.name.unwrap_or_else(|| {
                session
                    .document()
                    .get(symbol.instance)
                    .expect("checked plant")
                    .name
                    .clone()
            });
            let request = plants::PlantEditRequest {
                record,
                height_mm: height,
                spread_mm: spread,
                name,
                move_by,
                settings: plants::PlantInstance {
                    symbol_instance: record,
                    size_basis: basis,
                    label,
                    plan_visible: visible,
                },
            };
            Ok::<_, ToolError>((
                plants::prepare_edit_request(session, revision, actor, &request)?,
                record,
                symbol.instance,
            ))
        } else {
            let angle = args.rotation_deg.unwrap_or(0.).to_radians() / 2.;
            let mut request = plants::PlantPlaceRequest::new(
                session,
                plants::generic_definition(kind)?,
                args.name.unwrap_or_else(|| {
                    if kind == PlantKind::Tree {
                        "Tree"
                    } else {
                        "Shrub"
                    }
                    .into()
                }),
                height,
                spread,
                Placement {
                    translation_mm: position.unwrap_or([0.; 3]),
                    rotation_xyzw: [0., 0., angle.sin(), angle.cos()],
                },
            )?;
            request.settings.size_basis = basis;
            request.settings.label = label;
            request.settings.plan_visible = visible;
            Ok((
                plants::prepare_place(session, revision, actor, &request)?,
                request.placement.record,
                request.placement.instance,
            ))
        }
    })??;
    compile(prepared.document(), revision, call)?;
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({"revision":revision,"record":record,"instance":instance}))
}
