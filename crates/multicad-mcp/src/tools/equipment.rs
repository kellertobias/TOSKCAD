use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    equipment::{self, CeilingDatum, HostPlacement, Kind, WallSide},
    model,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
macro_rules! families {
    ($($family:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum Family { $($family),+ }
        impl From<Family> for Kind { fn from(value: Family) -> Self { match value { $(Family::$family => Kind::$family),+ } } }
    }
}
families!(
    PowerOutletTypeF,
    DoublePowerOutletTypeF,
    TriplePowerOutletTypeF,
    DataOutletRj45,
    TvAntennaOutlet,
    Switch,
    CeilingLuminaire,
    WallLuminaire,
    LampHolderE27,
    Radiator,
    KitchenSink,
    Washbasin,
    Toilet,
    Bathtub,
    Shower,
    CookerOven,
    Cooktop,
    WashingMachine,
    Dryer,
    Refrigerator,
    Dishwasher,
    JunctionBox
);
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostArgs {
    Floor {
        xy: [Quantity; 2],
        height: Quantity,
        rotation_deg: Option<f64>,
    },
    Wall {
        wall: String,
        segment: usize,
        left_face: bool,
        along: Quantity,
        standoff: Option<Quantity>,
        height: Quantity,
        rotation_deg: Option<f64>,
    },
    Ceiling {
        xy: [Quantity; 2],
        ceiling_above_floor: Option<Quantity>,
        drop: Quantity,
        rotation_deg: Option<f64>,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentArgs {
    pub expected_revision: u64,
    pub record: Option<String>,
    pub family: Option<Family>,
    pub name: Option<String>,
    /// Width, depth and height with explicit length units.
    pub size: Option<[Quantity; 3]>,
    /// Required when creating. Omitted edits retain the floor reference.
    pub floor: Option<String>,
    /// Required when creating; replacing this intent repairs an unresolved host.
    pub host: Option<HostArgs>,
    pub built_in: Option<bool>,
    /// Omitted/null retains; empty text clears the label.
    pub label: Option<String>,
    pub service_front: Option<Quantity>,
    pub door_front: Option<Quantity>,
    pub service_visible: Option<bool>,
    pub door_visible: Option<bool>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentInspectArgs {
    pub record: String,
}
pub(super) fn equipment_profiles(_: &dyn Host, _: &Request, _: super::NoArgs) -> ToolResult {
    Ok(
        json!({"convention":equipment::PlanConvention::ToskCadGenericV1,"provenance":"Owned generic planning geometry and symbols; no manufacturer fit or installation certification", "families": Kind::ALL.into_iter().map(|kind| { let p = kind.preset(); json!({"family":kind,"name":p.name,"category":p.category,"size_mm":p.size_mm,"mount":p.mount,"mounting_height_mm":p.mounting_height_mm,"standalone":p.supports_standalone,"built_in":p.supports_built_in,"power_connector":p.power_connector}) }).collect::<Vec<_>>()}),
    )
}
pub(super) fn inspect_equipment(
    host: &dyn Host,
    _: &Request,
    args: EquipmentInspectArgs,
) -> ToolResult {
    let (doc, revision, parameters) = snapshot(host)?;
    let record = id(&args.record)?;
    let item = equipment::resolve(&doc, &parameters, record)?
        .ok_or_else(|| ToolError::invalid("Choose equipment"))?;
    let symbol = multicad_core::symbols::read_instance(&doc, record)?;
    let pose = model::resolve_instance_pose(&doc, item.instance, &parameters);
    Ok(
        json!({"revision":revision,"record":record,"instance":item.instance,"family":item.template.kind,"size_mm":item.size_mm,"settings":item.settings,"attachment":item.attachment,"pose":pose.as_ref().ok(),"host_error":pose.err().map(|e|e.to_string()),"pin":symbol.pin,"convention":item.template.convention}),
    )
}
pub(super) fn set_equipment(host: &dyn Host, call: &Request, args: EquipmentArgs) -> ToolResult {
    let (doc, revision, parameters) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            "Equipment request is based on a stale revision",
        )
        .at_revision(revision));
    }
    let record = args.record.as_deref().map(id).transpose()?;
    let previous = record
        .map(|record| equipment::resolve(&doc, &parameters, record))
        .transpose()?
        .flatten();
    if record.is_some() && previous.is_none() {
        return Err(ToolError::invalid("Choose equipment"));
    }
    let kind = args.family.map(Kind::from).unwrap_or_else(|| {
        previous
            .as_ref()
            .map_or(Kind::PowerOutletTypeF, |p| p.template.kind)
    });
    if previous.as_ref().is_some_and(|p| p.template.kind != kind) {
        return Err(ToolError::invalid(
            "Existing equipment retains its pinned family; place a new family instead",
        ));
    }
    let dimension =
        |q: &Quantity| -> Result<_, ToolError> { Ok(q.dimension(&doc, &parameters)?.0) };
    let mm = |q: &Quantity| -> Result<f64, ToolError> {
        dimension(q)?
            .value_in(
                multicad_core::parameters::Unit::Millimeter,
                Some(&parameters),
            )
            .map_err(ToolError::invalid)
    };
    let rotation = |v: Option<f64>| -> Result<_, ToolError> {
        let v = v.unwrap_or(0.);
        if !v.is_finite() || v.abs() > 1e9 {
            return Err(ToolError::invalid(
                "Equipment rotation must be finite and bounded",
            ));
        }
        Ok(model::angle(v))
    };
    let size = args
        .size
        .as_ref()
        .map(|s| Ok::<_, ToolError>([mm(&s[0])?, mm(&s[1])?, mm(&s[2])?]))
        .transpose()?
        .unwrap_or_else(|| {
            previous
                .as_ref()
                .map_or(kind.preset().size_mm, |p| p.size_mm)
        });
    let floor = args
        .floor
        .as_deref()
        .map(id)
        .transpose()?
        .or_else(|| {
            previous
                .as_ref()
                .and_then(|p| p.attachment.as_ref().map(|(_, a)| a.floor))
        })
        .ok_or_else(|| ToolError::invalid("Choose a labeled floor datum"))?;
    let placement = match args.host {
        Some(HostArgs::Floor {
            xy,
            height,
            rotation_deg,
        }) => HostPlacement::Floor {
            xy: [dimension(&xy[0])?, dimension(&xy[1])?],
            height: dimension(&height)?,
            rotation: rotation(rotation_deg)?,
        },
        Some(HostArgs::Ceiling {
            xy,
            ceiling_above_floor,
            drop,
            rotation_deg,
        }) => HostPlacement::Ceiling {
            xy: [dimension(&xy[0])?, dimension(&xy[1])?],
            datum: ceiling_above_floor
                .as_ref()
                .map(|q| dimension(q).map(|height| CeilingDatum::AboveFloor { height }))
                .transpose()?
                .unwrap_or(CeilingDatum::LevelUnderside),
            drop: dimension(&drop)?,
            rotation: rotation(rotation_deg)?,
        },
        Some(HostArgs::Wall {
            wall,
            segment,
            left_face,
            along,
            standoff,
            height,
            rotation_deg,
        }) => {
            let wall = id(&wall)?;
            let source = model::read_wall(&doc, wall)?;
            HostPlacement::Wall {
                wall,
                segment,
                point_count: source.path.len(),
                closed: source.closed,
                side: if left_face {
                    WallSide::Left
                } else {
                    WallSide::Right
                },
                along: dimension(&along)?,
                standoff: standoff
                    .as_ref()
                    .map(dimension)
                    .transpose()?
                    .unwrap_or_else(|| model::length(0.)),
                height: dimension(&height)?,
                rotation: rotation(rotation_deg)?,
            }
        }
        None => previous
            .as_ref()
            .and_then(|p| p.attachment.as_ref().map(|(_, a)| a.placement.clone()))
            .ok_or_else(|| ToolError::invalid("Choose a floor, wall or ceiling placement host"))?,
    };
    let service = args.service_front.as_ref().map(mm).transpose()?;
    let door = args.door_front.as_ref().map(mm).transpose()?;
    let actor = call.actor.clone();
    let (prepared, record, instance) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let name = args.name.unwrap_or_else(|| {
            previous
                .as_ref()
                .and_then(|p| session.document().get(p.instance))
                .map_or_else(|| kind.preset().name.into(), |e| e.name.clone())
        });
        let mut new = if previous.is_none() {
            Some(equipment::EquipmentPlaceRequest::new(
                session,
                equipment::generic_definition(kind)?,
                name.clone(),
                size,
                floor,
                placement.clone(),
            )?)
        } else {
            None
        };
        let (record, instance, mut settings, mut envelopes) = if let Some(previous) = previous {
            let envelopes = [
                previous.settings.service_envelope,
                previous.settings.door_envelope,
            ]
            .into_iter()
            .flatten()
            .map(|id| {
                model::read_envelope(session.document(), id)
                    .map(|value| equipment::EnvelopeSource { id, value })
            })
            .collect::<Result<Vec<_>, _>>()?;
            (
                previous.record,
                previous.instance,
                previous.settings,
                envelopes,
            )
        } else {
            let r = new.as_ref().expect("new request");
            (
                r.placement.record,
                r.placement.instance,
                r.settings.clone(),
                r.envelopes.clone(),
            )
        };
        if let Some(built_in) = args.built_in {
            settings.installation = if built_in {
                equipment::Installation::BuiltIn
            } else {
                equipment::Installation::Standalone
            };
        }
        if let Some(label) = args.label {
            settings.label = (!label.trim().is_empty()).then_some(label);
        }
        if let Some(visible) = args.service_visible {
            settings.service_visible = visible;
        }
        if let Some(visible) = args.door_visible {
            settings.door_visible = visible;
        }
        for (distance, label, slot) in [
            (
                service,
                "Generic service access",
                &mut settings.service_envelope,
            ),
            (
                door,
                "Generic door access envelope",
                &mut settings.door_envelope,
            ),
        ] {
            if let Some(distance) = distance {
                if !distance.is_finite() || !(0. ..=100_000.).contains(&distance) {
                    return Err(ToolError::invalid(
                        "Equipment access distance must be between0 and100000mm",
                    ));
                }
                if let Some(id) = *slot {
                    if let Some(envelope) = envelopes.iter_mut().find(|e| e.id == id) {
                        envelope.value.front = model::length(distance);
                    }
                } else {
                    let envelope = equipment::EnvelopeSource::front(instance, distance, label);
                    *slot = Some(envelope.id);
                    envelopes.push(envelope);
                }
            }
        }
        let attachment = equipment::Attachment {
            owner: instance,
            floor,
            placement,
        };
        let prepared = if let Some(request) = &mut new {
            request.settings = settings;
            request.envelopes = envelopes;
            equipment::prepare_place(session, revision, actor, request)?
        } else {
            equipment::prepare_edit(
                session,
                revision,
                actor,
                &equipment::EquipmentEditRequest {
                    record,
                    name,
                    size_mm: size,
                    attachment,
                    settings,
                    envelopes,
                },
            )?
        };
        Ok::<_, ToolError>((prepared, record, instance))
    })??;
    compile(prepared.document(), revision, call)?;
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({"revision":revision,"record":record,"instance":instance}))
}
