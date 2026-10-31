//! Panel machining: grooves, rabbets, pockets, drilled holes and placed
//! hardware hole patterns. An operation is a document extension that targets
//! one or more rectangular panels; each target Part gets its own
//! `cabinetry.machining_cut` feature. Positions are in the panel's (u, v)
//! face coordinates, so an operation follows the panel when it moves or is
//! resized. These are modeled features, not CNC toolpaths.
use super::*;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::Mutation,
    model::{
        BodyInput, CreatedBody, ModelPrepared,
        common::{error, number, revision},
    },
    parameters::EvaluatedParameters,
};

pub const MACHINING_KIND: &str = "cabinetry.machining";
pub const MACHINING_CAPABILITY: &str = "cabinetry.machining.1";
pub const MACHINING_CUT_TYPE: &str = "cabinetry.machining_cut";
pub const MACHINING_CUT_SCHEMA: u32 = 1;
pub const MACHINING_CUT_CAPABILITY: &str = "cabinetry.machining_cut.1";
pub const HARDWARE_PATTERN_KIND: &str = "cabinetry.hardware";
pub const HARDWARE_PATTERN_CAPABILITY: &str = "cabinetry.hardware.1";
pub const MAX_TARGETS: usize = 64;
pub const MAX_HOLES: usize = 256;
pub const MAX_PIN_ROWS: usize = 8;
/// The System 32 hole pitch.
pub const SYSTEM32_PITCH_MM: f64 = 32.;
const MARGIN_MM: f64 = 1.;
const TOL: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelEdgeName {
    UStart,
    UEnd,
    VStart,
    VEnd,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelAxis {
    U,
    V,
}
/// Where an operation enters the panel. Face coordinates are the panel's
/// (u, v) on both faces. On an edge face, the first coordinate runs along
/// the edge from its start and the second up the thickness from the bottom
/// face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "face", rename_all = "snake_case", deny_unknown_fields)]
pub enum MachiningFace {
    /// The face at the full thickness, cut toward the bottom.
    Top,
    /// The face at the panel plane, cut toward the top.
    Bottom,
    Edge {
        edge: PanelEdgeName,
    },
    /// The top or bottom face that looks into the cabinet: toward the
    /// carcass interior for a carcass panel, otherwise toward the other
    /// target panels. Each target resolves its own face, so the inner faces
    /// of a left and a right side take one operation.
    Inside,
}
/// The edge a shelf-pin row offset is measured from, across the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowEdge {
    Start,
    End,
}
/// One row of shelf-pin holes, `offset` from an edge across the row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShelfPinRow {
    pub from: RowEdge,
    pub offset: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MachiningKind {
    /// A groove or dado running the full panel length along `along`,
    /// `offset` from the start edge of the other axis.
    Groove {
        along: PanelAxis,
        offset: Dimension,
        width: Dimension,
        depth: Dimension,
    },
    /// A rabbet along one edge.
    Rabbet {
        edge: PanelEdgeName,
        width: Dimension,
        depth: Dimension,
    },
    /// A rectangular pocket centered at `center`.
    Pocket {
        center: [Dimension; 2],
        size: [Dimension; 2],
        depth: Dimension,
    },
    /// Holes of one diameter; no depth drills through.
    Drill {
        holes: Vec<[Dimension; 2]>,
        diameter: Dimension,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<Dimension>,
    },
    /// System 32 style shelf-pin rows: each row runs along `along` with
    /// holes every `pitch` from `start` (from the start edge) up to `end`
    /// (no hole center closer to the end edge). `count` fixes the number of
    /// holes per row; without it the row fills the range. Blind holes only.
    ShelfPins {
        along: PanelAxis,
        rows: Vec<ShelfPinRow>,
        pitch: Dimension,
        start: Dimension,
        end: Dimension,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
        diameter: Dimension,
        depth: Dimension,
    },
    /// A reusable hardware hole pattern placed at `at`, turned by
    /// `quarter_turns` × 90° counterclockwise in the face coordinates.
    Hardware {
        pattern: EntityId,
        at: [Dimension; 2],
        #[serde(default)]
        quarter_turns: u8,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachiningDefinition {
    pub kind: MachiningKind,
    pub face: MachiningFace,
    /// Panel Parts that receive the operation.
    pub targets: Vec<EntityId>,
    /// A suppressed operation keeps its cuts in each Part's history but
    /// removes no material and reports no holes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareHole {
    pub at: [Dimension; 2],
    pub diameter: Dimension,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Dimension>,
}
/// Added to the pattern's diameters and blind depths in panels of this
/// manufacturing Material.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialAllowance {
    pub material: EntityId,
    pub diameter: Dimension,
    pub depth: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwarePatternDefinition {
    pub holes: Vec<HardwareHole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowances: Vec<MaterialAllowance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachiningCutDefinition {
    pub operation: EntityId,
    pub input: BodyInput,
    pub output: BodyOutput,
}

impl HardwarePatternDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.holes.is_empty() || self.holes.len() > MAX_HOLES {
            return Err(invalid(format!(
                "A hardware pattern needs 1–{MAX_HOLES} holes"
            )));
        }
        let mut materials = BTreeSet::new();
        if self
            .allowances
            .iter()
            .any(|a| !materials.insert(a.material))
        {
            return Err(invalid("A material has two allowances in one pattern"));
        }
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut out = vec![];
        for h in &self.holes {
            out.extend(&h.at);
            out.push(&h.diameter);
            out.extend(&h.depth);
        }
        for a in &self.allowances {
            out.extend([&a.diameter, &a.depth]);
        }
        out
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        let mut inputs = parameter_links(self.dimensions());
        inputs.extend(
            self.allowances
                .iter()
                .map(|a| Link::new(a.material, format!("material/{}", a.material))),
        );
        Ok(EntityData::Extension {
            definition: definition(HARDWARE_PATTERN_KIND, self)?,
            inputs,
        })
    }
}
impl MachiningDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.targets.is_empty() || self.targets.len() > MAX_TARGETS {
            return Err(invalid(format!(
                "Machining needs 1–{MAX_TARGETS} target panels"
            )));
        }
        if self.targets.iter().collect::<BTreeSet<_>>().len() != self.targets.len() {
            return Err(invalid("A panel is targeted twice"));
        }
        let edge = matches!(self.face, MachiningFace::Edge { .. });
        match &self.kind {
            MachiningKind::Groove { .. }
            | MachiningKind::Rabbet { .. }
            | MachiningKind::Pocket { .. }
                if edge =>
            {
                return Err(invalid(
                    "Grooves, rabbets and pockets are cut into the top or bottom face",
                ));
            }
            MachiningKind::ShelfPins { .. } if edge => {
                return Err(invalid(
                    "Shelf-pin rows are drilled into a top, bottom or inside face",
                ));
            }
            MachiningKind::ShelfPins { rows, count, .. }
                if rows.is_empty()
                    || rows.len() > MAX_PIN_ROWS
                    || count.is_some_and(|c| c == 0 || c as usize > MAX_HOLES) =>
            {
                return Err(invalid(format!(
                    "Shelf pins need 1–{MAX_PIN_ROWS} rows and 1–{MAX_HOLES} holes per row"
                )));
            }
            MachiningKind::Drill { holes, .. } if holes.is_empty() || holes.len() > MAX_HOLES => {
                return Err(invalid(format!("A drilling needs 1–{MAX_HOLES} holes")));
            }
            MachiningKind::Hardware {
                pattern,
                quarter_turns,
                ..
            } if pattern.is_nil() || *quarter_turns > 3 => {
                return Err(invalid(
                    "A hardware placement needs a pattern and 0–3 quarter turns",
                ));
            }
            _ => {}
        }
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        match &self.kind {
            MachiningKind::Groove {
                offset,
                width,
                depth,
                ..
            } => vec![offset, width, depth],
            MachiningKind::Rabbet { width, depth, .. } => vec![width, depth],
            MachiningKind::Pocket {
                center,
                size,
                depth,
            } => center.iter().chain(size).chain([depth]).collect(),
            MachiningKind::Drill {
                holes,
                diameter,
                depth,
            } => holes
                .iter()
                .flatten()
                .chain([diameter])
                .chain(depth)
                .collect(),
            MachiningKind::ShelfPins {
                rows,
                pitch,
                start,
                end,
                diameter,
                depth,
                ..
            } => rows
                .iter()
                .map(|r| &r.offset)
                .chain([pitch, start, end, diameter, depth])
                .collect(),
            MachiningKind::Hardware { at, .. } => at.iter().collect(),
        }
    }
    fn inputs(&self, document: &Document) -> Result<Vec<Link>, DocumentError> {
        let mut links = parameter_links(self.dimensions());
        for (i, part) in self.targets.iter().enumerate() {
            let feature = panel_feature(document, *part)
                .ok_or_else(|| invalid(format!("{part} is not a panel")))?;
            links.push(Link::new(feature, format!("target/{i}")));
        }
        if let MachiningKind::Hardware { pattern, .. } = &self.kind {
            links.push(Link::new(*pattern, "pattern"));
        }
        Ok(links)
    }
    pub(crate) fn entity_data(&self, document: &Document) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(MACHINING_KIND, self)?,
            inputs: self.inputs(document)?,
        })
    }
    pub fn label(&self) -> &'static str {
        match self.kind {
            MachiningKind::Groove { .. } => "groove",
            MachiningKind::Rabbet { .. } => "rabbet",
            MachiningKind::Pocket { .. } => "pocket",
            MachiningKind::Drill { .. } => "drilling",
            MachiningKind::ShelfPins { .. } => "shelf pins",
            MachiningKind::Hardware { .. } => "hardware",
        }
    }
}
impl MachiningCutDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.operation.is_nil()
            || self.input.producer.is_nil()
            || self.output.body != self.input.body
        {
            return Err(invalid("A machining cut modifies its own Part"));
        }
        definition(MACHINING_CUT_TYPE, self)
    }
    pub fn inputs(&self) -> Vec<Link> {
        vec![
            Link::new(self.input.producer, "source"),
            Link::new(self.operation, "operation"),
        ]
    }
}

