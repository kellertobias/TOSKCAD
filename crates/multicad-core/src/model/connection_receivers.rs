//! Exact receiving cuts driven by a shared connection axis and frozen predecessor.
use super::{
    BodyInput, BodyOutput, CreatedBody, ModelPrepared, OccurrencePath, PrimitiveFrame,
    ResolvedThread, RigidPose, ThreadMode, ThreadParameters, ThreadRepresentation,
    TransferPoseProjection,
    common::*,
    source::{check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    parameters::{EvaluatedParameters, Unit},
    references::TopologySnapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const CONNECTION_RECEIVER_TYPE: &str = "model.connection_receiver";
pub const CONNECTION_RECEIVER_CAPABILITY: &str = "model.connection_receiver.1";
pub const CONNECTION_RECEIVER_ENTRY_CAPABILITY: &str = "model.connection_receiver.face_entry.1";
const POSE_ROLE: &str = "connection_receiver/pose/";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConnectionReceiverCut {
    Round { diameter: Dimension },
    Tapped { thread: Box<ThreadParameters> },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConnectionReceiverExtent {
    Through {},
    Blind { depth: Dimension, floor: Dimension },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConnectionReceiverSeat {
    Counterbore {
        diameter: Dimension,
        depth: Dimension,
    },
    Countersink {
        diameter: Dimension,
        depth: Dimension,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionReceiverDefinition {
    pub output: BodyOutput,
    pub input: BodyInput,
    pub occurrence: OccurrencePath,
    pub axis: EntityId,
    /// Entry station along the shared positive axis. No world position is copied.
    pub station: Dimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_reference: Option<super::FaceInput>,
    pub cut: ConnectionReceiverCut,
    pub extent: ConnectionReceiverExtent,
    pub minimum_wall: Dimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat: Option<ConnectionReceiverSeat>,
}
#[derive(Clone, Debug)]
pub struct ResolvedConnectionReceiver {
    pub station_mm: f64,
    pub diameter_mm: f64,
    pub depth_mm: Option<f64>,
    pub floor_mm: Option<f64>,
    pub minimum_wall_mm: f64,
    pub seat: Option<(bool, f64, f64)>,
    pub thread: Option<Box<ResolvedThread>>,
}
impl ConnectionReceiverDefinition {
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut fields = vec![&self.station, &self.minimum_wall];
        match &self.cut {
            ConnectionReceiverCut::Round { diameter } => fields.push(diameter),
            ConnectionReceiverCut::Tapped { thread } => fields.extend(thread.dimensions()),
        }
        if let ConnectionReceiverExtent::Blind { depth, floor } = &self.extent {
            fields.extend([depth, floor]);
        }
        if let Some(
            ConnectionReceiverSeat::Counterbore { diameter, depth }
            | ConnectionReceiverSeat::Countersink { diameter, depth },
        ) = &self.seat
        {
            fields.extend([diameter, depth]);
        }
        fields
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body != self.input.body
            || self.input.body.is_nil()
            || self.input.producer.is_nil()
            || self.input.body == self.input.producer
            || self.axis.is_nil()
            || self.axis == self.input.producer
            || self.occurrence.root.is_nil()
            || self.occurrence.members.len() > 32
            || self.occurrence.members.iter().any(EntityId::is_nil)
            || self
                .occurrence
                .members
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.occurrence.members.len()
        {
            return Err(invalid(
                "Receiving cut requires a frozen predecessor, same output Part, axis and bounded occurrence path",
            ));
        }
        if let Some(face) = &self.entry_reference {
            super::validate_profile_face(face)?;
            if face.source != self.input.producer || self.station != super::length(0.) {
                return Err(invalid(
                    "A captured receiving entry must belong to its frozen predecessor and requires zero manual station",
                ));
            }
        }
        for field in self.dimensions() {
            // Thread angles are validated by its own schema.
            if !matches!(&self.cut, ConnectionReceiverCut::Tapped { .. }) {
                check_dimension(field, Unit::Millimeter)?;
            }
        }
        check_dimension(&self.station, Unit::Millimeter)?;
        check_dimension(&self.minimum_wall, Unit::Millimeter)?;
        if let ConnectionReceiverExtent::Blind { depth, floor } = &self.extent {
            check_dimension(depth, Unit::Millimeter)?;
            check_dimension(floor, Unit::Millimeter)?;
        }
        if let Some(
            ConnectionReceiverSeat::Counterbore { diameter, depth }
            | ConnectionReceiverSeat::Countersink { diameter, depth },
        ) = &self.seat
        {
            check_dimension(diameter, Unit::Millimeter)?;
            check_dimension(depth, Unit::Millimeter)?;
        }
        if let ConnectionReceiverCut::Tapped { thread } = &self.cut {
            thread.validate_for_mode(ThreadMode::CutInternal)?;
            if thread.frame != PrimitiveFrame::default() || thread.end_treatments.is_some() {
                return Err(invalid(
                    "Connection thread placement is owned by its axis; local frames and endpoint treatments are unsupported",
                ));
            }
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: CONNECTION_RECEIVER_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != CONNECTION_RECEIVER_TYPE || d.schema_version != 1 {
            return Err(invalid("Unsupported connection receiver provider/version"));
        }
        let s: Self = serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = parameter_links(self.dimensions());
        if let Some(face) = &self.entry_reference {
            links.push(Link {
                target: face.source,
                role: "connection_receiver/entry".into(),
                geometry: Some(face.intent.clone()),
            });
        }
        if let ConnectionReceiverCut::Tapped { thread } = &self.cut {
            for link in thread.input_links() {
                if !links.contains(&link) {
                    links.push(link);
                }
            }
        }
        links.extend([
            Link::new(self.input.producer, "prior_solid"),
            Link::new(self.axis, "connection_axis"),
        ]);
        for (i, id) in std::iter::once(self.occurrence.root)
            .chain(self.occurrence.members.iter().copied())
            .enumerate()
        {
            links.push(Link::new(id, format!("{POSE_ROLE}{i}")));
        }
        Ok(links)
    }
    pub fn resolve(
        &self,
        p: &EvaluatedParameters,
    ) -> Result<ResolvedConnectionReceiver, DocumentError> {
        self.validate()?;
        let mm = |d: &Dimension| d.value_in(Unit::Millimeter, Some(p)).map_err(invalid);
        let positive = |d: &Dimension| -> Result<f64, DocumentError> {
            let x = mm(d)?;
            if !x.is_finite() || x <= 0. || x > 1e9 {
                Err(invalid(
                    "Receiving dimensions must be finite positive lengths",
                ))
            } else {
                Ok(x)
            }
        };
        let station = mm(&self.station)?;
        if !station.is_finite() || station.abs() > 1e9 {
            return Err(invalid("Receiving station is outside model bounds"));
        }
        let (diameter, thread) = match &self.cut {
            ConnectionReceiverCut::Round { diameter } => (positive(diameter)?, None),
            ConnectionReceiverCut::Tapped { thread } => {
                let t = thread.resolve(p, ThreadMode::CutInternal)?;
                if t.representation != ThreadRepresentation::Modeled {
                    return Err(invalid("Receiving tapped cuts require modeled threads"));
                }
                (2. * t.major_radius_mm, Some(Box::new(t)))
            }
        };
        let (depth, floor) = match &self.extent {
            ConnectionReceiverExtent::Through {} => (None, None),
            ConnectionReceiverExtent::Blind { depth, floor } => {
                (Some(positive(depth)?), Some(positive(floor)?))
            }
        };
        let seat = self
            .seat
            .as_ref()
            .map(|s| {
                let (cone, d, h) = match s {
                    ConnectionReceiverSeat::Counterbore { diameter, depth } => {
                        (false, diameter, depth)
                    }
                    ConnectionReceiverSeat::Countersink { diameter, depth } => {
                        (true, diameter, depth)
                    }
                };
                let d = positive(d)?;
                let h = positive(h)?;
                if d <= diameter || depth.is_some_and(|depth| h >= depth) {
                    return Err(invalid(
                        "Receiving seat must be wider than its bore and shallower than a blind cut",
                    ));
                }
                Ok((cone, d, h))
            })
            .transpose()?;
        Ok(ResolvedConnectionReceiver {
            station_mm: station,
            diameter_mm: diameter,
            depth_mm: depth,
            floor_mm: floor,
            minimum_wall_mm: positive(&self.minimum_wall)?,
            seat,
            thread,
        })
    }
    /// Resolve captured entry from current native planar geometry. Scalar-only
    /// resolve remains available for structural preparation, never publication.
    pub fn resolve_current(
        &self,
        document: &Document,
        p: &EvaluatedParameters,
        revision: u64,
        snapshots: &BTreeMap<EntityId, TopologySnapshot>,
    ) -> Result<ResolvedConnectionReceiver, DocumentError> {
        let mut resolved = self.resolve(p)?;
        if let Some(face) = &self.entry_reference {
            let (axis_source, axis_face) = super::read_screw_connection_axis(document, self.axis)?;
            let axis = axis_source.resolve(document, p, &axis_face, revision, snapshots)?;
            let source = super::ScrewConnectionAxisDefinition::new(
                self.input.clone(),
                self.occurrence.clone(),
                face.intent.id,
            );
            let plane = source.resolve(document, p, face, revision, snapshots)?;
            let normal = plane.vector([0., 0., 1.]);
            let direction = axis.vector([0., 0., 1.]);
            let denominator = normal
                .iter()
                .zip(direction)
                .map(|(a, b)| a * b)
                .sum::<f64>();
            if denominator.abs() < 1. - 1e-9 {
                return Err(invalid(
                    "The receiving entry face must be perpendicular to the fastening axis",
                ));
            }
            let station = (0..3)
                .map(|i| (plane.translation_mm[i] - axis.translation_mm[i]) * normal[i])
                .sum::<f64>()
                / denominator;
            if !station.is_finite() || station.abs() > 1e9 {
                return Err(invalid(
                    "The captured receiving entry is outside model bounds",
                ));
            }
            resolved.station_mm = station;
        }
        Ok(resolved)
    }
    pub fn local_pose(
        &self,
        document: &Document,
        p: &EvaluatedParameters,
        revision: u64,
        snapshots: &BTreeMap<EntityId, TopologySnapshot>,
    ) -> Result<RigidPose, DocumentError> {
        let (source, face) = super::read_screw_connection_axis(document, self.axis)?;
        let world = source.resolve(document, p, &face, revision, snapshots)?;
        let receiver = super::resolve_occurrence_pose(
            document,
            &self.occurrence,
            self.input.body,
            p,
            revision,
            snapshots,
        )?;
        RigidPose {
            translation_mm: [
                0.,
                0.,
                self.resolve_current(document, p, revision, snapshots)?
                    .station_mm,
            ],
            ..Default::default()
        }
        .then(world)?
        .then(receiver.inverse()?)
    }
}
pub fn read_connection_receiver(
    document: &Document,
    id: EntityId,
) -> Result<ConnectionReceiverDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose a connection receiving feature"));
    };
    let s = ConnectionReceiverDefinition::from_definition(definition)?;
    let expected = s.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid("Receiving cut source and canonical links disagree"));
    }
    s.input.resolve_producer(document)?;
    if let Some(part) = document.get(s.output.body) {
        let EntityData::Part { features, .. } = &part.data else {
            return Err(invalid("Receiving output must be a Part"));
        };
        if !features.contains(&id) {
            return Err(invalid("Receiving output does not contain its feature"));
        }
    }
    if document.get(s.axis).is_some() {
        super::read_screw_connection_axis(document, s.axis)?;
    }
    for link in inputs.iter().filter(|l| l.role.starts_with("parameter/")) {
        if document
            .get(link.target)
            .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
        {
            return Err(invalid("Receiving dimensional input is not a Parameter"));
        }
    }
    Ok(s)
}
pub fn connection_receiver_pose_projection(
    document: &Document,
    id: EntityId,
) -> Result<Option<TransferPoseProjection>, DocumentError> {
    if !matches!(document.get(id).map(|e|&e.data),Some(EntityData::Feature{definition,..})if definition.type_id==CONNECTION_RECEIVER_TYPE)
    {
        return Ok(None);
    }
    let s = read_connection_receiver(document, id)?;
    super::occurrence_pose_projection(document, &s.occurrence, None).map(Some)
}
pub(crate) fn connection_receiver_evaluation_dependencies(
    document: &Document,
    id: EntityId,
) -> Result<Option<BTreeSet<EntityId>>, DocumentError> {
    let Some(projection) = connection_receiver_pose_projection(document, id)? else {
        return Ok(None);
    };
    let mut deps = document
        .get(id)
        .expect("validated receiver")
        .references()
        .into_iter()
        .filter(|l| !l.role.starts_with(POSE_ROLE))
        .map(|l| l.target)
        .collect::<BTreeSet<_>>();
    deps.extend(projection.geometry_dependencies);
    Ok(Some(deps))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionReceiverRequest {
    pub feature: EntityId,
    pub name: String,
    pub source: ConnectionReceiverDefinition,
}
pub fn prepare_connection_receiver(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ConnectionReceiverRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.source.resolve(session.parameters())?;
    super::boolean::available(session.document(), &request.source.input, true)?;
    super::validate_transfer_path(
        session.document(),
        &request.source.occurrence,
        request.source.input.body,
    )?;
    super::read_screw_connection_axis(session.document(), request.source.axis)?;
    if request.feature.is_nil()
        || request.feature == session.document().id()
        || session.document().get(request.feature).is_some()
        || request.name.trim().is_empty()
        || request.name.len() > 1000
    {
        return Err(error(
            "Receiving feature requires a fresh identity and valid name",
        ));
    }
    let mut e = Entity::new(
        &request.name,
        EntityData::Feature {
            definition: request.source.definition()?,
            inputs: request.source.inputs()?,
            suppressed: false,
        },
    );
    e.id = request.feature;
    let mut part = session
        .document()
        .get(request.source.input.body)
        .expect("validated Part")
        .clone();
    let EntityData::Part { features, .. } = &mut part.data else {
        unreachable!()
    };
    features.push(request.feature);
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Create connection receiving cut".into(),
        edits: vec![Mutation::Insert(e), Mutation::Replace(part)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            body: request.source.output.body,
            feature: request.feature,
        }],
    })
}
pub fn prepare_connection_receiver_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    source: &ConnectionReceiverDefinition,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let old = read_connection_receiver(session.document(), feature)?;
    if old.input != source.input || old.output != source.output {
        return Err(error(
            "Receiving edits retain the original predecessor and output Part",
        ));
    }
    source.resolve(session.parameters())?;
    super::read_screw_connection_axis(session.document(), source.axis)?;
    super::validate_transfer_path(session.document(), &source.occurrence, source.input.body)?;
    prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit connection receiving cut",
        CreatedBody {
            body: source.output.body,
            feature,
        },
        source.definition()?,
        source.inputs()?,
    )
}
