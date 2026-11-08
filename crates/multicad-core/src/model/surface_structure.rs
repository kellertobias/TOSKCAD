//! Canonical native relief on retained planar faces of one Part.
use super::{
    BodyInput, CreatedBody, FaceInput, ModelPrepared, PROFILE_FACE_ROLE, angle,
    boolean::available,
    common::{error, number, parameter_links, prepare_edit, revision},
    length,
    source::{check_dimension, invalid},
    validate_profile_face,
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy,
    parameters::{EvaluatedParameters, Unit},
    references::GeometryIntent,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SURFACE_STRUCTURE_TYPE: &str = "model.surface_structure";
pub const SURFACE_STRUCTURE_CAPABILITY: &str = "model.surface_structure.1";
pub const MAX_SURFACE_STRUCTURE_FACES: usize = 4;
pub const MAX_SURFACE_STRUCTURE_MOTIFS: usize = 192;
pub const MAX_SURFACE_IMAGE_AXIS: u8 = 12;
pub const MAX_SURFACE_IMAGE_REPEAT: u8 = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SurfaceStructurePattern {
    Ribs,
    Waves,
    Stipple,
    /// Raw grayscale samples, row-major from the positive V side of the window.
    HeightMap {
        width: u8,
        height: u8,
        samples: Vec<u8>,
        repeat: [u8; 2],
    },
}

impl SurfaceStructurePattern {
    fn image_motifs(&self) -> Result<Option<usize>, DocumentError> {
        let Self::HeightMap {
            width,
            height,
            samples,
            repeat,
        } = self
        else {
            return Ok(None);
        };
        if !(1..=MAX_SURFACE_IMAGE_AXIS).contains(width)
            || !(1..=MAX_SURFACE_IMAGE_AXIS).contains(height)
            || samples.len() != usize::from(*width) * usize::from(*height)
        {
            return Err(invalid(format!(
                "Height Map needs a complete grayscale sample grid with 1–{MAX_SURFACE_IMAGE_AXIS} samples per axis"
            )));
        }
        if !repeat
            .iter()
            .all(|n| (1..=MAX_SURFACE_IMAGE_REPEAT).contains(n))
        {
            return Err(invalid(format!(
                "Height Map repeats must be between 1 and {MAX_SURFACE_IMAGE_REPEAT} per axis"
            )));
        }
        let occupied = samples.iter().filter(|&&sample| sample != 0).count();
        if occupied == 0 {
            return Err(invalid(
                "The Height Map is entirely black; add at least one nonzero sample",
            ));
        }
        Ok(Some(
            occupied * usize::from(repeat[0]) * usize::from(repeat[1]),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceStructureDirection {
    Raised,
    Recessed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStructureParameters {
    pub pattern: SurfaceStructurePattern,
    pub direction: SurfaceStructureDirection,
    pub amplitude: Dimension,
    pub pitch: Dimension,
    pub size: [Dimension; 2],
    pub offset: [Dimension; 2],
    pub rotation: Dimension,
    pub border: Dimension,
}

impl Default for SurfaceStructureParameters {
    fn default() -> Self {
        Self {
            pattern: SurfaceStructurePattern::Ribs,
            direction: SurfaceStructureDirection::Raised,
            amplitude: length(0.8),
            pitch: length(4.),
            size: [length(20.), length(16.)],
            offset: [length(0.), length(0.)],
            rotation: angle(0.),
            border: length(1.),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSurfaceStructure {
    pub pattern: SurfaceStructurePattern,
    pub direction: SurfaceStructureDirection,
    pub amplitude_mm: f64,
    pub pitch_mm: f64,
    pub size_mm: [f64; 2],
    pub offset_mm: [f64; 2],
    pub rotation_rad: f64,
    pub border_mm: f64,
}

impl SurfaceStructureParameters {
    fn lengths(&self) -> [&Dimension; 7] {
        [
            &self.amplitude,
            &self.pitch,
            &self.size[0],
            &self.size[1],
            &self.offset[0],
            &self.offset[1],
            &self.border,
        ]
    }

    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedSurfaceStructure, DocumentError> {
        let mm = |dimension: &Dimension| number(dimension, Unit::Millimeter, parameters);
        let amplitude_mm = mm(&self.amplitude)?;
        let pitch_mm = if matches!(&self.pattern, SurfaceStructurePattern::HeightMap { .. }) {
            0.
        } else {
            let pitch = mm(&self.pitch)?;
            if !(0.5..=200.).contains(&pitch) {
                return Err(invalid("Pattern pitch must be 0.5–200 mm"));
            }
            pitch
        };
        let size_mm = [mm(&self.size[0])?, mm(&self.size[1])?];
        let offset_mm = [mm(&self.offset[0])?, mm(&self.offset[1])?];
        let rotation_rad = number(&self.rotation, Unit::Radian, parameters)?;
        let border_mm = mm(&self.border)?;
        if !(0.05..=20.).contains(&amplitude_mm) {
            return Err(invalid("Relief height or depth must be 0.05–20 mm"));
        }
        if !size_mm.iter().all(|value| (1.0..=300.).contains(value))
            || !offset_mm.iter().all(|value| value.abs() <= 300.)
        {
            return Err(invalid(
                "Window dimensions must be 1–300 mm and offsets within ±300 mm",
            ));
        }
        if !(0.1..=20.).contains(&border_mm) {
            return Err(invalid("The clear border must be 0.1–20 mm"));
        }
        Ok(ResolvedSurfaceStructure {
            pattern: self.pattern.clone(),
            direction: self.direction,
            amplitude_mm,
            pitch_mm,
            size_mm,
            offset_mm,
            rotation_rad: rotation_rad.rem_euclid(std::f64::consts::TAU),
            border_mm,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStructureRequest {
    pub feature: EntityId,
    pub input: BodyInput,
    pub faces: Vec<GeometryIntent>,
    pub parameters: SurfaceStructureParameters,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    input: BodyInput,
    faces: Vec<EntityId>,
    parameters: SurfaceStructureParameters,
}

impl SurfaceStructureRequest {
    fn source_and_links(&self) -> Result<(Source, Vec<Link>), DocumentError> {
        if self.feature.is_nil()
            || self.input.body.is_nil()
            || self.input.producer.is_nil()
            || self.feature == self.input.body
            || self.feature == self.input.producer
            || self.input.body == self.input.producer
        {
            return Err(invalid(
                "Surface Structure needs a Part and its preceding producer",
            ));
        }
        if !(1..=MAX_SURFACE_STRUCTURE_FACES).contains(&self.faces.len()) {
            return Err(invalid(format!(
                "Select one to {MAX_SURFACE_STRUCTURE_FACES} distinct planar faces of one Part"
            )));
        }
        if self
            .parameters
            .pattern
            .image_motifs()?
            .is_some_and(|count| count * self.faces.len() > MAX_SURFACE_STRUCTURE_MOTIFS)
        {
            return Err(invalid(format!(
                "Height Map exceeds {MAX_SURFACE_STRUCTURE_MOTIFS} raised or recessed cells; reduce repeats, selected faces or sample density"
            )));
        }
        let lengths = self.parameters.lengths();
        for dimension in lengths {
            check_dimension(dimension, Unit::Millimeter)?;
        }
        check_dimension(&self.parameters.rotation, Unit::Radian)?;
        let mut ids = BTreeSet::new();
        let mut links = vec![Link::new(self.input.producer, "source")];
        for (index, intent) in self.faces.iter().enumerate() {
            validate_profile_face(&FaceInput {
                source: self.input.producer,
                intent: intent.clone(),
            })?;
            if !ids.insert(intent.id)
                || self.faces[..index].iter().any(|prior| {
                    prior.origins == intent.origins && prior.boundary == intent.boundary
                })
            {
                return Err(invalid("Remove the duplicate Surface Structure face"));
            }
            links.push(Link {
                target: self.input.producer,
                role: PROFILE_FACE_ROLE.into(),
                geometry: Some(intent.clone()),
            });
        }
        links.extend(parameter_links(
            lengths
                .into_iter()
                .chain([&self.parameters.rotation])
                .collect(),
        ));
        Ok((
            Source {
                input: self.input.clone(),
                faces: self.faces.iter().map(|intent| intent.id).collect(),
                parameters: self.parameters.clone(),
            },
            links,
        ))
    }
}

pub fn read_surface_structure(
    document: &Document,
    feature: EntityId,
) -> Result<SurfaceStructureRequest, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|entity| &entity.data)
    else {
        return Err(invalid("Choose an existing Surface Structure feature"));
    };
    if definition.type_id != SURFACE_STRUCTURE_TYPE || definition.schema_version != 1 {
        return Err(invalid("Unsupported Surface Structure provider or version"));
    }
    let source: Source = serde_json::from_value(definition.data.clone())
        .map_err(|failure| invalid(failure.to_string()))?;
    let faces = source
        .faces
        .iter()
        .map(|id| {
            inputs
                .iter()
                .filter_map(|link| link.geometry.as_ref())
                .find(|intent| intent.id == *id)
                .cloned()
                .ok_or_else(|| invalid("A Surface Structure face capture is missing"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let request = SurfaceStructureRequest {
        feature,
        input: source.input,
        faces,
        parameters: source.parameters,
    };
    let (_, expected) = request.source_and_links()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Surface Structure source and retained links disagree",
        ));
    }
    let Some(EntityData::Part { features, .. }) =
        document.get(request.input.body).map(|entity| &entity.data)
    else {
        return Err(invalid("Surface Structure belongs to a Part"));
    };
    let output = features
        .iter()
        .position(|id| *id == feature)
        .ok_or_else(|| invalid("Surface Structure is absent from its Part history"))?;
    if !features[..output].contains(&request.input.producer) {
        return Err(invalid("Surface Structure must follow its frozen producer"));
    }
    Ok(request)
}

pub fn prepare_surface_structure(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &SurfaceStructureRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let (source, inputs) = request.source_and_links()?;
    request.parameters.resolve(session.parameters())?;
    let definition = Definition {
        type_id: SURFACE_STRUCTURE_TYPE.into(),
        schema_version: 1,
        data: serde_json::to_value(source).map_err(|failure| error(failure.to_string()))?,
    };
    let output = CreatedBody {
        body: request.input.body,
        feature: request.feature,
    };
    if let Some(existing) = document.get(request.feature) {
        let retained = read_surface_structure(document, request.feature)?;
        if retained.input != request.input {
            return Err(error(
                "Edit Surface Structure on its original Part and producer",
            ));
        }
        if matches!(
            existing.data,
            EntityData::Feature {
                suppressed: true,
                ..
            }
        ) {
            return Err(error("Unsuppress Surface Structure before editing it"));
        }
        return prepare_edit(
            session,
            expected_revision,
            actor,
            "Edit Surface Structure",
            output,
            definition,
            inputs,
        );
    }
    if request.feature == document.id() {
        return Err(error("Surface Structure needs a fresh feature identity"));
    }
    available(document, &request.input, true)?;
    let mut entity = Entity::new(
        "Surface Structure",
        EntityData::Feature {
            definition,
            inputs,
            suppressed: false,
        },
    );
    entity.id = request.feature;
    let mut part = document
        .get(request.input.body)
        .expect("validated Part")
        .clone();
    let EntityData::Part { features, .. } = &mut part.data else {
        unreachable!("validated Part");
    };
    features.push(request.feature);
    let mut edits = vec![Mutation::Insert(entity), Mutation::Replace(part)];
    let index = hierarchy::Index::new(document).map_err(|failure| error(failure.to_string()))?;
    if let Some(parent) = index
        .parent(request.input.body)
        .map_err(|failure| error(failure.to_string()))?
    {
        let mut hierarchy = document.hierarchy().clone();
        hierarchy
            .members
            .entry(parent)
            .or_default()
            .push(request.feature);
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Apply Surface Structure".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![output],
    })
}