pub fn machinings(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| is_kind(e, MACHINING_KIND))
}
pub fn hardware_patterns(document: &Document) -> impl Iterator<Item = &Entity> {
    document
        .entities()
        .filter(|e| is_kind(e, HARDWARE_PATTERN_KIND))
}
pub fn machining_cuts(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == MACHINING_CUT_TYPE),
    )
}
pub fn read_machining(
    document: &Document,
    id: EntityId,
) -> Result<MachiningDefinition, DocumentError> {
    let value: MachiningDefinition =
        parse(&extension(document, id, MACHINING_KIND)?, MACHINING_KIND)?;
    value.validate()?;
    Ok(value)
}
pub fn read_hardware_pattern(
    document: &Document,
    id: EntityId,
) -> Result<HardwarePatternDefinition, DocumentError> {
    let value: HardwarePatternDefinition = parse(
        &extension(document, id, HARDWARE_PATTERN_KIND)?,
        HARDWARE_PATTERN_KIND,
    )?;
    value.validate()?;
    Ok(value)
}
pub fn read_machining_cut(
    document: &Document,
    feature: EntityId,
) -> Result<MachiningCutDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a machining cut feature"));
    };
    let value: MachiningCutDefinition = parse(definition, MACHINING_CUT_TYPE)?;
    value.definition()?;
    let expected = value.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid(
            "Machining cut source and canonical input links disagree",
        ));
    }
    Ok(value)
}
/// Machining operations that target a Part.
pub fn machinings_of(document: &Document, part: EntityId) -> Vec<EntityId> {
    machinings(document)
        .filter(|e| read_machining(document, e.id).is_ok_and(|m| m.targets.contains(&part)))
        .map(|e| e.id)
        .collect()
}
pub(super) fn validate_machining(document: &Document) -> Result<(), DocumentError> {
    for e in hardware_patterns(document) {
        read_hardware_pattern(document, e.id)?;
    }
    for e in machinings(document) {
        read_machining(document, e.id)?;
    }
    for e in machining_cuts(document) {
        read_machining_cut(document, e.id)?;
    }
    Ok(())
}

