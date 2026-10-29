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
    pub center: [f64; 3],
    pub axis: [f64; 3],
    pub diameter_mm: f64,
    pub depth_mm: f64,
    pub through: bool,
}
/// A groove, rabbet or pocket: length and width on the face, and depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedRecess {
    pub kind: &'static str,
    pub size_mm: [f64; 2],
    pub depth_mm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMachining {
    pub prisms: Vec<CutPrism>,
    pub holes: Vec<ResolvedHole>,
    pub recesses: Vec<ResolvedRecess>,
}

/// A face plane: a world point at face coordinates (0, 0), the two in-face
/// axes, the inward direction and the extents of the face and into it.
struct Face {
    origin: [f64; 3],
    a: [f64; 3],
    b: [f64; 3],
    inward: [f64; 3],
    size: [f64; 2],
    depth: f64,
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
        MachiningFace::Bottom => Face {
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
    fn point(&self, p: [f64; 2]) -> [f64; 3] {
        add(add(self.origin, self.a, p[0]), self.b, p[1])
    }
}
fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn rect(a: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
    vec![[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]]
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
    let f = face(&b, machining.face);
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
        prisms: vec![],
        holes: vec![],
        recesses: vec![],
    };
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
        out.holes.push(ResolvedHole {
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
            out.prisms.push(f.prism(polygon, None, d));
            let length = if *along == PanelAxis::U { fl } else { fw };
            out.recesses.push(ResolvedRecess {
                kind: "groove",
                size_mm: [length, w],
                depth_mm: d,
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
            out.prisms.push(f.prism(polygon, None, d));
            out.recesses.push(ResolvedRecess {
                kind: "rabbet",
                size_mm: [length, w],
                depth_mm: d,
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
            out.prisms.push(f.prism(rect(lo, hi), None, d));
            out.recesses.push(ResolvedRecess {
                kind: "pocket",
                size_mm: s,
                depth_mm: d,
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
    for t in &machining.targets {
        resolve_machining(document, session.parameters(), machining, *t)?;
    }
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
    for t in &machining.targets {
        resolve_machining(document, session.parameters(), machining, *t)?;
    }
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
