//! Canonical source transactions; callers native-preflight before Apply.
use super::*;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Mutation, Placement},
    symbols::{self, SymbolDefinition, SymbolPlaceRequest, SymbolValue},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentPlaceRequest {
    pub definition: SymbolDefinition,
    pub placement: SymbolPlaceRequest,
    pub attachment_id: EntityId,
    pub attachment: Attachment,
    pub settings_id: EntityId,
    pub settings: InstanceSettings,
    #[serde(default)]
    pub envelopes: Vec<EnvelopeSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeSource {
    pub id: EntityId,
    pub value: crate::model::EnvelopeDefinition,
}
impl EnvelopeSource {
    pub fn front(target: EntityId, distance_mm: f64, label: &str) -> Self {
        use crate::model::{EnvelopeDefinition, length};
        Self {
            id: EntityId::new(),
            value: EnvelopeDefinition {
                target,
                front: length(distance_mm),
                back: length(0.),
                left: length(0.),
                right: length(0.),
                top: length(0.),
                label: label.into(),
            },
        }
    }
}
fn envelope_edits(
    document: &Document,
    settings: &InstanceSettings,
    owner: EntityId,
    sources: &[EnvelopeSource],
) -> Result<Vec<Mutation>, DocumentError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut edits = vec![];
    for source in sources {
        if source.value.target != owner
            || ![settings.service_envelope, settings.door_envelope].contains(&Some(source.id))
            || !seen.insert(source.id)
        {
            return Err(invalid(
                "Equipment may edit only its distinct owned service and door envelopes",
            ));
        }
        let entity = source.value.entity(
            source.id,
            if settings.service_envelope == Some(source.id) {
                "Equipment service access"
            } else {
                "Equipment door access"
            },
        )?;
        if document.get(source.id).is_some() {
            if crate::model::read_envelope(document, source.id)?.target != owner {
                return Err(invalid("Equipment envelope belongs to another object"));
            }
            edits.push(Mutation::Replace(entity));
        } else {
            edits.push(Mutation::Insert(entity));
        }
    }
    Ok(edits)
}

fn candidate(
    session: &Session,
    revision: u64,
    actor: Actor,
    label: &str,
    doc: &Document,
) -> Result<Prepared, CommandError> {
    let mut edits = vec![];
    for entity in doc.entities() {
        match session.document().get(entity.id) {
            None => edits.push(Mutation::Insert(entity.clone())),
            Some(old) if old != entity => edits.push(Mutation::Replace(entity.clone())),
            _ => {}
        }
    }
    if session
        .document()
        .entities()
        .any(|e| doc.get(e.id).is_none())
    {
        return Err(
            invalid("Equipment transaction unexpectedly removed a canonical entity").into(),
        );
    }
    session.prepare(Request {
        expected_revision: revision,
        actor,
        label: label.into(),
        edits,
    })
}

fn overrides(
    template: &Template,
    size_mm: [f64; 3],
) -> Result<BTreeMap<EntityId, SymbolValue>, DocumentError> {
    if size_mm
        .iter()
        .any(|v| !v.is_finite() || *v < 1. || *v > 100_000.)
    {
        return Err(invalid(
            "Equipment dimensions must be between 1 and 100000 mm",
        ));
    }
    Ok([
        template.width_parameter,
        template.depth_parameter,
        template.height_parameter,
    ]
    .into_iter()
    .zip(size_mm)
    .map(|(id, value)| {
        (
            id,
            SymbolValue {
                value,
                unit: "mm".into(),
            },
        )
    })
    .collect())
}

pub fn check_mount(template: &Template, attachment: &Attachment) -> Result<(), DocumentError> {
    let valid = matches!(
        (template.mount, &attachment.placement),
        (
            MountClass::WallCenter | MountClass::WallBottom,
            HostPlacement::Wall { .. }
        ) | (MountClass::CeilingTop, HostPlacement::Ceiling { .. })
            | (
                MountClass::FloorBottom | MountClass::CounterTop,
                HostPlacement::Floor { .. }
            )
    );
    if !valid {
        return Err(invalid(
            "Choose the wall, ceiling or floor host required by this equipment type",
        ));
    }
    Ok(())
}

impl EquipmentPlaceRequest {
    pub fn new(
        session: &Session,
        definition: SymbolDefinition,
        name: String,
        size_mm: [f64; 3],
        floor: EntityId,
        host: HostPlacement,
    ) -> Result<Self, CommandError> {
        let template = template_for(&definition.template, definition.root)?
            .ok_or_else(|| invalid("Choose an equipment definition"))?;
        let imported = symbols::prepare_import(
            session,
            session.revision(),
            Actor::local(crate::commands::Channel::Mcp),
            &definition,
        )?;
        let placement = SymbolPlaceRequest::new(
            imported.prepared.document(),
            imported.definition,
            name,
            overrides(&template, size_mm)?,
            Placement::default(),
        )?;
        let attachment = Attachment {
            owner: placement.instance,
            floor,
            placement: host,
        };
        check_mount(&template, &attachment)?;
        attachment.resolve(session.document(), session.parameters())?;
        let settings = InstanceSettings {
            symbol_instance: placement.record,
            installation: if template.supports_standalone {
                Installation::Standalone
            } else {
                Installation::BuiltIn
            },
            label: None,
            service_envelope: None,
            door_envelope: None,
            service_visible: false,
            door_visible: false,
        };
        let mut settings = settings;
        let mut envelopes = vec![];
        let preset = template.kind.preset();
        for (distance, label, slot) in [
            (
                preset.service_front_mm,
                "Generic service access",
                &mut settings.service_envelope,
            ),
            (
                preset.door_front_mm,
                "Generic door access envelope",
                &mut settings.door_envelope,
            ),
        ] {
            if distance > 0. {
                let envelope = EnvelopeSource::front(placement.instance, distance, label);
                *slot = Some(envelope.id);
                envelopes.push(envelope);
            }
        }
        Ok(Self {
            definition,
            placement,
            attachment_id: EntityId::new(),
            attachment,
            settings_id: EntityId::new(),
            settings,
            envelopes,
        })
    }
}