/// One drilled hole in world millimeters. `axis` points into the material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedHole {
    /// Center in the entry face's coordinates.
    pub at: [f64; 2],
    pub center: [f64; 3],
    pub axis: [f64; 3],
    pub diameter_mm: f64,
    pub depth_mm: f64,
    pub through: bool,
}
/// A groove, rabbet or pocket: length and width on the face, and depth.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRecess {
    pub kind: &'static str,
    pub size_mm: [f64; 2],
    pub depth_mm: f64,
    /// The recess outline on the face, in face coordinates.
    pub outline: Vec<[f64; 2]>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMachining {
    /// The face actually cut: `Inside` resolves to the top or bottom face.
    pub face: MachiningFace,
    /// The frame of that face.
    pub frame: MachiningFaceFrame,
    pub prisms: Vec<CutPrism>,
    pub holes: Vec<ResolvedHole>,
    pub recesses: Vec<ResolvedRecess>,
}

/// A face plane: a world point at face coordinates (0, 0), the two in-face
/// axes, the inward direction and the extents of the face and into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MachiningFaceFrame {
    pub origin: [f64; 3],
    pub a: [f64; 3],
    pub b: [f64; 3],
    pub inward: [f64; 3],
    pub size: [f64; 2],
    pub depth: f64,
}
type Face = MachiningFaceFrame;
/// The frame of one face of a panel box. `Inside` must be resolved first
/// (see [`resolve_machining_face`]); here it reads as the bottom face.
pub fn panel_face_frame(b: &PanelBox, which: MachiningFace) -> MachiningFaceFrame {
    face(b, which)
}
/// The rectangular box of a panel Part, for face placement.
pub fn machining_panel_box(
    document: &Document,
    parameters: &EvaluatedParameters,
    part: EntityId,
) -> Result<PanelBox, DocumentError> {
    super::joint::panel_box(document, parameters, part)
}
fn box_center(b: &PanelBox) -> [f64; 3] {
    let n = b.normal();
    let p = add(add(b.origin, b.u, b.length_mm / 2.), b.v, b.width_mm / 2.);
    add(p, n, b.thickness_mm / 2.)
}
/// The face an operation enters on one target. `Inside` becomes the top or
/// bottom face toward the carcass interior (carcass panels) or toward the
/// other targets.
pub fn resolve_machining_face(
    document: &Document,
    parameters: &EvaluatedParameters,
    machining: &MachiningDefinition,
    target: EntityId,
) -> Result<MachiningFace, DocumentError> {
    if machining.face != MachiningFace::Inside {
        return Ok(machining.face);
    }
    let name = document
        .get(target)
        .map_or("A panel".into(), |e| e.name.clone());
    let b = super::joint::panel_box(document, parameters, target)?;
    let source = panel_feature(document, target)
        .map(|f| read_panel(document, f))
        .transpose()?;
    let toward = match source.map(|p| p.source) {
        Some(PanelSource::Carcass { carcass, .. }) => {
            let c = resolve_carcass(document, parameters, &read_carcass(document, carcass)?)?;
            let [w, h, d] = c.size_mm;
            let (s, k) = c.rotation_rad.sin_cos();
            let local = [w / 2., d / 2., h / 2.];
            [
                c.origin[0] + local[0] * k - local[1] * s,
                c.origin[1] + local[0] * s + local[1] * k,
                c.origin[2] + local[2],
            ]
        }
        _ => {
            let others: Vec<_> = machining
                .targets
                .iter()
                .filter(|t| **t != target)
                .map(|t| super::joint::panel_box(document, parameters, *t).map(|b| box_center(&b)))
                .collect::<Result<_, _>>()?;
            if others.is_empty() {
                return Err(invalid(format!(
                    "{name} is not a carcass panel; the inside face needs a carcass or other target panels to face"
                )));
            }
            let n = others.len() as f64;
            std::array::from_fn(|i| others.iter().map(|c| c[i]).sum::<f64>() / n)
        }
    };
    let c = box_center(&b);
    let side = dot3(
        [toward[0] - c[0], toward[1] - c[1], toward[2] - c[2]],
        b.normal(),
    );
    if side.abs() <= 1e-3 {
        return Err(invalid(format!(
            "Neither face of {name} looks into the cabinet; choose the top or bottom face"
        )));
    }
    Ok(if side > 0. {
        MachiningFace::Top
    } else {
        MachiningFace::Bottom
    })
}
fn add(p: [f64; 3], d: [f64; 3], k: f64) -> [f64; 3] {
    [p[0] + d[0] * k, p[1] + d[1] * k, p[2] + d[2] * k]
}
fn face(b: &PanelBox, which: MachiningFace) -> Face {
    let n = b.normal();
    let (l, w, t) = (b.length_mm, b.width_mm, b.thickness_mm);
    let neg = |d: [f64; 3]| d.map(|c| -c);
    match which {
        MachiningFace::Top => Face {
            origin: add(b.origin, n, t),
            a: b.u,
            b: b.v,
            inward: neg(n),
            size: [l, w],
            depth: t,
        },
        MachiningFace::Bottom | MachiningFace::Inside => Face {
            origin: b.origin,
            a: b.u,
            b: b.v,
            inward: n,
            size: [l, w],
            depth: t,
        },
        MachiningFace::Edge { edge } => match edge {
            PanelEdgeName::UStart => Face {
                origin: b.origin,
                a: b.v,
                b: n,
                inward: b.u,
                size: [w, t],
                depth: l,
            },
            PanelEdgeName::UEnd => Face {
                origin: add(b.origin, b.u, l),
                a: b.v,
                b: n,
                inward: neg(b.u),
                size: [w, t],
                depth: l,
            },
            PanelEdgeName::VStart => Face {
                origin: b.origin,
                a: b.u,
                b: n,
                inward: b.v,
                size: [l, t],
                depth: w,
            },
            PanelEdgeName::VEnd => Face {
                origin: add(b.origin, b.v, w),
                a: b.u,
                b: n,
                inward: neg(b.v),
                size: [l, t],
                depth: w,
            },
        },
    }
}
impl Face {
    /// A prism from just outside the face to `depth` inside it.
    fn prism(
        &self,
        polygon: Vec<[f64; 2]>,
        circle: Option<([f64; 2], f64)>,
        depth: f64,
    ) -> CutPrism {
        let origin = add(self.origin, self.inward, -MARGIN_MM);
        let flip = dot3(cross(self.a, self.b), self.inward) < 0.;
        let swap = |[p, q]: [f64; 2]| if flip { [q, p] } else { [p, q] };
        let mut polygon: Vec<_> = polygon.into_iter().map(swap).collect();
        if flip {
            polygon.reverse();
        }
        CutPrism {
            origin,
            u: if flip { self.b } else { self.a },
            v: if flip { self.a } else { self.b },
            polygon,
            circle: circle.map(|(c, r)| (swap(c), r)),
            sweep: self.inward.map(|c| c * (depth + MARGIN_MM)),
        }
    }
    /// The world point at face coordinates `p`.
    pub fn point(&self, p: [f64; 2]) -> [f64; 3] {
        add(add(self.origin, self.a, p[0]), self.b, p[1])
    }
    /// Face coordinates of a world point, and its distance into the face.
    pub fn local(&self, w: [f64; 3]) -> ([f64; 2], f64) {
        let d = [
            w[0] - self.origin[0],
            w[1] - self.origin[1],
            w[2] - self.origin[2],
        ];
        ([dot3(d, self.a), dot3(d, self.b)], dot3(d, self.inward))
    }
}
fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn clamp_outline(polygon: &[[f64; 2]], size: [f64; 2]) -> Vec<[f64; 2]> {
    polygon
        .iter()
        .map(|p| [p[0].clamp(0., size[0]), p[1].clamp(0., size[1])])
        .collect()
}
fn rect(a: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
    vec![[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]]
}

