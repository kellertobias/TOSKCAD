//! Source-only datum sketches. Coordinates are millimeters, never display pixels.
use crate::document::{
    Definition, Document, DocumentError, Entity, EntityData, EntityId, Mutation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const TYPE: &str = "core.datum_sketch";
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
            let (source, frame, plane) =
                crate::constraint_sketch::source::Source::read(document, id)?;
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
        let EntityData::Plane {
            origin_mm, normal, ..
        } = &document
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
        let frame = Frame {
            origin: *origin_mm,
            u: source.u,
            v: cross(*normal, source.u),
            n: *normal,
        };
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
    if shared {
        moved.id = EntityId::new();
        let mut owner = document
            .get(id)
            .ok_or_else(|| invalid("Sketch is missing"))?
            .clone();
        if let EntityData::Sketch { plane, .. } = &mut owner.data {
            *plane = moved.id;
        }
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
