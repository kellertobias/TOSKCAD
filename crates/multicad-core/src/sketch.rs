//! Source-only datum sketches. Coordinates are millimeters, never display pixels.
use crate::{
    commands::{Actor, CommandError, ErrorCode, Prepared, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    references::{
        Attachment, GeometryIntent, Origin, ReferenceFrame, ReferenceReport, Resolution,
        TopologyKey, TopologyKind, TopologySnapshot,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const TYPE: &str = "core.datum_sketch";
/// A parent-local normal-only Plane offset. Origin is [0, 0, offset], normal is
/// local +Z, and the sketch's U descriptor is a local XY unit vector.
pub const OFFSET_PLANE_ROLE: &str = "offset_plane";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Datum {
    Xy,
    Xz,
    Yz,
}
impl Datum {
    pub const ALL: [Self; 3] = [Self::Xy, Self::Xz, Self::Yz];
    pub fn label(self) -> &'static str {
        match self {
            Self::Xy => "XY",
            Self::Xz => "XZ",
            Self::Yz => "YZ",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|d| d.label().eq_ignore_ascii_case(s))
    }
    pub fn frame(self) -> Frame {
        match self {
            Self::Xy => Frame {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                n: [0., 0., 1.],
            },
            Self::Xz => Frame {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 0., 1.],
                n: [0., -1., 0.],
            },
            Self::Yz => Frame {
                origin: [0.; 3],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
                n: [1., 0., 0.],
            },
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub n: [f64; 3],
}
impl Frame {
    pub fn point(self, p: [f64; 2]) -> [f64; 3] {
        std::array::from_fn(|i| self.origin[i] + self.u[i] * p[0] + self.v[i] * p[1])
    }
    pub fn vector(self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| self.u[i] * p[0] + self.v[i] * p[1] + self.n[i] * p[2])
    }
    pub fn coordinates(self, p: [f64; 3]) -> [f64; 2] {
        let d = std::array::from_fn(|i| p[i] - self.origin[i]);
        [dot(d, self.u), dot(d, self.v)]
    }
}
pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn validate_frame(frame: Frame) -> Result<(), DocumentError> {
    let reference = ReferenceFrame {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
    };
    reference.validate().map_err(invalid)?;
    if reference
        .normal()
        .into_iter()
        .zip(frame.n)
        .any(|(a, b)| !b.is_finite() || (a - b).abs() > 1e-9)
    {
        return Err(invalid(
            "Plane frame must have a right-handed orthonormal basis",
        ));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlaneOptions {
    pub normal_offset_mm: f64,
    pub angle_rad: f64,
}
impl PlaneOptions {
    /// Display preview and transactional creation share this validated transform.
    pub fn apply(self, frame: Frame) -> Result<Frame, DocumentError> {
        self.validate()?;
        validate_frame(frame)?;
        let (s, c) = self.angle_rad.sin_cos();
        let frame = Frame {
            origin: std::array::from_fn(|i| frame.origin[i] + frame.n[i] * self.normal_offset_mm),
            u: std::array::from_fn(|i| frame.u[i] * c + frame.v[i] * s),
            v: std::array::from_fn(|i| frame.v[i] * c - frame.u[i] * s),
            n: frame.n,
        };
        validate_frame(frame)?;
        Ok(frame)
    }
    fn validate(self) -> Result<(), DocumentError> {
        if !self.normal_offset_mm.is_finite()
            || self.normal_offset_mm.abs() > 1e9
            || !self.angle_rad.is_finite()
            || self.angle_rad.abs() > 1e6
        {
            return Err(invalid(
                "Plane offset/rotation is nonfinite or exceeds source limits",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub enum CreationPlane<'a> {
    Datum(Datum),
    Existing {
        id: EntityId,
        report: Option<&'a ReferenceReport>,
    },
    Face {
        snapshot: &'a TopologySnapshot,
        key: TopologyKey,
        anchor: Option<&'a Origin>,
    },
}
pub struct NewSketch {
    pub prepared: Prepared,
    pub sketch: EntityId,
    pub plane: EntityId,
    /// Current producer-local frame; occurrence placement stays in UI context.
    pub frame: Frame,
}
fn command_error(code: ErrorCode, message: impl Into<String>) -> CommandError {
    CommandError {
        code,
        message: message.into(),
        involved: vec![],
        parameter: None,
    }
}
fn plane_basis(normal: [f64; 3]) -> Result<[f64; 3], DocumentError> {
    if normal.iter().any(|v| !v.is_finite()) || (dot(normal, normal) - 1.).abs() > 1e-9 {
        return Err(invalid("Construction plane has an invalid normal"));
    }
    let raw = if normal[0].hypot(normal[1]) > 1e-7 {
        [-normal[1], normal[0], 0.]
    } else {
        let projection = normal[0];
        std::array::from_fn(|i| f64::from(i == 0) - projection * normal[i])
    };
    let length = dot(raw, raw).sqrt();
    Ok(raw.map(|v| v / length))
}
/// Inspect current plane geometry without using saved datum coordinates in place
/// of missing attachment evaluation. A Plane has no intrinsic in-plane roll.
pub fn inspect_plane(
    document: &Document,
    id: EntityId,
    revision: u64,
    report: Option<&ReferenceReport>,
) -> Result<Frame, DocumentError> {
    if report.is_some_and(|r| r.revision != revision) {
        return Err(invalid("Construction plane evaluation is stale"));
    }
    let Some(EntityData::Plane { normal, anchor, .. }) = document.get(id).map(|e| &e.data) else {
        return Err(invalid("Construction plane is missing"));
    };
    if anchor
        .as_ref()
        .is_some_and(|l| l.geometry.is_none() && l.role != OFFSET_PLANE_ROLE)
    {
        return Err(invalid("Unsupported construction plane anchor role"));
    }
    let frame = crate::references::plane_frame(
        document,
        id,
        plane_basis(*normal)?,
        report.map(|r| (revision, r)),
    )?;
    validate_frame(frame)?;
    Ok(frame)
}
fn offset_descriptor(entity: &Entity) -> Result<(EntityId, f64), DocumentError> {
    let EntityData::Plane {
        origin_mm,
        normal,
        anchor: Some(link),
    } = &entity.data
    else {
        return Err(invalid("Offset plane descriptor is missing"));
    };
    if link.role != OFFSET_PLANE_ROLE
        || link.geometry.is_some()
        || *normal != [0., 0., 1.]
        || origin_mm[0] != 0.
        || origin_mm[1] != 0.
        || !origin_mm[2].is_finite()
        || origin_mm[2].abs() > 1e9
    {
        return Err(invalid(
            "Offset plane requires parent-local +Z and a normal-only offset",
        ));
    }
    Ok((link.target, origin_mm[2]))
}
/// Explicit offset chains are bounded and remain attached to the current parent.
pub(crate) fn offset_plane_frame(
    document: &Document,
    plane: EntityId,
    u: [f64; 3],
    evaluated: Option<(u64, &ReferenceReport)>,
) -> Result<Frame, DocumentError> {
    if u.iter().any(|v| !v.is_finite()) || (dot(u, u) - 1.).abs() > 1e-9 || u[2].abs() > 1e-9 {
        return Err(invalid(
            "Offset-plane sketch U must be a unit vector in parent-local XY",
        ));
    }
    let mut current = plane;
    let mut offset = 0.;
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current) || visited.len() > 128 {
            return Err(invalid(
                "Offset plane chain is cyclic or exceeds 128 planes",
            ));
        }
        let entity = document
            .get(current)
            .ok_or_else(|| invalid("Offset plane parent is missing"))?;
        let EntityData::Plane { anchor, .. } = &entity.data else {
            return Err(invalid("Offset plane parent must be another Plane"));
        };
        if anchor.as_ref().is_some_and(|l| l.role == OFFSET_PLANE_ROLE) {
            let (parent, distance) = offset_descriptor(entity)?;
            offset += distance;
            current = parent;
        } else {
            let (revision, report) =
                evaluated.map_or((0, None), |(revision, report)| (revision, Some(report)));
            let base = inspect_plane(document, current, revision, report)?;
            let frame = Frame {
                origin: std::array::from_fn(|i| base.origin[i] + base.n[i] * offset),
                u: base.vector(u),
                v: base.vector(cross([0., 0., 1.], u)),
                n: base.n,
            };
            validate_frame(frame)?;
            return Ok(frame);
        }
    }
}
/// Snapshots must come from the host's current trusted geometry evaluator, in
/// canonical Feature coordinates. This checks owner, revision and provenance;
/// it never infers a face from proximity or stores a final Part/occurrence link.
pub fn prepare_new(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    plane: CreationPlane<'_>,
    options: PlaneOptions,
) -> Result<NewSketch, CommandError> {
    if session.revision() != expected_revision {
        return Err(command_error(
            ErrorCode::StaleRevision,
            "Sketch creation requires the current document revision",
        ));
    }
    options
        .validate()
        .map_err(|e| command_error(ErrorCode::InvalidRequest, e.to_string()))?;
    let mut edits = vec![];
    let (plane_id, fallback_u, frame) = match plane {
        CreationPlane::Datum(datum) => {
            let frame = options.apply(datum.frame())?;
            let parent = Entity::new(
                format!("{name} datum"),
                EntityData::Plane {
                    origin_mm: datum.frame().origin,
                    normal: datum.frame().n,
                    anchor: None,
                },
            );
            let parent_id = parent.id;
            if options == PlaneOptions::default() {
                edits.push(Mutation::Insert(parent));
                (parent_id, frame.u, frame)
            } else {
                let offset = Entity::new(
                    format!("{name} offset plane"),
                    EntityData::Plane {
                        origin_mm: [0., 0., options.normal_offset_mm],
                        normal: [0., 0., 1.],
                        anchor: Some(Link::new(parent_id, OFFSET_PLANE_ROLE)),
                    },
                );
                let id = offset.id;
                edits.extend([Mutation::Insert(parent), Mutation::Insert(offset)]);
                let (s, c) = options.angle_rad.sin_cos();
                (id, [c, s, 0.], frame)
            }
        }
        CreationPlane::Existing { id, report } => {
            let base = inspect_plane(session.document(), id, expected_revision, report)?;
            let frame = options.apply(base)?;
            if options == PlaneOptions::default() {
                let EntityData::Plane { normal, .. } =
                    session.document().get(id).expect("inspected").data
                else {
                    unreachable!("inspected Plane")
                };
                (id, plane_basis(normal)?, frame)
            } else {
                let offset = Entity::new(
                    format!("{name} offset plane"),
                    EntityData::Plane {
                        origin_mm: [0., 0., options.normal_offset_mm],
                        normal: [0., 0., 1.],
                        anchor: Some(Link::new(id, OFFSET_PLANE_ROLE)),
                    },
                );
                let id = offset.id;
                edits.push(Mutation::Insert(offset));
                let (s, c) = options.angle_rad.sin_cos();
                (id, [c, s, 0.], frame)
            }
        }
        CreationPlane::Face {
            snapshot,
            key,
            anchor,
        } => {
            if snapshot.revision != expected_revision {
                return Err(command_error(
                    ErrorCode::StaleRevision,
                    "Face snapshot is not current",
                ));
            }
            if !session.document().get(snapshot.owner).is_some_and(|e| {
                matches!(
                    e.data,
                    EntityData::Feature {
                        suppressed: false,
                        ..
                    }
                )
            }) {
                return Err(command_error(
                    ErrorCode::InvalidRequest,
                    "Face sketches require a canonical unsuppressed Feature producer, not a Part or Instance",
                ));
            }
            if key.kind != TopologyKind::Face {
                return Err(command_error(
                    ErrorCode::InvalidRequest,
                    "Choose a planar face",
                ));
            }
            let mut intent = GeometryIntent::capture(snapshot, key)
                .map_err(|e| command_error(ErrorCode::InvalidRequest, e))?;
            let candidate = snapshot
                .elements
                .iter()
                .find(|e| e.key == key)
                .expect("captured");
            let anchored = if let Some(anchor) = anchor {
                candidate.frames.iter().find(|f| f.origin == *anchor)
            } else if candidate.frames.len() == 1 {
                candidate.frames.first()
            } else {
                None
            }
            .ok_or_else(|| {
                command_error(
                    ErrorCode::InvalidRequest,
                    "Choose an available, unambiguous planar source frame",
                )
            })?;
            intent.attachment = Some(Attachment {
                anchor: anchored.origin.clone(),
                offset_mm: [0., 0., options.normal_offset_mm],
                angle_rad: options.angle_rad,
            });
            let Resolution::Resolved {
                frame: Some(resolved),
                ..
            } = crate::references::resolve(&intent, snapshot, expected_revision)
            else {
                return Err(command_error(
                    ErrorCode::InvalidRequest,
                    "Face attachment could not be resolved",
                ));
            };
            let frame = Frame {
                origin: resolved.origin_mm,
                u: resolved.u,
                v: resolved.v,
                n: resolved.normal(),
            };
            let mut link = Link::new(snapshot.owner, "face");
            link.geometry = Some(intent);
            let entity = Entity::new(
                format!("{name} face plane"),
                EntityData::Plane {
                    origin_mm: frame.origin,
                    normal: frame.n,
                    anchor: Some(link),
                },
            );
            let id = entity.id;
            edits.push(Mutation::Insert(entity));
            (id, frame.u, frame)
        }
    };
    let source = crate::constraint_sketch::source::Source {
        u: fallback_u,
        sketch: Default::default(),
    };
    let sketch = Entity::new(
        name,
        EntityData::Sketch {
            plane: plane_id,
            definition: source.definition()?,
            inputs: vec![],
        },
    );
    let sketch_id = sketch.id;
    edits.push(Mutation::Insert(sketch));
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "New sketch".into(),
        edits,
    })?;
    Ok(NewSketch {
        prepared,
        sketch: sketch_id,
        plane: plane_id,
        frame,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Line {
        id: EntityId,
        a: [f64; 2],
        b: [f64; 2],
    },
    Rectangle {
        id: EntityId,
        a: [f64; 2],
        b: [f64; 2],
    },
    Circle {
        id: EntityId,
        center: [f64; 2],
        radius_mm: f64,
    },
    Arc {
        id: EntityId,
        center: [f64; 2],
        a: [f64; 2],
        b: [f64; 2],
        clockwise: bool,
    },
}
impl Geometry {
    pub fn id(&self) -> EntityId {
        match self {
            Self::Line { id, .. }
            | Self::Rectangle { id, .. }
            | Self::Circle { id, .. }
            | Self::Arc { id, .. } => *id,
        }
    }
    pub fn segments(&self) -> Vec<[[f64; 2]; 2]> {
        match *self {
            Self::Line { a, b, .. } => vec![[a, b]],
            Self::Rectangle { a, b, .. } => {
                let c = [b[0], a[1]];
                let d = [a[0], b[1]];
                vec![[a, c], [c, b], [b, d], [d, a]]
            }
            Self::Circle {
                center, radius_mm, ..
            } => arc_segments(center, radius_mm, 0., std::f64::consts::TAU),
            Self::Arc {
                center,
                a,
                b,
                clockwise,
                ..
            } => {
                let start = (a[1] - center[1]).atan2(a[0] - center[0]);
                let end = (b[1] - center[1]).atan2(b[0] - center[0]);
                let sweep = if clockwise {
                    -((start - end).rem_euclid(std::f64::consts::TAU))
                } else {
                    (end - start).rem_euclid(std::f64::consts::TAU)
                };
                arc_segments(
                    center,
                    (a[0] - center[0]).hypot(a[1] - center[1]),
                    start,
                    sweep,
                )
            }
        }
    }
    fn validate(&self) -> Result<(), DocumentError> {
        if let Self::Circle {
            id,
            center,
            radius_mm,
        } = self
        {
            if id.is_nil()
                || center.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
                || !radius_mm.is_finite()
                || !(1e-7..=1e9).contains(radius_mm)
            {
                return Err(invalid("Invalid circle"));
            }
            return Ok(());
        }
        if let Self::Arc {
            id, center, a, b, ..
        } = self
        {
            let r = (a[0] - center[0]).hypot(a[1] - center[1]);
            let s = (b[0] - center[0]).hypot(b[1] - center[1]);
            if id.is_nil()
                || center
                    .iter()
                    .chain(a)
                    .chain(b)
                    .any(|v| !v.is_finite() || v.abs() > 1e9)
                || r < 1e-7
                || (r - s).abs() > 1e-7
                || (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7
            {
                return Err(invalid("Invalid circular arc"));
            }
            return Ok(());
        }
        let (a, b, rectangle) = match self {
            Self::Line { a, b, .. } => (*a, *b, false),
            Self::Rectangle { a, b, .. } => (*a, *b, true),
            _ => return Err(invalid("Unsupported geometry")),
        };
        if self.id().is_nil()
            || a.into_iter()
                .chain(b)
                .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid(
                "Sketch coordinates must be finite and within ±1,000 km",
            ));
        }
        let d = [b[0] - a[0], b[1] - a[1]];
        if (rectangle && (d[0].abs() < 1e-7 || d[1].abs() < 1e-7)) || d[0].hypot(d[1]) < 1e-7 {
            return Err(invalid(
                "Line length and rectangle sides must be greater than 0.0000001 mm",
            ));
        }
        Ok(())
    }
}
/// Display tessellation only; source circles/arcs retain their exact parameters.
fn arc_segments(center: [f64; 2], radius: f64, start: f64, sweep: f64) -> Vec<[[f64; 2]; 2]> {
    let count = ((sweep.abs() / std::f64::consts::TAU * 96.).ceil() as usize).clamp(2, 96);
    let point = |i: usize| {
        let angle = start + sweep * i as f64 / count as f64;
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]
    };
    (0..count).map(|i| [point(i), point(i + 1)]).collect()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sketch {
    pub u: [f64; 3],
    pub geometry: Vec<Geometry>,
}
fn invalid(s: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(s.into())
}
impl Sketch {
    pub fn read(
        document: &Document,
        id: EntityId,
    ) -> Result<(Self, Frame, EntityId), DocumentError> {
        Self::read_impl(document, id, None)
    }
    pub fn read_evaluated(
        document: &Document,
        id: EntityId,
        revision: u64,
        report: &crate::references::ReferenceReport,
    ) -> Result<(Self, Frame, EntityId), DocumentError> {
        Self::read_impl(document, id, Some((revision, report)))
    }
    fn read_impl(
        document: &Document,
        id: EntityId,
        evaluated: Option<(u64, &crate::references::ReferenceReport)>,
    ) -> Result<(Self, Frame, EntityId), DocumentError> {
        let entity = document
            .get(id)
            .ok_or_else(|| invalid("Sketch does not exist"))?;
        let EntityData::Sketch {
            plane, definition, ..
        } = &entity.data
        else {
            return Err(invalid("Select a sketch"));
        };
        if definition.type_id == crate::constraint_sketch::source::TYPE {
            use crate::constraint_sketch::source::Source;
            let (source, frame, plane) = if let Some((revision, report)) = evaluated {
                Source::read_evaluated(document, id, revision, report)?
            } else {
                Source::read(document, id)?
            };
            let geometry = source.display_geometry()?;
            return Ok((
                Self {
                    u: source.u,
                    geometry,
                },
                frame,
                plane,
            ));
        }
        if definition.type_id != TYPE || definition.schema_version != 1 {
            return Err(invalid("Unsupported sketch provider or version"));
        }
        let source = Self::source(definition)?;
        let EntityData::Plane { normal, .. } = &document
            .get(*plane)
            .ok_or_else(|| invalid("Sketch plane is missing"))?
            .data
        else {
            return Err(invalid("Sketch requires a construction plane"));
        };
        if (dot(source.u, source.u) - 1.).abs() > 1e-9
            || dot(source.u, *normal).abs() > 1e-9
            || source.u.iter().any(|v| !v.is_finite())
        {
            return Err(invalid("Sketch basis must be orthonormal to its plane"));
        }
        let frame = crate::references::plane_frame(document, *plane, source.u, evaluated)?;
        Ok((source, frame, *plane))
    }
    fn source(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.schema_version != 1 {
            return Err(invalid("Unsupported sketch version"));
        }
        let source: Self = serde_json::from_value(definition.data.clone())
            .map_err(|e| invalid(format!("Invalid sketch source: {e}")))?;
        if source.u.iter().any(|v| !v.is_finite()) || (dot(source.u, source.u) - 1.).abs() > 1e-9 {
            return Err(invalid("Invalid sketch basis"));
        }
        if source.geometry.len() > 100_000 {
            return Err(invalid("Sketch geometry limit exceeded"));
        }
        let mut ids = BTreeSet::new();
        for g in &source.geometry {
            g.validate()?;
            if !ids.insert(g.id()) {
                return Err(invalid("Duplicate sketch geometry identity"));
            }
        }
        Ok(source)
    }
    fn definition(&self) -> Result<Definition, DocumentError> {
        Ok(Definition {
            type_id: TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
}
pub fn create(datum: Datum, name: &str) -> Result<(EntityId, Vec<Mutation>), DocumentError> {
    let f = datum.frame();
    let plane = Entity::new(
        format!("{name} plane"),
        EntityData::Plane {
            origin_mm: f.origin,
            normal: f.n,
            anchor: None,
        },
    );
    let sketch = Entity::new(
        name,
        EntityData::Sketch {
            plane: plane.id,
            definition: Sketch {
                u: f.u,
                geometry: vec![],
            }
            .definition()?,
            inputs: vec![],
        },
    );
    Ok((
        sketch.id,
        vec![Mutation::Insert(plane), Mutation::Insert(sketch)],
    ))
}
pub fn append(
    document: &Document,
    id: EntityId,
    geometry: Geometry,
) -> Result<Vec<Mutation>, DocumentError> {
    geometry.validate()?;
    let (mut source, _, _) = Sketch::read(document, id)?;
    source.geometry.push(geometry);
    replace(document, id, source)
}
pub fn remove_geometry(
    document: &Document,
    id: EntityId,
    geometry: EntityId,
) -> Result<Vec<Mutation>, DocumentError> {
    let (mut source, _, _) = Sketch::read(document, id)?;
    let before = source.geometry.len();
    source.geometry.retain(|g| g.id() != geometry);
    if source.geometry.len() == before {
        return Err(invalid("Select a sketch element to delete"));
    }
    replace(document, id, source)
}
fn replace(
    document: &Document,
    id: EntityId,
    source: Sketch,
) -> Result<Vec<Mutation>, DocumentError> {
    let mut entity = document
        .get(id)
        .ok_or_else(|| invalid("Sketch is missing"))?
        .clone();
    if let EntityData::Sketch { definition, .. } = &mut entity.data {
        if definition.type_id != TYPE {
            return Err(invalid(
                "Constrained sketches require the solver transaction API",
            ));
        }
        *definition = source.definition()?;
    }
    Ok(vec![Mutation::Replace(entity)])
}
/// Detach the sketch onto an owned translated plane, preserving shared datums.
pub fn translate(
    document: &Document,
    id: EntityId,
    world_delta: [f64; 3],
) -> Result<Vec<Mutation>, DocumentError> {
    if world_delta.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
        return Err(invalid("Translation must be finite and within ±1,000 km"));
    }
    let (_, frame, plane) = Sketch::read(document, id)?;
    let mut moved = document
        .get(plane)
        .ok_or_else(|| invalid("Plane is missing"))?
        .clone();
    let shared = document
        .entities()
        .any(|e| e.id != id && e.references().iter().any(|l| l.target == plane));
    let mut edits = vec![];
    let mut owner = document
        .get(id)
        .ok_or_else(|| invalid("Sketch is missing"))?
        .clone();
    if shared {
        moved.id = EntityId::new();
        if let EntityData::Sketch { plane, .. } = &mut owner.data {
            *plane = moved.id;
        }
    }
    // An offset source stores U in parent-local coordinates. Detaching onto an
    // absolute plane must convert that descriptor to its evaluated world basis.
    if let EntityData::Sketch { definition, .. } = &mut owner.data {
        if definition.type_id == crate::constraint_sketch::source::TYPE {
            let (mut source, _, _) = crate::constraint_sketch::source::Source::read(document, id)?;
            source.u = frame.u;
            *definition = source.definition()?;
        } else {
            let (mut source, _, _) = Sketch::read(document, id)?;
            source.u = frame.u;
            *definition = source.definition()?;
        }
    }
    if document.get(id) != Some(&owner) {
        edits.push(Mutation::Replace(owner));
    }
    moved.data = EntityData::Plane {
        origin_mm: std::array::from_fn(|i| frame.origin[i] + world_delta[i]),
        normal: frame.n,
        anchor: None,
    };
    edits.push(if shared {
        Mutation::Insert(moved)
    } else {
        Mutation::Replace(moved)
    });
    Ok(edits)
}
/// Built-in provider validation runs at the canonical document boundary.
pub(crate) fn validate(document: &Document) -> Result<(), DocumentError> {
    for e in document.entities() {
        if let EntityData::Plane {
            anchor: Some(link), ..
        } = &e.data
            && link.role == OFFSET_PLANE_ROLE
        {
            let (parent, _) = offset_descriptor(e)?;
            if document
                .get(parent)
                .is_some_and(|e| !matches!(e.data, EntityData::Plane { .. }))
            {
                return Err(invalid("Offset plane parent must be another Plane"));
            }
        }
        if let EntityData::Sketch {
            plane, definition, ..
        } = &e.data
            && definition.type_id == TYPE
        {
            let source = Sketch::source(definition)?;
            if let Some(Entity {
                data: EntityData::Plane { normal, .. },
                ..
            }) = document.get(*plane)
                && dot(source.u, *normal).abs() > 1e-9
            {
                return Err(invalid("Sketch basis must be orthogonal to its plane"));
            }
        }
    }
    Ok(())
}