/// Hole centers of shelf-pin rows on a face of `size` (face coordinates),
/// row by row from the start edge. Refuses rows that leave the face and
/// ranges that hold no hole or fewer than a fixed count.
pub fn shelf_pin_points(
    parameters: &EvaluatedParameters,
    kind: &MachiningKind,
    size: [f64; 2],
    name: &str,
) -> Result<Vec<[f64; 2]>, DocumentError> {
    let MachiningKind::ShelfPins {
        along,
        rows,
        pitch,
        start,
        end,
        count,
        ..
    } = kind
    else {
        return Ok(vec![]);
    };
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let (pitch, start, end) = (mm(pitch)?, mm(start)?, mm(end)?);
    let (length, across) = match along {
        PanelAxis::U => (size[0], size[1]),
        PanelAxis::V => (size[1], size[0]),
    };
    if pitch <= TOL {
        return Err(invalid("The shelf-pin pitch must be positive"));
    }
    if start < -TOL || end < -TOL {
        return Err(invalid("Shelf-pin start and end limits cannot be negative"));
    }
    let last = length - end;
    if start > last + TOL {
        return Err(invalid(format!(
            "The shelf-pin range from {start} mm to {end} mm before the end leaves no room on the {length} mm side of {name}"
        )));
    }
    let fits = ((last - start) / pitch + 1e-9).floor() as usize + 1;
    let n = match count {
        Some(c) if *c as usize > fits => {
            return Err(invalid(format!(
                "{c} holes at {pitch} mm from {start} mm pass the end limit of {name}; {fits} fit"
            )));
        }
        Some(c) => *c as usize,
        None => fits,
    };
    if n * rows.len() > MAX_HOLES {
        return Err(invalid(format!(
            "The rows make {} holes on {name}; at most {MAX_HOLES} are allowed",
            n * rows.len()
        )));
    }
    let mut out = vec![];
    for row in rows {
        let offset = mm(&row.offset)?;
        let c = match row.from {
            RowEdge::Start => offset,
            RowEdge::End => across - offset,
        };
        for k in 0..n {
            let a = start + k as f64 * pitch;
            out.push(match along {
                PanelAxis::U => [a, c],
                PanelAxis::V => [c, a],
            });
        }
    }
    Ok(out)
}

