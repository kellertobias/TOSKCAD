use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    references::{GeometryIntent, TopologyKind},
    sketch_regions::RegionIntent,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const EXTRUDE_TYPE: &str = "model.extrude";
pub const EXTRUDE_SCHEMA: u32 = 1;
pub const CAPABILITY: &str = "model.extrude.1";
pub const PART_REFERENCE_CAPABILITY: &str = "model.part_reference.1";
pub const MAX_EXTRUDE_OUTPUTS: usize = 128;

pub(super) fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
pub fn length(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}
pub fn angle(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "deg".into(),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    NewBody,
    Join,
    Cut,
    Intersect,
    /// Generic extrusion/Boolean Split is reserved. Bounded two-membership
    /// partitions use the separate model.split contract and semantic outputs.
    Split,
}

/// Body identity is output/target metadata. Only `producer` is a dependency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyInput {
    pub body: EntityId,
    pub producer: EntityId,
}
impl BodyInput {
    /// Resolve the explicit source relationship without consulting a final Part
    /// as a geometry dependency. Native evaluation still validates its solid.
    pub fn resolve_producer(&self, document: &Document) -> Result<EntityId, DocumentError> {
        let Some(EntityData::Part { features, .. }) = document.get(self.body).map(|e| &e.data)
        else {
            return Err(invalid("Model target must be an existing Part"));
        };
        if !features.contains(&self.producer)
            || !matches!(
                document.get(self.producer).map(|e| &e.data),
                Some(EntityData::Feature { .. })
            )
        {
            return Err(invalid(
                "Model target requires an existing producer from its body history",
            ));
        }
        Ok(self.producer)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyOutput {
    pub body: EntityId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSource {
    pub sketch: EntityId,
    pub region: RegionIntent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceInput {
    pub source: EntityId,
    pub intent: GeometryIntent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Direction {
    /// Inclination from the sketch plane: 90 degrees follows positive N.
    pub elevation: Dimension,
    /// Bearing within the sketch plane, measured from U towards V.
    pub azimuth: Dimension,
    pub reverse: bool,
}
impl Default for Direction {
    fn default() -> Self {
        Self {
            elevation: angle(90.),
            azimuth: angle(0.),
            reverse: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Termination {
    Distance {
        distance: Dimension,
    },
    ToPlane {
        plane: EntityId,
        /// Signed movement of the end plane along its own normal, in the
        /// supplied length unit; independent of the start-plane offset.
        offset: Dimension,
    },
    /// The sole canonical intent/source live on Feature.inputs. Repairing that
    /// link therefore cannot leave a duplicated payload selector stale.
    ToFace {
        reference: EntityId,
        /// Signed movement along the current native outward normal of the
        /// selected face. An attachment anchor changes origin/roll, not sign.
        offset: Dimension,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Extent {
    OneSided {
        end: Termination,
    },
    TwoSided {
        forward: Termination,
        backward: Termination,
    },
    /// The total distance, divided equally along both sweep directions.
    Symmetric {
        total: Dimension,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeParameters {
    pub direction: Direction,
    /// Offset along sketch N, independently of sweep direction or extent.
    pub start_offset: Dimension,
    pub taper: Dimension,
    pub extent: Extent,
}
impl Default for ExtrudeParameters {
    fn default() -> Self {
        Self {
            direction: Default::default(),
            start_offset: length(0.),
            taper: angle(0.),
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: length(10.),
                },
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeDefinition {
    pub profile: ProfileSource,
    pub output: BodyOutput,
    pub parameters: ExtrudeParameters,
    pub operation: Operation,
    pub targets: Vec<BodyInput>,
    pub keep_tools: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedExtrude {
    /// Unit sweep direction in local U,V,N coordinates. Distances below are
    /// measured along this vector, including when it is tilted.
    pub direction: [f64; 3],
    /// Shift frame.origin by frame.N*this value BEFORE applying sweep extents.
    pub start_offset_mm: f64,
    pub forward_mm: f64,
    pub backward_mm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedTermination {
    Distance { distance_mm: f64 },
    ToPlane { plane: EntityId, offset_mm: f64 },
    ToFace { reference: EntityId, offset_mm: f64 },
}
impl ResolvedTermination {
    fn distance(&self) -> Result<f64, DocumentError> {
        match self {
            Self::Distance { distance_mm } => Ok(*distance_mm),
            _ => Err(invalid(
                "Associative termination requires current native geometry; it has no resolved distance",
            )),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedExtent {
    OneSided {
        end: ResolvedTermination,
    },
    TwoSided {
        forward: ResolvedTermination,
        backward: ResolvedTermination,
    },
    Symmetric {
        total_mm: f64,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedExtrudeControls {
    pub direction: [f64; 3],
    /// Offset along the sketch normal, independently of the sweep direction.
    pub start_offset_mm: f64,
    /// Signed draft angle, strictly between -PI/2 and PI/2. Positive values
    /// expand the profile toward each distance end; native geometry validates
    /// supported profiles and rejects collapsing or intersecting boundaries.
    pub taper_rad: f64,
    pub extent: ResolvedExtent,
}
pub(crate) fn check_dimension(value: &Dimension, unit: Unit) -> Result<(), DocumentError> {
    if Unit::parse(value.unit()).is_none_or(|u| u.dimension() != unit.dimension()) {
        return Err(invalid(format!(
            "Expected {}-compatible model units",
            unit.symbol()
        )));
    }
    match value {
        Dimension::Literal { value, .. } if !value.is_finite() || value.abs() > 1e9 => {
            return Err(invalid("Model dimension is nonfinite or exceeds limits"));
        }
        Dimension::Expression { expression } => {
            expression.validate().map_err(|e| invalid(e.to_string()))?
        }
        _ => {}
    }
    Ok(())
}
impl Termination {
    fn dimension(&self) -> &Dimension {
        match self {
            Self::Distance { distance } => distance,
            Self::ToPlane { offset, .. } | Self::ToFace { offset, .. } => offset,
        }
    }
    fn validate(&self) -> Result<(), DocumentError> {
        check_dimension(self.dimension(), Unit::Millimeter)?;
        match self {
            Self::ToPlane { plane, .. } if plane.is_nil() => {
                Err(invalid("Termination plane identity is nil"))
            }
            Self::ToFace { reference, .. } if reference.is_nil() => {
                Err(invalid("Termination face reference identity is nil"))
            }
            _ => Ok(()),
        }
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedTermination, DocumentError> {
        self.validate()?;
        let value = self
            .dimension()
            .value_in(Unit::Millimeter, Some(parameters))
            .map_err(invalid)?;
        Ok(match self {
            Self::Distance { .. } => {
                if value <= 0. {
                    return Err(invalid("Extrusion distance must be positive"));
                }
                ResolvedTermination::Distance { distance_mm: value }
            }
            Self::ToPlane { plane, .. } => ResolvedTermination::ToPlane {
                plane: *plane,
                offset_mm: value,
            },
            Self::ToFace { reference, .. } => ResolvedTermination::ToFace {
                reference: *reference,
                offset_mm: value,
            },
        })
    }
}
impl ExtrudeParameters {
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut result = vec![
            &self.direction.elevation,
            &self.direction.azimuth,
            &self.start_offset,
            &self.taper,
        ];
        match &self.extent {
            Extent::OneSided { end } => result.push(end.dimension()),
            Extent::TwoSided { forward, backward } => {
                result.push(forward.dimension());
                result.push(backward.dimension());
            }
            Extent::Symmetric { total } => result.push(total),
        }
        result
    }
    pub(super) fn terminations(&self) -> Vec<&Termination> {
        match &self.extent {
            Extent::OneSided { end } => vec![end],
            Extent::TwoSided { forward, backward } => vec![forward, backward],
            Extent::Symmetric { .. } => vec![],
        }
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        for value in [
            &self.direction.elevation,
            &self.direction.azimuth,
            &self.taper,
        ] {
            check_dimension(value, Unit::Radian)?;
        }
        check_dimension(&self.start_offset, Unit::Millimeter)?;
        for end in self.terminations() {
            end.validate()?;
        }
        if let Extent::Symmetric { total } = &self.extent {
            check_dimension(total, Unit::Millimeter)?;
        }
        Ok(())
    }
    /// Resolve typed control values without guessing a distance to a plane or
    /// face. Exact termination remains the native evaluator's responsibility.
    pub fn resolve_controls(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedExtrudeControls, DocumentError> {
        self.validate()?;
        let number = |d: &Dimension, u| d.value_in(u, Some(parameters)).map_err(invalid);
        let elevation = number(&self.direction.elevation, Unit::Radian)?;
        let azimuth = number(&self.direction.azimuth, Unit::Radian)?;
        let (elevation_sin, elevation_cos) = super::common::cardinal_sin_cos(elevation);
        let (azimuth_sin, azimuth_cos) = super::common::cardinal_sin_cos(azimuth);
        if elevation <= 0. || elevation >= std::f64::consts::PI || elevation_sin.abs() < 1e-8 {
            return Err(invalid(
                "Extrusion elevation must be between 0 and 180 degrees and cross the sketch plane",
            ));
        }
        let taper_rad = number(&self.taper, Unit::Radian)?;
        if !taper_rad.is_finite() || taper_rad.abs() >= std::f64::consts::FRAC_PI_2 {
            return Err(invalid(
                "Extrusion taper must be strictly between -90 and 90 degrees",
            ));
        }
        let sign = if self.direction.reverse { -1. } else { 1. };
        let direction = [
            elevation_cos * azimuth_cos * sign,
            elevation_cos * azimuth_sin * sign,
            elevation_sin * sign,
        ];
        let extent = match &self.extent {
            Extent::OneSided { end } => ResolvedExtent::OneSided {
                end: end.resolve(parameters)?,
            },
            Extent::TwoSided { forward, backward } => {
                let forward = forward.resolve(parameters)?;
                let backward = backward.resolve(parameters)?;
                if let (
                    ResolvedTermination::Distance { distance_mm: a },
                    ResolvedTermination::Distance { distance_mm: b },
                ) = (&forward, &backward)
                    && a + b > 1e9
                {
                    return Err(invalid("Extrusion span exceeds the model distance limit"));
                }
                ResolvedExtent::TwoSided { forward, backward }
            }
            Extent::Symmetric { total } => {
                let total = number(total, Unit::Millimeter)?;
                if total <= 0. {
                    return Err(invalid(
                        "Symmetric extrusion total distance must be positive",
                    ));
                }
                ResolvedExtent::Symmetric { total_mm: total }
            }
        };
        Ok(ResolvedExtrudeControls {
            direction,
            start_offset_mm: number(&self.start_offset, Unit::Millimeter)?,
            taper_rad,
            extent,
        })
    }
    /// Compatibility helper for callers requiring a numeric distance sweep.
    /// An associative target never becomes a fabricated numeric fallback.
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedExtrude, DocumentError> {
        let controls = self.resolve_controls(parameters)?;
        if controls.taper_rad != 0. {
            return Err(invalid(
                "Numeric distance sweep cannot represent extrusion taper; use resolved controls",
            ));
        }
        let (forward_mm, backward_mm) = match &controls.extent {
            ResolvedExtent::OneSided { end } => (end.distance()?, 0.),
            ResolvedExtent::TwoSided { forward, backward } => {
                (forward.distance()?, backward.distance()?)
            }
            ResolvedExtent::Symmetric { total_mm } => (total_mm / 2., total_mm / 2.),
        };
        Ok(ResolvedExtrude {
            direction: controls.direction,
            start_offset_mm: controls.start_offset_mm,
            forward_mm,
            backward_mm,
        })
    }
}
pub(super) fn validate_region(region: &RegionIntent) -> Result<(), DocumentError> {
    let mut curves = BTreeSet::new();
    for boundary in std::iter::once(&region.outer).chain(&region.holes) {
        if boundary.curves.is_empty() {
            return Err(invalid("Profile boundaries cannot be empty"));
        }
        for id in &boundary.curves {
            if id.is_nil() || !curves.insert(*id) {
                return Err(invalid(
                    "Profile boundaries require distinct non-nil curve identities",
                ));
            }
        }
    }
    if curves.len() > 512 {
        return Err(invalid("Profile curve budget exceeded"));
    }
    Ok(())
}
impl ExtrudeDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.profile.sketch.is_nil() || self.output.body.is_nil() {
            return Err(invalid("Extrusion source/output identity is nil"));
        }
        validate_region(&self.profile.region)?;
        self.parameters.validate()?;
        if self.targets.len() > 128 {
            return Err(invalid("Model target budget exceeded"));
        }
        let mut bodies = BTreeSet::new();
        for target in &self.targets {
            if target.body.is_nil()
                || target.producer.is_nil()
                || target.body == target.producer
                || !bodies.insert(target.body)
            {
                return Err(invalid(
                    "Body inputs require distinct bodies and explicit producer identities",
                ));
            }
        }
        if self.operation == Operation::NewBody && (!self.targets.is_empty() || self.keep_tools) {
            return Err(invalid(
                "New-body extrusion cannot contain Boolean targets or keep-tools intent",
            ));
        }
        if self.operation != Operation::NewBody && self.targets.is_empty() {
            return Err(invalid(
                "Boolean extrusion requires explicit body/producer inputs",
            ));
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: EXTRUDE_TYPE.into(),
            schema_version: EXTRUDE_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != EXTRUDE_TYPE || definition.schema_version != EXTRUDE_SCHEMA {
            return Err(invalid("Unsupported extrusion provider/version"));
        }
        let source: Self = serde_json::from_value(definition.data.clone())
            .map_err(|e| invalid(format!("Invalid extrusion source: {e}")))?;
        source.validate()?;
        Ok(source)
    }
    pub(super) fn face_references(&self) -> BTreeSet<EntityId> {
        self.parameters
            .terminations()
            .into_iter()
            .filter_map(|end| match end {
                Termination::ToFace { reference, .. } => Some(*reference),
                _ => None,
            })
            .collect()
    }
    fn plain_links(&self) -> Vec<Link> {
        let mut links = vec![Link::new(self.profile.sketch, "profile")];
        let parameters: BTreeSet<_> = self
            .parameters
            .dimensions()
            .into_iter()
            .filter_map(Dimension::expression)
            .flat_map(|e| e.bindings.iter().map(|b| b.target))
            .collect();
        links.extend(
            parameters
                .into_iter()
                .map(|id| Link::new(id, format!("parameter/{id}"))),
        );
        let planes: BTreeSet<_> = self
            .parameters
            .terminations()
            .into_iter()
            .filter_map(|end| match end {
                Termination::ToPlane { plane, .. } => Some(*plane),
                _ => None,
            })
            .collect();
        links.extend(
            planes
                .into_iter()
                .map(|id| Link::new(id, format!("termination/plane/{id}"))),
        );
        links.extend(
            self.targets
                .iter()
                .map(|target| Link::new(target.producer, format!("body/{}/producer", target.body))),
        );
        links
    }
    pub fn inputs(&self, faces: &[FaceInput]) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let wanted = self.face_references();
        let mut seen = BTreeSet::new();
        let mut links = self.plain_links();
        for face in faces {
            face.intent.validate().map_err(invalid)?;
            if face.source.is_nil()
                || face.intent.kind != TopologyKind::Face
                || !wanted.contains(&face.intent.id)
                || !seen.insert(face.intent.id)
            {
                return Err(invalid(
                    "Face inputs must uniquely match termination reference identities",
                ));
            }
            links.push(Link {
                target: face.source,
                role: format!("termination/face/{}", face.intent.id),
                geometry: Some(face.intent.clone()),
            });
        }
        if seen != wanted {
            return Err(invalid("A termination face input is missing"));
        }
        Ok(links)
    }
}
fn validate_feature(
    document: &Document,
    id: EntityId,
    source: &ExtrudeDefinition,
    inputs: &[Link],
) -> Result<(), DocumentError> {
    let faces: Vec<_> = inputs
        .iter()
        .filter_map(|link| {
            link.geometry.as_ref().map(|intent| FaceInput {
                source: link.target,
                intent: intent.clone(),
            })
        })
        .collect();
    let expected = source.inputs(&faces)?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Extrusion source and canonical input links disagree",
        ));
    }
    for link in inputs {
        if let Some(entity) = document.get(link.target) {
            let correct = if link.role == "profile" {
                matches!(entity.data, EntityData::Sketch { .. })
            } else if link.role.starts_with("parameter/") {
                matches!(entity.data, EntityData::Parameter { .. })
            } else if link.role.starts_with("termination/plane/") {
                matches!(entity.data, EntityData::Plane { .. })
            } else {
                matches!(entity.data, EntityData::Feature { .. })
            };
            if !correct {
                return Err(invalid("Extrusion input has the wrong entity kind"));
            }
        }
    }
    if let Some(body) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &body.data else {
            return Err(invalid("Extrusion output must identify a Part"));
        };
        if !features.contains(&id) {
            return Err(invalid(
                "Extrusion output body must contain its producing feature",
            ));
        }
    }
    for target in &source.targets {
        if let Some(body) = document.get(target.body) {
            let EntityData::Part { features, .. } = &body.data else {
                return Err(invalid("Model target metadata must identify a Part"));
            };
            if !features.contains(&target.producer) {
                return Err(invalid(
                    "Model target producer is not in its body's history",
                ));
            }
        }
    }
    Ok(())
}
pub fn read_extrude(
    document: &Document,
    feature: EntityId,
) -> Result<ExtrudeDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose an extrusion feature"));
    };
    let source = ExtrudeDefinition::from_definition(definition)?;
    validate_feature(document, feature, &source, inputs)?;
    Ok(source)
}
pub(crate) fn validate(document: &Document) -> Result<(), DocumentError> {
    super::extrude_group::validate_document(document)?;
    super::placement::validate_document(document)?;
    super::assembly::validate_document(document)?;
    super::transfer::validate_document(document)?;
    super::wall::validate_document(document)?;
    super::level::validate_document(document)?;
    super::cabinetry::validate_document(document)?;
    for entity in document.entities() {
        if let EntityData::Feature { definition, .. } = &entity.data {
            match definition.type_id.as_str() {
                EXTRUDE_TYPE => {
                    read_extrude(document, entity.id)?;
                }
                super::PRIMITIVE_TYPE => {
                    super::read_primitive(document, entity.id)?;
                }
                super::BOOLEAN_TYPE => {
                    super::read_boolean(document, entity.id)?;
                }
                super::SPLIT_TYPE => {
                    super::read_split(document, entity.id)?;
                }
                super::ALIGN_TYPE => {
                    super::read_alignment(document, entity.id)?;
                }
                super::TRANSFORM_TYPE => {
                    super::read_transform(document, entity.id)?;
                }
                super::SCALE_TYPE => {
                    super::read_scale(document, entity.id)?;
                }
                super::EDGE_TREATMENT_TYPE => {
                    super::read_edge_treatment(document, entity.id)?;
                }
                super::ADVANCED_TYPE => {
                    super::read_advanced(document, entity.id)?;
                }
                super::TRANSFER_TYPE => {
                    super::read_transfer_feature(document, entity.id)?;
                }
                super::REVOLVE_TYPE => {
                    super::read_revolve(document, entity.id)?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}
pub fn required_capabilities(document: &Document) -> BTreeSet<String> {
    let mut capabilities: BTreeSet<String> = document
        .entities()
        .filter_map(|entity| match &entity.data {
            EntityData::Feature { definition, .. }
                if [
                    EXTRUDE_TYPE,
                    super::PRIMITIVE_TYPE,
                    super::REVOLVE_TYPE,
                    super::BOOLEAN_TYPE,
                    super::SPLIT_TYPE,
                    super::TRANSFORM_TYPE,
                    super::SCALE_TYPE,
                    super::EDGE_TREATMENT_TYPE,
                    super::ADVANCED_TYPE,
                    super::TRANSFER_TYPE,
                    super::ALIGN_TYPE,
                    super::WALL_TYPE,
                    super::SLAB_TYPE,
                    super::STAIR_TYPE,
                    super::PANEL_TYPE,
                    super::PANEL_JOINT_CUT_TYPE,
                    super::MACHINING_CUT_TYPE,
                ]
                .contains(&definition.type_id.as_str()) =>
            {
                Some(format!(
                    "{}.{}",
                    definition.type_id, definition.schema_version
                ))
            }
            EntityData::Extension { definition, .. }
                if [
                    super::EXTRUDE_GROUP_TYPE,
                    super::INSTANCE_PLACEMENT_TYPE,
                    super::INSTANCE_ALIGNMENT_TYPE,
                    super::JOINT_TYPE,
                    super::ASSEMBLY_TYPE,
                    super::NAMED_POSE_TYPE,
                    super::TRANSFER_GROUP_TYPE,
                    super::WALL_TYPE_KIND,
                    super::OPENING_TYPE_KIND,
                    super::LEVEL_KIND,
                    super::ENVELOPE_KIND,
                    super::CARCASS_KIND,
                    super::STACK_KIND,
                    super::PANEL_JOINT_KIND,
                    super::MACHINING_KIND,
                    super::HARDWARE_KIND,
                ]
                .contains(&definition.type_id.as_str()) =>
            {
                Some(format!(
                    "{}.{}",
                    definition.type_id, definition.schema_version
                ))
            }
            EntityData::Instance { component, .. }
                if matches!(
                    document.get(*component).map(|e| &e.data),
                    Some(EntityData::Assembly { .. })
                ) =>
            {
                Some(super::ASSEMBLY_CAPABILITY.into())
            }
            EntityData::Instance {
                placement_source: Some(_),
                ..
            } => Some(super::INSTANCE_PLACEMENT_CAPABILITY.into()),
            _ => None,
        })
        .collect();
    if document.entities().any(|entity| {
        entity.references().iter().any(|link| {
            link.geometry.is_some()
                && document
                    .get(link.target)
                    .is_some_and(|target| matches!(target.data, EntityData::Part { .. }))
        })
    }) {
        capabilities.insert(PART_REFERENCE_CAPABILITY.into());
    }
    capabilities
}