pub fn prepare_place(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &EquipmentPlaceRequest,
) -> Result<Prepared, CommandError> {
    if request.attachment.owner != request.placement.instance
        || request.settings.symbol_instance != request.placement.record
    {
        return Err(invalid("Equipment placement, host and settings must share one object").into());
    }
    let template = template_for(&request.definition.template, request.definition.root)?
        .ok_or_else(|| invalid("Choose an equipment definition"))?;
    check_mount(&template, &request.attachment)?;
    request
        .attachment
        .resolve(session.document(), session.parameters())?;
    let imported = symbols::prepare_import_at(
        session,
        revision,
        actor.clone(),
        &request.definition,
        request.placement.definition,
    )?;
    if imported.definition != request.placement.definition {
        return Err(invalid("Equipment placement requires its imported definition").into());
    }
    let staged = Session::new(imported.prepared.document().clone())?;
    let placed = symbols::prepare_place(&staged, 0, actor.clone(), &request.placement)?;
    let mut entity = placed
        .prepared
        .document()
        .get(request.placement.instance)
        .expect("placed instance")
        .clone();
    if let EntityData::Instance {
        placement_source, ..
    } = &mut entity.data
    {
        *placement_source = Some(request.attachment_id);
    }
    let mut edits = envelope_edits(
        placed.prepared.document(),
        &request.settings,
        request.placement.instance,
        &request.envelopes,
    )?;
    edits.extend([
        Mutation::Insert(request.attachment.entity(request.attachment_id)?),
        Mutation::Replace(entity),
        Mutation::Insert(request.settings.entity(request.settings_id)?),
    ]);
    let doc = placed.prepared.document().edited(&edits)?;
    candidate(session, revision, actor, "Place equipment", &doc)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentEditRequest {
    pub record: EntityId,
    pub name: String,
    pub size_mm: [f64; 3],
    pub attachment: Attachment,
    pub settings: InstanceSettings,
    #[serde(default)]
    pub envelopes: Vec<EnvelopeSource>,
}

/// Replacing the explicit host is also the repair operation for a lost host.
/// It does not need the old host to resolve or preserve a stale world pose.
pub fn prepare_edit(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &EquipmentEditRequest,
) -> Result<Prepared, CommandError> {
    let symbol = symbols::read_instance(session.document(), request.record)?;
    let source = symbols::read_definition(session.document(), symbol.definition)?;
    let template = template_for(&source.template, source.root)?
        .ok_or_else(|| invalid("Choose an equipment instance"))?;
    if request.attachment.owner != symbol.instance
        || request.settings.symbol_instance != request.record
    {
        return Err(invalid("Equipment edits must retain their canonical object identity").into());
    }
    check_mount(&template, &request.attachment)?;
    request
        .attachment
        .resolve(session.document(), session.parameters())?;
    let mut values = symbol.overrides;
    values.extend(overrides(&template, request.size_mm)?);
    let edited = symbols::prepare_edit(
        session,
        revision,
        actor.clone(),
        &symbols::SymbolEditRequest {
            record: request.record,
            overrides: values,
        },
    )?;
    let mut instance = edited
        .prepared
        .document()
        .get(symbol.instance)
        .expect("symbol instance")
        .clone();
    let driver = match &instance.data {
        EntityData::Instance {
            placement_source: Some(id),
            ..
        } if read_attachment(session.document(), *id).is_ok() => *id,
        _ => return Err(invalid(
            "Equipment must retain its direct hosted placement; repair its placement source first",
        )
        .into()),
    };
    instance.name = request.name.clone();
    let settings_id = session.document().entities().find_map(|e| match &e.data {
        EntityData::Extension { definition, .. } if definition.type_id == INSTANCE_TYPE => {
            InstanceSettings::from_definition(definition)
                .ok()
                .filter(|s| s.symbol_instance == request.record)
                .map(|_| e.id)
        }
        _ => None,
    });
    let settings = request.settings.entity(settings_id.unwrap_or_default())?;
    let mut edits = envelope_edits(
        edited.prepared.document(),
        &request.settings,
        symbol.instance,
        &request.envelopes,
    )?;
    edits.extend([
        Mutation::Replace(instance),
        Mutation::Replace(request.attachment.entity(driver)?),
        if settings_id.is_some() {
            Mutation::Replace(settings)
        } else {
            Mutation::Insert(settings)
        },
    ]);
    let doc = edited.prepared.document().edited(&edits)?;
    candidate(session, revision, actor, "Edit equipment", &doc)
}