/// Resolve an operation on one target panel, refusing cuts that leave no
/// material, leave the face or cannot be reached from it.
pub fn resolve_machining(
    document: &Document,
    parameters: &EvaluatedParameters,
    machining: &MachiningDefinition,
    target: EntityId,
) -> Result<ResolvedMachining, DocumentError> {
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    let name = document
        .get(target)
        .map_or("A panel".into(), |e| e.name.clone());
    let b = super::joint::panel_box(document, parameters, target)?;
    let which = resolve_machining_face(document, parameters, machining, target)?;
    let f = face(&b, which);
    let m = MARGIN_MM;
    let [fl, fw] = f.size;
    let blind = |d: f64, what: &str| {
        if d <= TOL {
            Err(invalid(format!("The {what} depth must be positive")))
        } else if d >= f.depth - TOL {
            Err(invalid(format!(
                "The {what} is {d} mm deep but {name} is only {} mm there; it would cut through",
                f.depth
            )))
        } else {
            Ok(d)
        }
    };
    let mut out = ResolvedMachining {
        face: which,
        frame: f,
        prisms: vec![],
        holes: vec![],
        recesses: vec![],
    };
    if machining.suppressed {
        return Ok(out);
    }
    let drill = |out: &mut ResolvedMachining, at: [f64; 2], dia: f64, depth: Option<f64>| {
        let r = dia / 2.;
        if dia <= TOL {
            return Err(invalid("Hole diameters must be positive"));
        }
        if at[0] - r < -TOL || at[0] + r > fl + TOL || at[1] - r < -TOL || at[1] + r > fw + TOL {
            return Err(invalid(format!(
                "A {dia} mm hole at ({}, {}) leaves the {fl} × {fw} mm face of {name}",
                at[0], at[1]
            )));
        }
        let (d, through) = match depth {
            Some(d) => (blind(d, "hole")?, false),
            None => (f.depth + m, true),
        };
        out.prisms.push(f.prism(vec![], Some((at, r)), d));
        if let Some(other) = out.holes.iter().find(|h| {
            let gap = ((h.at[0] - at[0]).powi(2) + (h.at[1] - at[1]).powi(2)).sqrt();
            gap < (h.diameter_mm + dia) / 2. - TOL
        }) {
            return Err(invalid(format!(
                "Holes at ({}, {}) and ({}, {}) on {name} overlap; space them at least one diameter apart",
                other.at[0], other.at[1], at[0], at[1]
            )));
        }
        if out.holes.len() >= MAX_HOLES {
            return Err(invalid(format!(
                "The operation makes more than {MAX_HOLES} holes on {name}"
            )));
        }
        out.holes.push(ResolvedHole {
            at,
            center: f.point(at),
            axis: f.inward,
            diameter_mm: dia,
            depth_mm: if through { f.depth } else { d },
            through,
        });
        Ok(())
    };
    match &machining.kind {
        MachiningKind::Groove {
            along,
            offset,
            width,
            depth,
        } => {
            let (o, w, d) = (mm(offset)?, mm(width)?, blind(mm(depth)?, "groove")?);
            let across = if *along == PanelAxis::U { fw } else { fl };
            if w <= TOL || o < -TOL || o + w > across + TOL {
                return Err(invalid(format!(
                    "The {w} mm groove at {o} mm does not fit across the {across} mm panel"
                )));
            }
            let polygon = match along {
                PanelAxis::U => rect([-m, o], [fl + m, o + w]),
                PanelAxis::V => rect([o, -m], [o + w, fw + m]),
            };
            let outline = clamp_outline(&polygon, f.size);
            out.prisms.push(f.prism(polygon, None, d));
            let length = if *along == PanelAxis::U { fl } else { fw };
            out.recesses.push(ResolvedRecess {
                kind: "groove",
                size_mm: [length, w],
                depth_mm: d,
                outline,
            });
        }
        MachiningKind::Rabbet { edge, width, depth } => {
            let (w, d) = (mm(width)?, blind(mm(depth)?, "rabbet")?);
            let (across, length) = match edge {
                PanelEdgeName::UStart | PanelEdgeName::UEnd => (fl, fw),
                _ => (fw, fl),
            };
            if w <= TOL || w >= across - TOL {
                return Err(invalid(format!(
                    "A {w} mm rabbet does not leave material on the {across} mm panel"
                )));
            }
            let polygon = match edge {
                PanelEdgeName::UStart => rect([-m, -m], [w, fw + m]),
                PanelEdgeName::UEnd => rect([fl - w, -m], [fl + m, fw + m]),
                PanelEdgeName::VStart => rect([-m, -m], [fl + m, w]),
                PanelEdgeName::VEnd => rect([-m, fw - w], [fl + m, fw + m]),
            };
            let outline = clamp_outline(&polygon, f.size);
            out.prisms.push(f.prism(polygon, None, d));
            out.recesses.push(ResolvedRecess {
                kind: "rabbet",
                size_mm: [length, w],
                depth_mm: d,
                outline,
            });
        }
        MachiningKind::Pocket {
            center,
            size,
            depth,
        } => {
            let (c, s, d) = (
                [mm(&center[0])?, mm(&center[1])?],
                [mm(&size[0])?, mm(&size[1])?],
                blind(mm(depth)?, "pocket")?,
            );
            let lo = [c[0] - s[0] / 2., c[1] - s[1] / 2.];
            let hi = [c[0] + s[0] / 2., c[1] + s[1] / 2.];
            if s[0] <= TOL
                || s[1] <= TOL
                || lo[0] < -TOL
                || lo[1] < -TOL
                || hi[0] > fl + TOL
                || hi[1] > fw + TOL
            {
                return Err(invalid(format!(
                    "The {} × {} mm pocket leaves the face of {name}",
                    s[0], s[1]
                )));
            }
            let outline = rect(lo, hi);
            out.prisms.push(f.prism(rect(lo, hi), None, d));
            out.recesses.push(ResolvedRecess {
                kind: "pocket",
                size_mm: s,
                depth_mm: d,
                outline,
            });
        }
        MachiningKind::Drill {
            holes,
            diameter,
            depth,
        } => {
            let dia = mm(diameter)?;
            let depth = depth.as_ref().map(mm).transpose()?;
            for h in holes {
                drill(&mut out, [mm(&h[0])?, mm(&h[1])?], dia, depth)?;
            }
        }
        MachiningKind::ShelfPins {
            diameter, depth, ..
        } => {
            let (dia, depth) = (mm(diameter)?, mm(depth)?);
            for at in shelf_pin_points(parameters, &machining.kind, f.size, &name)? {
                drill(&mut out, at, dia, Some(depth))?;
            }
        }
        MachiningKind::Hardware {
            pattern,
            at,
            quarter_turns,
        } => {
            let hw = read_hardware_pattern(document, *pattern)?;
            let material = read_panel(
                document,
                panel_feature(document, target).expect("checked panel"),
            )
            .and_then(|p| resolve_panel(document, parameters, &p))
            .map(|p| p.stock_spec.material)?;
            let allowance = hw.allowances.iter().find(|a| a.material == material);
            let (dd, dz) = match allowance {
                Some(a) => (mm(&a.diameter)?, mm(&a.depth)?),
                None => (0., 0.),
            };
            let base = [mm(&at[0])?, mm(&at[1])?];
            for h in &hw.holes {
                let mut p = [mm(&h.at[0])?, mm(&h.at[1])?];
                for _ in 0..*quarter_turns {
                    p = [-p[1], p[0]];
                }
                let depth = h
                    .depth
                    .as_ref()
                    .map(|d| mm(d).map(|d| d + dz))
                    .transpose()?;
                drill(
                    &mut out,
                    [base[0] + p[0], base[1] + p[1]],
                    mm(&h.diameter)? + dd,
                    depth,
                )?;
            }
        }
    }
    Ok(out)
}

/// Resolve an operation on every target and refuse holes that overlap a hole
/// of another operation on the same face. `existing` is the operation being
/// edited, which is not compared with itself.
pub fn check_machining(
    document: &Document,
    parameters: &EvaluatedParameters,
    machining: &MachiningDefinition,
    existing: Option<EntityId>,
) -> Result<Vec<ResolvedMachining>, DocumentError> {
    let mut out = vec![];
    for &t in &machining.targets {
        let r = resolve_machining(document, parameters, machining, t)?;
        for other in machinings_of(document, t) {
            if Some(other) == existing {
                continue;
            }
            let op = read_machining(document, other)?;
            let Ok(o) = resolve_machining(document, parameters, &op, t) else {
                continue;
            };
            for h in &r.holes {
                if let Some(clash) = o.holes.iter().find(|g| holes_overlap(h, g)) {
                    let panel = document.get(t).map_or(String::new(), |e| e.name.clone());
                    let op_name = document
                        .get(other)
                        .map_or(String::new(), |e| e.name.clone());
                    return Err(invalid(format!(
                        "A hole at ({:.1}, {:.1}) on {panel} overlaps a hole of {op_name} at ({:.1}, {:.1})",
                        h.at[0], h.at[1], clash.at[0], clash.at[1]
                    )));
                }
            }
        }
        out.push(r);
    }
    Ok(out)
}
fn holes_overlap(a: &ResolvedHole, b: &ResolvedHole) -> bool {
    if dot3(a.axis, b.axis) < 1. - 1e-9 {
        return false;
    }
    let d = [
        b.center[0] - a.center[0],
        b.center[1] - a.center[1],
        b.center[2] - a.center[2],
    ];
    let along = dot3(d, a.axis);
    if along.abs() > 1e-6 {
        return false;
    }
    dot3(d, d).sqrt() < (a.diameter_mm + b.diameter_mm) / 2. - TOL
}

/// Shelf support levels of a carcass: offsets from the interior bottom to a
/// shelf underside resting on pins in holes that both sides share. A pin
/// is taken to be the hole diameter, so a shelf rests half a diameter above
/// the hole center. Only levels with room for a shelf are listed.
pub fn shelf_support_offsets(
    document: &Document,
    parameters: &EvaluatedParameters,
    carcass: EntityId,
) -> Result<Vec<f64>, DocumentError> {
    let definition = read_carcass(document, carcass)?;
    let resolved = resolve_carcass(document, parameters, &definition)?;
    let t = stock_thickness(&read_panel_stock(document, definition.stock)?);
    let bottom = resolved.origin[2] + t;
    let interior = resolved.size_mm[1] - 2. * t;
    let mut sides = [vec![], vec![]];
    for (role, _, part) in carcass_panels(document, carcass)? {
        let i = match role {
            PanelRole::LeftSide => 0,
            PanelRole::RightSide => 1,
            _ => continue,
        };
        for op in machinings_of(document, part) {
            let m = read_machining(document, op)?;
            if !matches!(m.kind, MachiningKind::ShelfPins { .. }) {
                continue;
            }
            for h in resolve_machining(document, parameters, &m, part)?.holes {
                sides[i].push(h.center[2] + h.diameter_mm / 2. - bottom);
            }
        }
    }
    let mut levels: Vec<f64> = sides[0]
        .iter()
        .copied()
        .filter(|z| sides[1].iter().any(|w| (w - z).abs() <= 1e-6))
        .filter(|z| *z >= -1e-6 && z + t <= interior + 1e-6)
        .collect();
    levels.sort_by(f64::total_cmp);
    levels.dedup_by(|a, b| (*a - *b).abs() <= 1e-6);
    Ok(levels)
}

#[derive(Clone, Debug)]
pub struct MachiningPrepared {
    pub operation: EntityId,
    pub model: ModelPrepared,
}
fn check_name(name: &str, what: &str) -> Result<(), CommandError> {
    if name.trim().is_empty() || name.len() > 1000 {
        return Err(error(format!("{what} name must contain 1–1000 bytes")));
    }
    Ok(())
}
/// Create a reusable hardware hole pattern.
pub fn prepare_hardware(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    pattern: &HardwarePatternDefinition,
) -> Result<(EntityId, Prepared), CommandError> {
    revision(session, expected_revision)?;
    check_name(name, "Hardware pattern")?;
    let entity = Entity::new(name, pattern.entity_data()?);
    let id = entity.id;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Create hardware pattern".into(),
        edits: vec![Mutation::Insert(entity)],
    })?;
    Ok((id, prepared))
}
/// Edit a hardware pattern; every placement rebuilds.
pub fn prepare_hardware_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    pattern: &HardwarePatternDefinition,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    read_hardware_pattern(session.document(), id)?;
    let mut entity = session
        .document()
        .get(id)
        .cloned()
        .ok_or_else(|| error("Hardware pattern is missing"))?;
    entity.data = pattern.entity_data()?;
    let candidate = session
        .document()
        .edited(&[Mutation::Replace(entity.clone())])?;
    for e in machinings(&candidate) {
        let m = read_machining(&candidate, e.id)?;
        if matches!(&m.kind, MachiningKind::Hardware { pattern, .. } if *pattern == id) {
            for t in &m.targets {
                resolve_machining(&candidate, session.parameters(), &m, *t)?;
            }
        }
    }
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit hardware pattern".into(),
        edits: vec![Mutation::Replace(entity)],
    })
}
/// Create a machining operation and every target panel's cut in one
/// transaction.
pub fn prepare_machining(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    machining: &MachiningDefinition,
) -> Result<MachiningPrepared, CommandError> {
    revision(session, expected_revision)?;
    check_name(name, "Machining")?;
    let document = session.document();
    machining.validate()?;
    check_machining(document, session.parameters(), machining, None)?;
    let entity = Entity::new(name, machining.entity_data(document)?);
    let id = entity.id;
    let mut edits = vec![Mutation::Insert(entity)];
    let mut candidate = document.edited(&edits)?;
    let mut outputs = vec![];
    for &part in &machining.targets {
        let label = format!(
            "{} {}",
            document.get(part).map_or(String::new(), |e| e.name.clone()),
            machining.label()
        );
        outputs.push(super::chain::append_cut(
            &mut candidate,
            &mut edits,
            part,
            label,
            |input| {
                let cut = MachiningCutDefinition {
                    operation: id,
                    input,
                    output: BodyOutput { body: part },
                };
                Ok((cut.definition()?, cut.inputs()))
            },
        )?);
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Create {}", machining.label()),
        edits,
    })?;
    Ok(MachiningPrepared {
        operation: id,
        model: ModelPrepared { prepared, outputs },
    })
}
/// Change an operation's kind, sizes or face. Its targets stay; every
/// target's cut rebuilds.
pub fn prepare_machining_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    machining: &MachiningDefinition,
) -> Result<MachiningPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    if read_machining(document, id)?.targets != machining.targets {
        return Err(error(
            "An operation keeps its target panels; delete it and create another to change them",
        ));
    }
    machining.validate()?;
    check_machining(document, session.parameters(), machining, Some(id))?;
    let mut entity = document
        .get(id)
        .cloned()
        .ok_or_else(|| error("Machining is missing"))?;
    entity.data = machining.entity_data(document)?;
    let outputs = machining_cuts(document)
        .filter_map(|e| {
            read_machining_cut(document, e.id)
                .ok()
                .filter(|c| c.operation == id)
                .map(|c| CreatedBody {
                    feature: e.id,
                    body: c.output.body,
                })
        })
        .collect();
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Edit {}", machining.label()),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(MachiningPrepared {
        operation: id,
        model: ModelPrepared { prepared, outputs },
    })
}
/// Delete an operation and its cuts, restoring the panels.
pub fn prepare_machining_delete(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    read_machining(document, id)?;
    let cuts: Vec<_> = machining_cuts(document)
        .filter(|e| read_machining_cut(document, e.id).is_ok_and(|c| c.operation == id))
        .map(|e| e.id)
        .collect();
    let mut candidate = document.clone();
    let mut edits = vec![];
    super::chain::remove_cuts(&mut candidate, &mut edits, &cuts, "machining")?;
    edits.push(Mutation::Remove(id));
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Delete machining".into(),
        edits,
    })
}
