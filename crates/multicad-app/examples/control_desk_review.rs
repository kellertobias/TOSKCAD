//! The PCB-driven control desk in one native editable project: a KiCad
//! board (hand-written fixture) with generated STEP models and one
//! deliberately missing model drives the standoff pattern and the component
//! openings of a bent aluminium panel; birch plywood cheeks carry the panel
//! on screws; a printed PETG guide receives the board's mounting holes and
//! component outlines through the shared transfer engine; captive inserts
//! sit on the back flange. A board reimport moves, removes and adds
//! components, and the board-drive check reports every derived panel feature
//! that no longer matches its source until it is synchronized. Drawings,
//! schedules, the developed DXF and scoped STEP/3MF sets come from the same
//! document. The app test `tests/control_desk.rs` builds the same project and
//! checks it.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    exchange::*,
    inspection::GeometryReference,
    materials::*,
    model::{
        self, AlignmentInput, AssemblyRequest, BendDirection, BodyInput, BodyOutput,
        ComponentPlacementRequest, CreatedBody, Cutout, CutoutFace, CutoutShape, Flange,
        FlangeKind, HardwarePreset, HardwareSide, MountingPattern, NumericTransform,
        OccurrencePath, PanelPlane, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, Relief,
        SheetBase, SheetDefinition, SheetRequest, Thread, TransferRequest, TransferSelection,
        TransferSource, TransferTarget, TransferTargetParameters, angle, imports, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{
    Compiler, Model,
    export::{Exporter, sheet_dxf::SheetDxfLayers},
    imports::{KicadOptions, read_kicad_file, read_step_file},
    schedules::{BomOptions, Schedule, bend_schedule, bom_schedule},
};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

pub type Error = Box<dyn std::error::Error>;
/// Revision A of the board, as written by hand in KiCad 8 syntax.
pub const BOARD_A: &str = include_str!("../tests/fixtures/control_desk/control-panel.kicad_pcb");
/// Revision B: SW1 and H4 move, D1 is removed and SW3 is added.
pub const BOARD_B: &str =
    include_str!("../tests/fixtures/control_desk/control-panel-revB.kicad_pcb");
pub const BOARD_FILE: &str = "control-panel.kicad_pcb";
pub const FABRICATED: &str = "fabricated";
pub const PRINTED: &str = "printed";
pub const PURCHASED: &str = "purchased";
/// Footprints imported from the board; the mounting holes stay holes.
pub const COMPONENTS_A: [&str; 5] = ["DS1", "SW1", "SW2", "ENC1", "D1"];
pub const COMPONENTS_B: [&str; 5] = ["DS1", "SW1", "SW2", "SW3", "ENC1"];
/// The model the board names for ENC1 is never generated.
pub const MISSING_MODEL: &str = "rotary_encoder_ec11.step";
/// Board top below the panel's base face: the standoff length.
pub const STANDOFF_MM: f64 = 10.;
pub const BOARD_T: f64 = 1.6;
/// The printed guide frame fills the gap between the board top and the
/// panel's base face: z −10 … 0. Its window is 140 × 80 mm.
pub const GUIDE_Z: [f64; 2] = [-STANDOFF_MM, 0.];
pub const GUIDE_BAND: f64 = 10.;
pub const CHEEK_T: f64 = 12.;
pub const FLANGE_INSET: f64 = 14.;

pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}

/// Native preflight, then the atomic commit.
pub fn checked(session: &mut Session, prepared: Prepared) -> Result<(), Error> {
    let model = Compiler::default().compile(prepared.document(), session.revision())?;
    if !model
        .snapshot()
        .references
        .for_model(prepared.document())?
        .all_resolved()
    {
        return Err("Native references are unresolved".into());
    }
    session.commit(prepared)?;
    Ok(())
}
fn parameter(name: &str, expression: &str, description: &str) -> Entity {
    let mut e = Entity::new(
        name,
        EntityData::Parameter {
            expression: expression.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    e.description = description.into();
    e
}
fn bound(document: &Document, text: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(document, text, "mm", None)?,
    })
}
/// `value` rounded to a micrometer, without trailing zeros.
fn number(value: f64) -> String {
    let r = (value * 1000.).round() / 1000.;
    let r = if r == 0. { 0. } else { r };
    format!("{r}")
}
/// `=$name ± value mm`.
fn offset(name: &str, value: f64) -> String {
    if value < 0. {
        format!("=${name} - {} mm", number(-value))
    } else {
        format!("=${name} + {} mm", number(value))
    }
}

// ---------------------------------------------------------------------------
// Source files: the board text and its generated STEP models.

/// A STEP file holding the union of axis-aligned boxes (size, minimum corner).
fn boxes(path: &Path, parts: &[([f64; 3], [f64; 3])]) -> Result<(), Error> {
    let policy = GeometryPolicy::default();
    let mut all: Option<Solid> = None;
    for (size, at) in parts {
        let b = Solid::box_solid(*size, policy)?.transformed(RigidTransform {
            translation_mm: *at,
            ..Default::default()
        })?;
        all = Some(match all {
            None => b,
            Some(a) => a
                .boolean(&b, BooleanOperation::Union, policy)?
                .ok_or("Empty model union")?,
        });
    }
    all.ok_or("No model boxes")?.write_step(path)?;
    Ok(())
}
/// Write the board revision and every STEP model except the encoder's into
/// `dir` (`models/` beside the board, as `${KIPRJMOD}` resolves it).
pub fn write_sources(dir: &Path, board: &str) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(dir)?;
    let dir = &dir.canonicalize()?;
    let models = dir.join("models");
    std::fs::create_dir_all(&models)?;
    // OLED module: 40 × 26 × 4 mm.
    boxes(
        &models.join("oled_module.step"),
        &[([40., 26., 4.], [-20., -13., 0.])],
    )?;
    // 12 mm push button: a 12 × 12 × 3.5 body and an 8 × 8 cap to 13 mm.
    boxes(
        &models.join("push_button_12mm.step"),
        &[
            ([12., 12., 3.5], [-6., -6., 0.]),
            ([8., 8., 9.5], [-4., -4., 3.5]),
        ],
    )?;
    // 5 mm LED with its collar: 5 × 5 × 8.6 mm.
    boxes(
        &models.join("led_5mm.step"),
        &[([5., 5., 8.6], [-2.5, -2.5, 0.])],
    )?;
    let path = dir.join(BOARD_FILE);
    std::fs::write(&path, board)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Board-driven panel features.

/// How a component's footprint becomes a panel opening.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Opening {
    /// A round hole of this diameter on the footprint origin.
    Circle { diameter: f64 },
    /// The rotated courtyard grown by `margin` per side (negative shrinks it
    /// to a viewing window).
    Window { margin: f64 },
}
/// The panel rule of each board reference, in cutout order. New rules are
/// appended so existing cutout ids stay put. Components without a rule are
/// reported, never silently given an opening.
pub const RULES: [(&str, Opening); 6] = [
    ("DS1", Opening::Window { margin: -6. }),
    ("SW1", Opening::Circle { diameter: 12.5 }),
    ("SW2", Opening::Circle { diameter: 12.5 }),
    ("ENC1", Opening::Circle { diameter: 7.5 }),
    ("D1", Opening::Circle { diameter: 5.2 }),
    ("SW3", Opening::Circle { diameter: 12.5 }),
];

/// One derived panel feature in board coordinates (Y up, KiCad origin).
#[derive(Clone, Debug, PartialEq)]
pub enum Derived {
    Standoff {
        hole: String,
        center: [f64; 2],
    },
    Opening {
        reference: String,
        rule: Opening,
        center: [f64; 2],
        size: [f64; 2],
        missing: bool,
    },
}
impl Derived {
    pub fn source(&self) -> &str {
        match self {
            Self::Standoff { hole, .. } => hole,
            Self::Opening { reference, .. } => reference,
        }
    }
}
/// Every board-driven feature in sheet order: the standoff points (one per
/// mounting hole, by reference) and the openings in rule order. Components
/// gone from the board keep their last footprint, flagged `missing`.
pub fn board_drive(document: &Document, board: EntityId) -> Result<Vec<Derived>, Error> {
    let record = imports::read_board(document, board)?;
    let mut holes = record.holes.clone();
    holes.sort_by(|a, b| a.footprint.cmp(&b.footprint));
    let mut out: Vec<_> = holes
        .into_iter()
        .map(|h| Derived::Standoff {
            hole: h.footprint,
            center: h.center,
        })
        .collect();
    let components: Vec<_> = imports::components(document)
        .filter_map(|e| imports::read_component(document, e.id).ok())
        .filter(|c| c.board == board)
        .collect();
    for (reference, rule) in RULES {
        let Some(c) = components
            .iter()
            .find(|c| c.footprint.reference == reference)
        else {
            continue;
        };
        let fp = &c.footprint;
        let (s, co) = fp.rotation_deg.to_radians().sin_cos();
        let [lo, hi] = fp.envelope;
        let corners = [
            [lo[0], lo[1]],
            [hi[0], lo[1]],
            [hi[0], hi[1]],
            [lo[0], hi[1]],
        ]
        .map(|p| [p[0] * co - p[1] * s, p[0] * s + p[1] * co]);
        let min = |k: usize| corners.iter().map(|p| p[k]).fold(f64::MAX, f64::min);
        let max = |k: usize| corners.iter().map(|p| p[k]).fold(f64::MIN, f64::max);
        let (center, size) = match rule {
            Opening::Circle { diameter } => (fp.at, [diameter, diameter]),
            Opening::Window { margin } => (
                [
                    fp.at[0] + (min(0) + max(0)) / 2.,
                    fp.at[1] + (min(1) + max(1)) / 2.,
                ],
                [max(0) - min(0) + 2. * margin, max(1) - min(1) + 2. * margin],
            ),
        };
        out.push(Derived::Opening {
            reference: reference.into(),
            rule,
            center,
            size,
            missing: c.missing,
        });
    }
    Ok(out)
}
/// The standoff points and cutouts of `drive` as parameter expressions in
/// the panel's base frame: u = `$board_x` + x, v = `$board_y` + y.
fn panel_features(
    document: &Document,
    drive: &[Derived],
) -> Result<(Vec<[Dimension; 2]>, Vec<Cutout>), Error> {
    let at = |p: [f64; 2]| -> Result<[Dimension; 2], Error> {
        Ok([
            bound(document, &offset("board_x", p[0]))?,
            bound(document, &offset("board_y", p[1]))?,
        ])
    };
    let mut points = vec![];
    let mut cutouts = vec![];
    for d in drive {
        match d {
            Derived::Standoff { center, .. } => points.push(at(*center)?),
            Derived::Opening {
                rule, center, size, ..
            } => cutouts.push(Cutout {
                face: CutoutFace::Base,
                shape: match rule {
                    Opening::Circle { diameter } => CutoutShape::Circle {
                        center: at(*center)?,
                        diameter: length(*diameter),
                    },
                    Opening::Window { .. } => CutoutShape::Rectangle {
                        center: at(*center)?,
                        size: size.map(length),
                    },
                },
            }),
        }
    }
    Ok((points, cutouts))
}
/// The sheet with its board-driven standoff points and cutouts replaced.
fn synchronized(
    document: &Document,
    sheet: EntityId,
    board: EntityId,
) -> Result<SheetDefinition, Error> {
    let mut definition = model::read_sheet(document, sheet)?;
    let (points, cutouts) = panel_features(document, &board_drive(document, board)?)?;
    definition.fasteners[STANDOFFS].points = points;
    definition.cutouts = cutouts;
    Ok(definition)
}
/// Pattern indices on the panel.
pub const STANDOFFS: usize = 0;
pub const CHEEK_SCREWS: usize = 1;
pub const INSERTS: usize = 2;

/// One finding of the board-drive check. `kind` is stable; the message
/// names the source, the feature and the measured difference.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub kind: &'static str,
    pub subject: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// The scenario.

pub struct Component {
    pub reference: String,
    pub body: CreatedBody,
    pub member: EntityId,
}
pub struct Scenario {
    pub session: Session,
    pub desk_width: EntityId,
    pub panel_depth: EntityId,
    pub board_x: EntityId,
    pub board_y: EntityId,
    pub aluminium: EntityId,
    pub stock: EntityId,
    pub birch: EntityId,
    pub petg: EntityId,
    pub sheet: SheetRequest,
    pub cheeks: [PrimitiveRequest; 2],
    pub guide: PrimitiveRequest,
    pub board: CreatedBody,
    pub board_member: EntityId,
    pub guide_member: EntityId,
    pub components: Vec<Component>,
    pub assembly: EntityId,
    pub stack: EntityId,
    /// (pattern, item, hardware Part) after the last fastener change.
    pub hardware: Vec<(u32, model::HardwareItem, EntityId)>,
    pub missing_models: Vec<(String, String)>,
    pub unsupported: Vec<String>,
    pub book: EntityId,
    pub pages: [EntityId; 4],
    pub width_dimension: EntityId,
    pub drawing_set: EntityId,
    pub step_set: EntityId,
    pub print_set: EntityId,
}
impl Scenario {
    pub fn component(&self, reference: &str) -> &Component {
        self.components
            .iter()
            .find(|c| c.reference == reference)
            .unwrap_or_else(|| panic!("no component {reference}"))
    }
    pub fn hardware_part(&self, pattern: usize, item: model::HardwareItem) -> EntityId {
        self.hardware
            .iter()
            .find(|(p, i, _)| *p as usize == pattern && *i == item)
            .map(|(.., part)| *part)
            .expect("hardware Part")
    }
    fn path(&self, member: EntityId) -> OccurrencePath {
        OccurrencePath {
            root: self.stack,
            members: vec![member],
        }
    }

    /// Every finding of the board-drive check against the current document:
    /// stale or extra derived panel features, components gone from the
    /// board, components without a rule, missing STEP models, and
    /// components that cross the printed guide without an opening.
    pub fn findings(&self) -> Result<Vec<Finding>, Error> {
        let document = self.session.document();
        let parameters = self.session.parameters();
        let mut out = vec![];
        let drive = board_drive(document, self.board.feature)?;
        let current = model::read_sheet(document, self.sheet.feature)?;
        let wanted = synchronized(document, self.sheet.feature, self.board.feature)?;
        let have = model::resolve_sheet(document, parameters, &current)?;
        let want = model::resolve_sheet(document, parameters, &wanted)?;
        let center = |s: &model::ResolvedShape| match *s {
            model::ResolvedShape::Circle { center, .. }
            | model::ResolvedShape::Rectangle { center, .. } => center,
        };
        let openings = |r: &model::ResolvedSheet| -> Vec<_> {
            r.cutouts
                .iter()
                .filter(|c| matches!(c.source, model::CutoutSource::Cutout { .. }))
                .map(|c| c.shape)
                .collect()
        };
        let standoffs = drive
            .iter()
            .filter(|d| matches!(d, Derived::Standoff { .. }));
        let opening_sources = drive
            .iter()
            .filter(|d| matches!(d, Derived::Opening { .. }));
        let compare = |out: &mut Vec<Finding>,
                       id: String,
                       source: &str,
                       have: Option<([f64; 2], String)>,
                       want: Option<([f64; 2], String)>| {
            match (have, want) {
                (Some((h, hs)), Some((w, ws))) => {
                    let moved = (h[0] - w[0]).hypot(h[1] - w[1]);
                    let (hs, ws) = if hs.is_empty() {
                        (hs, ws)
                    } else {
                        (format!(" {hs}"), format!(" {ws}"))
                    };
                    if moved > 1e-6 || hs != ws {
                        out.push(Finding {
                            kind: "panel_stale",
                            subject: id.clone(),
                            message: format!(
                                "{id} ({source}) is at ({}, {}){hs}; the board puts it at \
                                 ({}, {}){ws}, {} mm away. Synchronize the panel.",
                                number(h[0]),
                                number(h[1]),
                                number(w[0]),
                                number(w[1]),
                                number(moved)
                            ),
                        });
                    }
                }
                (None, Some((w, ws))) => out.push(Finding {
                    kind: "panel_stale",
                    subject: id.clone(),
                    message: format!(
                        "{id} ({source}) is not on the panel; the board needs it at ({}, {}) \
                         {ws}. Synchronize the panel.",
                        number(w[0]),
                        number(w[1])
                    ),
                }),
                (Some((h, _)), None) => out.push(Finding {
                    kind: "panel_stale",
                    subject: id.clone(),
                    message: format!(
                        "{id} at ({}, {}) has no source on the board any more. Synchronize \
                         the panel.",
                        number(h[0]),
                        number(h[1])
                    ),
                }),
                (None, None) => {}
            }
        };
        let (hp, wp) = (
            &have.patterns[STANDOFFS].points,
            &want.patterns[STANDOFFS].points,
        );
        let sources: Vec<_> = standoffs.map(|d| d.source().to_owned()).collect();
        for i in 0..hp.len().max(wp.len()) {
            let source = sources.get(i).map_or("", String::as_str);
            compare(
                &mut out,
                format!("fastener.{STANDOFFS}.{i}"),
                source,
                hp.get(i).map(|p| (*p, String::new())),
                wp.get(i).map(|p| (*p, String::new())),
            );
        }
        let (ho, wo) = (openings(&have), openings(&want));
        let size = |s: &model::ResolvedShape| match *s {
            model::ResolvedShape::Circle { diameter_mm, .. } => format!("D{}", number(diameter_mm)),
            model::ResolvedShape::Rectangle { size_mm, .. } => {
                format!("{} × {}", number(size_mm[0]), number(size_mm[1]))
            }
        };
        let opening_refs: Vec<_> = opening_sources.map(|d| d.source().to_owned()).collect();
        for i in 0..ho.len().max(wo.len()) {
            let source = opening_refs.get(i).map_or("", String::as_str);
            compare(
                &mut out,
                format!("cutout.{i}"),
                source,
                ho.get(i).map(|s| (center(s), size(s))),
                wo.get(i).map(|s| (center(s), size(s))),
            );
        }
        // Component-level findings.
        let components: Vec<_> = imports::components(document)
            .filter_map(|e| imports::read_component(document, e.id).ok())
            .filter(|c| c.board == self.board.feature)
            .collect();
        for c in &components {
            let r = &c.footprint.reference;
            if c.missing {
                let feature = opening_refs
                    .iter()
                    .position(|o| o == r)
                    .map_or("no panel feature".into(), |i| format!("cutout.{i}"));
                out.push(Finding {
                    kind: "component_missing",
                    subject: r.clone(),
                    message: format!(
                        "{r} is not on the latest board. Its Part keeps its last placement and \
                         {feature} is retained for review; remove both deliberately."
                    ),
                });
            }
            if !RULES.iter().any(|(rule, _)| rule == r) {
                out.push(Finding {
                    kind: "panel_unmapped",
                    subject: r.clone(),
                    message: format!("{r} has no panel rule; no opening was derived for it."),
                });
            }
            if c.model.is_none() {
                let path = c
                    .footprint
                    .models
                    .first()
                    .map_or("no model reference".into(), |m| m.path.clone());
                out.push(Finding {
                    kind: "model_missing",
                    subject: r.clone(),
                    message: format!(
                        "{r} has no linked STEP model ({path}); it is a {} mm courtyard envelope. \
                         Its panel opening comes from the footprint, so the real part's height \
                         and shape are not checked. Link a model to repair.",
                        number(c.height_mm)
                    ),
                });
            }
        }
        // Components crossing the printed guide without an opening.
        let model = Compiler::default().compile(document, self.session.revision())?;
        let policy = GeometryPolicy::default();
        let guide = model
            .solid(self.guide_member)
            .ok_or("The printed guide is not evaluated")?;
        for c in &self.components {
            let Some(solid) = model.solid(c.member) else {
                continue;
            };
            if let Some(common) = guide.boolean(solid, BooleanOperation::Intersect, policy)? {
                let volume = common.volume_mm3()?;
                if volume > 1e-6 {
                    out.push(Finding {
                        kind: "guide_interference",
                        subject: c.reference.clone(),
                        message: format!(
                            "{} reaches into the printed guide frame by {} mm³. Move the \
                             component or open the frame.",
                            c.reference,
                            number(volume)
                        ),
                    });
                }
            }
        }
        Ok(out)
    }
}

/// A placed member's top planar face at local `z`, captured on `producer`.
fn top(
    model: &Model,
    producer: EntityId,
    owner: EntityId,
    z: f64,
) -> Result<AlignmentInput, Error> {
    let mut snapshot = model.snapshot().shapes[&producer].topology.clone();
    snapshot.owner = owner;
    let selected = snapshot
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && matches!(e.geometry, Some(AnalyticGeometry::Plane { frame })
                if frame.normal()[2] > 0.999 && (frame.origin_mm[2] - z).abs() < 1e-7)
        })
        .ok_or_else(|| format!("No top face at z = {z}"))?;
    Ok(AlignmentInput {
        source: owner,
        intent: GeometryIntent::capture(&snapshot, selected.key)?,
    })
}
/// The Part's current head producer.
fn head(model: &Model, part: EntityId) -> EntityId {
    let s = model.snapshot();
    *s.bodies
        .get(&part)
        .or_else(|| s.body_sources.get(&part))
        .expect("evaluated Part")
}
/// A planar face of a Part whose normal matches `normal` and whose plane
/// passes through coordinate `at` along it.
pub fn face(
    model: &Model,
    part: EntityId,
    normal: [f64; 3],
    at: f64,
) -> Result<GeometryReference, Error> {
    let feature = head(model, part);
    let mut topology = model.snapshot().shapes[&feature].topology.clone();
    topology.owner = part;
    let element = topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && matches!(e.geometry, Some(AnalyticGeometry::Plane { frame })
                    if frame.normal().iter().zip(normal).all(|(a, b)| (a - b).abs() < 1e-9)
                        && (frame.origin_mm.iter().zip(normal).map(|(o, n)| o * n).sum::<f64>() - at).abs() < 1e-6)
        })
        .ok_or_else(|| format!("Planar face {normal:?} at {at} is missing"))?;
    Ok(GeometryReference {
        source: part,
        intent: GeometryIntent::capture(&topology, element.key)?,
    })
}

fn guide_target(s: &Scenario, model: &Model, request: &mut TransferRequest) -> Result<(), Error> {
    let producer = head(model, s.guide.body);
    let face = top(model, producer, producer, GUIDE_Z[1])?;
    request.targets.push(TransferTarget::new(
        BodyInput {
            body: s.guide.body,
            producer,
        },
        s.path(s.guide_member),
        face.intent.id,
        TransferTargetParameters::default(),
    ));
    request.references.push(face);
    Ok(())
}

fn material(name: &str, density: f64, color: [u8; 4]) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(density),
            color_rgba: color,
            section_hatch: None,
        },
    )
}
fn tag(session: &mut Session, parts: &[EntityId], tag: &str) -> Result<(), Error> {
    let prepared = prepare_tags(
        session,
        session.revision(),
        actor(),
        parts,
        &TagEdit::Add([tag.to_string()].into()),
    )?;
    session.commit(prepared)?;
    Ok(())
}
fn assign(session: &mut Session, parts: &[EntityId], material: EntityId) -> Result<(), Error> {
    let prepared = prepare_assignment(
        session,
        session.revision(),
        actor(),
        parts,
        &Assignment {
            material: Some(material),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )?;
    session.commit(prepared)?;
    Ok(())
}
fn flange(edge: u32, leg: &str, document: &Document) -> Result<Flange, Error> {
    Ok(Flange {
        edge,
        kind: FlangeKind::Bend { angle: angle(90.) },
        direction: BendDirection::Down,
        length: bound(document, leg)?,
        inside_radius: None,
        start_inset: Some(length(FLANGE_INSET)),
        end_inset: Some(length(FLANGE_INSET)),
        relief: Some(Relief {
            width: length(2.),
            depth: length(4.),
        }),
    })
}
/// The mounting patterns: standoffs at the board holes (from the board),
/// cheek screws near the side edges and captive inserts on the back flange.
fn patterns(
    document: &Document,
    standoffs: Vec<[Dimension; 2]>,
) -> Result<Vec<MountingPattern>, Error> {
    let at = |u: &str, v: &str| -> Result<[Dimension; 2], Error> {
        let axis = |t: &str| -> Result<Dimension, Error> {
            if t.starts_with('=') {
                bound(document, t)
            } else {
                Ok(length(t.parse()?))
            }
        };
        Ok([axis(u)?, axis(v)?])
    };
    Ok(vec![
        MountingPattern {
            preset: HardwarePreset::Standoff {
                thread: Thread::M3,
                length: length(STANDOFF_MM),
                across_flats: length(5.5),
            },
            face: CutoutFace::Base,
            side: HardwareSide::Base,
            points: standoffs,
        },
        MountingPattern {
            preset: HardwarePreset::Screw {
                thread: Thread::M4,
                length: length(20.),
                washers: true,
                nut: false,
            },
            face: CutoutFace::Base,
            side: HardwareSide::Material,
            points: vec![
                at("6", "40")?,
                at("6", "=$panel_depth - 40 mm")?,
                at("=$desk_width - 6 mm", "40")?,
                at("=$desk_width - 6 mm", "=$panel_depth - 40 mm")?,
            ],
        },
        MountingPattern {
            preset: HardwarePreset::Insert {
                designation: "Self-clinching nut M3 (example entry)".into(),
                source: "User-entered catalogue values".into(),
                hole_diameter: length(4.2),
                body_diameter: length(6.3),
                height: length(1.5),
                min_sheet: length(1.),
                max_sheet: length(2.3),
            },
            face: CutoutFace::Flange { index: 1 },
            side: HardwareSide::Material,
            points: vec![
                at("30", "15")?,
                at(
                    &format!("=$desk_width - {} mm", 2. * FLANGE_INSET + 30.),
                    "15",
                )?,
            ],
        },
    ])
}

pub fn build(sources: &Path) -> Result<Scenario, Error> {
    let desk_width = parameter("desk_width", "240", "Panel width along X, cheek to cheek");
    let panel_depth = parameter("panel_depth", "160", "Panel depth along Y");
    let mut session = Session::new(Document::new("Control desk")?.edited(&[
        Mutation::Insert(desk_width.clone()),
        Mutation::Insert(panel_depth.clone()),
    ])?)?;
    // The board sits centered under the panel; its KiCad origin (top-left
    // corner, Y up after import) lands at ($board_x, $board_y).
    let board_x = parameter(
        "board_x",
        "=($desk_width - 160 mm) / 2",
        "Board origin along X: centered",
    );
    let board_y = parameter(
        "board_y",
        "=($panel_depth + 100 mm) / 2",
        "Board origin along Y: centered (KiCad Y runs down)",
    );
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Board placement parameters".into(),
        edits: vec![
            Mutation::Insert(board_x.clone()),
            Mutation::Insert(board_y.clone()),
        ],
    })?;
    session.commit(prepared)?;

    // Materials and the 1.5 mm aluminium sheet stock.
    let aluminium = material("EN AW-5754 H22", 2670., [196, 200, 206, 255]);
    let birch = material("Birch plywood", 680., [214, 186, 140, 255]);
    let petg = material("PETG", 1270., [40, 44, 52, 255]);
    let stock = LibraryDefinition::Stock(Stock {
        material: aluminium.id,
        specification: StockKind::Sheet {
            thickness_mm: 1.5,
            length_mm: 1000.,
            width_mm: 500.,
            inside_bend_radius_mm: 1.5,
            k_factor: 0.44,
        },
    })
    .entity(EntityId::new(), "AlMg3 1.5 sheet")?;
    let stock_id = stock.id;
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[aluminium.clone(), birch.clone(), petg.clone(), stock],
    )?;
    session.commit(prepared)?;

    // The board revision A and its components.
    let board_path = write_sources(sources, BOARD_A)?;
    let import = read_kicad_file(
        &board_path,
        &KicadOptions {
            components: Some(COMPONENTS_A.map(String::from).to_vec()),
            ..Default::default()
        },
    )?;
    let prepared = imports::prepare_kicad_import(&session, session.revision(), actor(), &import)?;
    let board = prepared.board.clone();
    let missing_models = prepared.missing_models.clone();
    let unsupported = prepared.unsupported.clone();
    let imported: Vec<_> = prepared
        .components
        .iter()
        .map(|(r, body, _)| (r.clone(), body.clone()))
        .collect();
    checked(&mut session, prepared.model.prepared)?;

    // The panel: a 240 × 160 base on XY with the material above it, front
    // and back flanges bent down, inset clear of the cheeks with reliefs,
    // the board-driven standoffs and openings, cheek screws and inserts.
    let document = session.document().clone();
    let drive = board_drive(&document, board.feature)?;
    let (standoffs, cutouts) = panel_features(&document, &drive)?;
    let sheet = SheetRequest::new(
        "Control panel",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stock_id,
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xy,
                origin: [length(0.), length(0.), length(0.)],
                length: bound(&document, "=$desk_width")?,
                width: bound(&document, "=$panel_depth")?,
            },
            reverse: false,
            inside_radius: None,
            k_factor: None,
            flanges: vec![
                flange(0, "=50 mm", &document)?,
                flange(2, "=30 mm", &document)?,
            ],
            cutouts,
            fasteners: vec![],
        },
    );
    let prepared = model::prepare_sheet(&session, session.revision(), actor(), &sheet)?;
    checked(&mut session, prepared.prepared)?;
    let fasteners = patterns(session.document(), standoffs)?;
    let prepared = model::prepare_sheet_fasteners(
        &session,
        session.revision(),
        actor(),
        sheet.feature,
        &fasteners,
    )?;
    let hardware: Vec<_> = prepared
        .hardware
        .iter()
        .map(|(p, i, o)| (*p, *i, o.body))
        .collect();
    checked(&mut session, prepared.model.prepared)?;
    let hardware_parts: Vec<_> = hardware.iter().map(|(.., p)| *p).collect();
    tag(&mut session, &hardware_parts, PURCHASED)?;
    tag(&mut session, &[sheet.body], FABRICATED)?;

    // Birch plywood cheeks under the side strips of the panel.
    let document = session.document().clone();
    let cheek = |name: &str, x: &str| -> Result<PrimitiveRequest, Error> {
        Ok(PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [
                    length(CHEEK_T),
                    bound(&document, "=$panel_depth - 10 mm")?,
                    length(90.),
                ],
                frame: PrimitiveFrame {
                    origin: [
                        if x == "0" {
                            length(0.)
                        } else {
                            bound(&document, x)?
                        },
                        length(5.),
                        length(-90.),
                    ],
                    ..Default::default()
                },
            },
        ))
    };
    let cheeks = [
        cheek("Left cheek", "0")?,
        cheek("Right cheek", &format!("=$desk_width - {CHEEK_T} mm"))?,
    ];
    for c in &cheeks {
        let prepared = model::prepare_primitive(&session, session.revision(), actor(), c)?;
        checked(&mut session, prepared.prepared)?;
    }
    let cheek_parts = [cheeks[0].body, cheeks[1].body];
    assign(&mut session, &cheek_parts, birch.id)?;
    tag(&mut session, &cheek_parts, FABRICATED)?;

    // The printed guide frame between the board and the panel: a PETG box
    // with a window cut by a Boolean, around every component on the board.
    let document = session.document().clone();
    let guide = PrimitiveRequest::new(
        "Printed guide frame",
        PrimitiveParameters::Box {
            size: [length(160.), length(100.), length(GUIDE_Z[1] - GUIDE_Z[0])],
            frame: PrimitiveFrame {
                origin: [
                    bound(&document, "=$board_x")?,
                    bound(&document, "=$board_y - 100 mm")?,
                    length(GUIDE_Z[0]),
                ],
                ..Default::default()
            },
        },
    );
    let window = PrimitiveRequest::new(
        "Guide window",
        PrimitiveParameters::Box {
            size: [
                length(160. - 2. * GUIDE_BAND),
                length(100. - 2. * GUIDE_BAND),
                length(GUIDE_Z[1] - GUIDE_Z[0] + 2.),
            ],
            frame: PrimitiveFrame {
                origin: [
                    bound(&document, &format!("=$board_x + {GUIDE_BAND} mm"))?,
                    bound(&document, &format!("=$board_y - {} mm", 100. - GUIDE_BAND))?,
                    length(GUIDE_Z[0] - 1.),
                ],
                ..Default::default()
            },
        },
    );
    for request in [&guide, &window] {
        let prepared = model::prepare_primitive(&session, session.revision(), actor(), request)?;
        checked(&mut session, prepared.prepared)?;
    }
    let mut request = model::BooleanRequest::new(
        model::Operation::Cut,
        BodyInput {
            body: guide.body,
            producer: guide.feature,
        },
        vec![BodyInput {
            body: window.body,
            producer: window.feature,
        }],
    );
    request.name = "Guide window cut".into();
    let prepared = model::prepare_boolean(&session, session.revision(), actor(), &request)?;
    checked(&mut session, prepared.prepared)?;
    assign(&mut session, &[guide.body], petg.id)?;
    tag(&mut session, &[guide.body], PRINTED)?;

    // The board stack: the board and its components placed under the panel
    // on the standoffs, and the guide, as one assembly placed once.
    let document = session.document().clone();
    let board_place = || -> Result<NumericTransform, Error> {
        Ok(NumericTransform {
            translation: [
                bound(&document, "=$board_x")?,
                bound(&document, "=$board_y")?,
                length(-STANDOFF_MM - BOARD_T),
            ],
            ..Default::default()
        })
    };
    let mut members = vec![];
    let mut parts = vec![board.body];
    let member = ComponentPlacementRequest::new(board.body, board_place()?);
    let prepared = model::prepare_place_component(&session, session.revision(), actor(), &member)?;
    session.commit(prepared.prepared)?;
    let board_member = member.instance;
    members.push(board_member);
    let mut components = vec![];
    for (reference, body) in imported {
        let member = ComponentPlacementRequest::new(body.body, board_place()?);
        let prepared =
            model::prepare_place_component(&session, session.revision(), actor(), &member)?;
        session.commit(prepared.prepared)?;
        members.push(member.instance);
        parts.push(body.body);
        components.push(Component {
            reference,
            body,
            member: member.instance,
        });
    }
    let member = ComponentPlacementRequest::new(guide.body, NumericTransform::default());
    let prepared = model::prepare_place_component(&session, session.revision(), actor(), &member)?;
    session.commit(prepared.prepared)?;
    let guide_member = member.instance;
    members.push(guide_member);
    parts.push(guide.body);
    let assembly = AssemblyRequest::new("Board stack", members, parts);
    let prepared = model::prepare_assembly(&session, session.revision(), actor(), &assembly)?;
    checked(&mut session, prepared.prepared)?;
    let outer = ComponentPlacementRequest::new(assembly.assembly, NumericTransform::default());
    let prepared = model::prepare_place_component(&session, session.revision(), actor(), &outer)?;
    checked(&mut session, prepared.prepared)?;

    let mut s = Scenario {
        session,
        desk_width: desk_width.id,
        panel_depth: panel_depth.id,
        board_x: board_x.id,
        board_y: board_y.id,
        aluminium: aluminium.id,
        stock: stock_id,
        birch: birch.id,
        petg: petg.id,
        sheet,
        cheeks,
        guide,
        board,
        board_member,
        guide_member,
        components,
        assembly: assembly.assembly,
        stack: outer.instance,
        hardware,
        missing_models,
        unsupported,
        book: EntityId::new(),
        pages: [EntityId::new(); 4],
        width_dimension: EntityId::new(),
        drawing_set: EntityId::new(),
        step_set: EntityId::new(),
        print_set: EntityId::new(),
    };

    // The guide frame receives the board's mounting holes through the
    // transfer engine, with clearance around the standoffs.
    let model = Compiler::default().compile(s.session.document(), s.session.revision())?;
    let source = top(&model, s.board.feature, s.board.body, BOARD_T)?;
    let mut request = TransferRequest::new(
        "Guide standoff holes",
        TransferSource {
            component: s.board.body,
            path: s.path(s.board_member),
            selection: TransferSelection::FaceHoles {
                face: source.intent.id,
            },
        },
    );
    request.references.push(source);
    guide_target(&s, &model, &mut request)?;
    request.targets[0].parameters.clearance = length(1.8);
    let prepared = model::prepare_transfer(&s.session, s.session.revision(), actor(), &request)?;
    checked(&mut s.session, prepared.prepared)?;

    drawings(&mut s)?;
    sets(&mut s)?;
    Ok(s)
}

/// Board revision B: reimport with the same Parts, then report.
pub fn reimport(s: &mut Scenario, sources: &Path) -> Result<imports::KicadUpdatePlan, Error> {
    let path = write_sources(sources, BOARD_B)?;
    let import = read_kicad_file(
        &path,
        &KicadOptions {
            components: Some(COMPONENTS_B.map(String::from).to_vec()),
            ..Default::default()
        },
    )?;
    let (plan, prepared) = imports::prepare_kicad_update(
        &s.session,
        s.session.revision(),
        actor(),
        s.board.feature,
        &import,
    )?;
    let added: Vec<_> = prepared.outputs.clone();
    checked(&mut s.session, prepared.prepared)?;
    // New components join the board stack at the board's placement.
    for reference in &plan.added {
        let document = s.session.document();
        let body = added
            .iter()
            .find(|o| {
                imports::read_component(document, o.feature)
                    .is_ok_and(|c| c.footprint.reference == *reference)
            })
            .cloned()
            .ok_or("Added component Part is missing")?;
        let mut request = AssemblyRequest::existing(document, s.assembly)?;
        let member = ComponentPlacementRequest::new(
            body.body,
            NumericTransform {
                translation: [
                    bound(document, "=$board_x")?,
                    bound(document, "=$board_y")?,
                    length(-STANDOFF_MM - BOARD_T),
                ],
                ..Default::default()
            },
        );
        let prepared =
            model::prepare_place_component(&s.session, s.session.revision(), actor(), &member)?;
        s.session.commit(prepared.prepared)?;
        request.members.push(member.instance);
        request.definition_parts.push(body.body);
        let prepared =
            model::prepare_assembly_edit(&s.session, s.session.revision(), actor(), &request)?;
        checked(&mut s.session, prepared.prepared)?;
        s.components.push(Component {
            reference: reference.clone(),
            body,
            member: member.instance,
        });
    }
    Ok(plan)
}

/// Link the STEP model the reimport left unlinked for `reference`.
pub fn link_model(s: &mut Scenario, sources: &Path, reference: &str) -> Result<(), Error> {
    let component = s.component(reference).body.feature;
    let document = s.session.document();
    let c = imports::read_component(document, component)?;
    let name = c
        .footprint
        .models
        .first()
        .and_then(|m| m.path.rsplit('/').next())
        .ok_or("The footprint names no model")?
        .to_owned();
    let file = read_step_file(&sources.canonicalize()?.join("models").join(name))?;
    let prepared = imports::prepare_step_import(&s.session, s.session.revision(), actor(), &file)?;
    let source = prepared.source;
    let loose = prepared.model.outputs.clone();
    checked(&mut s.session, prepared.model.prepared)?;
    let prepared = imports::prepare_component_model(
        &s.session,
        s.session.revision(),
        actor(),
        component,
        Some(source),
    )?;
    checked(&mut s.session, prepared.prepared)?;
    // A STEP import always creates a Part per solid; the component uses the
    // source directly, so the unplaced duplicate at the origin goes.
    let prepared = s.session.prepare(Request {
        expected_revision: s.session.revision(),
        actor: actor(),
        label: "Remove unplaced STEP body".into(),
        edits: loose
            .iter()
            .flat_map(|o| [Mutation::Remove(o.body), Mutation::Remove(o.feature)])
            .collect(),
    })?;
    checked(&mut s.session, prepared)?;
    Ok(())
}

/// Write the board-driven standoff points and openings into the panel in
/// one transaction. Hardware Parts keep their identities.
pub fn prepare_sync(s: &Scenario) -> Result<Prepared, Error> {
    let document = s.session.document();
    let definition = synchronized(document, s.sheet.feature, s.board.feature)?;
    let prepared = model::prepare_sheet_edit(
        &s.session,
        s.session.revision(),
        actor(),
        s.sheet.feature,
        &definition,
    )?
    .prepared;
    Ok(prepared)
}

/// The operator's repair after revision B: link SW3's model and
/// synchronize the panel. D1 stays flagged for review
/// and ENC1's model stays missing.
pub fn repair(s: &mut Scenario, sources: &Path) -> Result<(), Error> {
    link_model(s, sources, "SW3").map_err(|e| format!("Link SW3 model: {e}"))?;
    let prepared = prepare_sync(s)?;
    checked(&mut s.session, prepared).map_err(|e| format!("Panel sync: {e}"))?;
    refresh_tables(s).map_err(|e| format!("Tables: {e}").into())
}

// ---------------------------------------------------------------------------
// Schedules, drawings and export sets.

/// The purchased hardware with pieces measured from the hardware solids.
pub fn hardware_schedule(model: &Model, session: &Session) -> Result<Schedule, Error> {
    let document = session.document();
    let rows = model
        .sheet_hardware(document)?
        .into_iter()
        .map(|h| {
            vec![
                document
                    .get(h.part)
                    .map(|e| e.name.clone())
                    .unwrap_or_default(),
                h.designation,
                h.quantity.to_string(),
                number(h.hole_mm),
                h.source,
                h.findings.join("; "),
            ]
        })
        .collect();
    Ok(Schedule {
        kind: "hardware",
        title: "Purchased hardware".into(),
        revision: session.revision(),
        source: project_file::ContentHash::document(document)?,
        notes: vec![
            "Pieces are counted from the solids of each hardware Part".into(),
            "Nominal ISO dimensions or user-entered catalogue values; no torque, strength or \
             installation data"
                .into(),
        ],
        columns: [
            "Part",
            "Designation",
            "Pieces",
            "Hole",
            "Dimension source",
            "Findings",
        ]
        .map(String::from)
        .to_vec(),
        rows,
    })
}
/// The board-driven features with the findings that concern them, as a
/// schedule. `compact` lists only the finding kinds (for the drawing table);
/// otherwise the full messages (for CSV).
pub fn findings_schedule(s: &Scenario, compact: bool) -> Result<Schedule, Error> {
    let document = s.session.document();
    let findings = s.findings()?;
    let mut used = vec![false; findings.len()];
    let mut concerning = |feature: &str, source: &str| -> String {
        let mut out = vec![];
        for (i, f) in findings.iter().enumerate() {
            if f.subject == feature || f.subject == source {
                used[i] = true;
                out.push(if compact {
                    f.kind.to_string()
                } else {
                    format!("{}: {}", f.kind, f.message)
                });
            }
        }
        out.join(if compact { ", " } else { " | " })
    };
    let (mut p, mut c) = (0, 0);
    let mut rows = vec![];
    for d in board_drive(document, s.board.feature)? {
        let (feature, derivation) = match &d {
            Derived::Standoff { center, .. } => {
                p += 1;
                (
                    format!("fastener.{STANDOFFS}.{}", p - 1),
                    format!("standoff ({}, {})", number(center[0]), number(center[1])),
                )
            }
            Derived::Opening { rule, missing, .. } => {
                c += 1;
                (
                    format!("cutout.{}", c - 1),
                    format!(
                        "{}{}",
                        match rule {
                            Opening::Circle { diameter } => format!("hole D{}", number(*diameter)),
                            Opening::Window { margin } => format!("window {} mm", number(*margin)),
                        },
                        if *missing { ", source missing" } else { "" }
                    ),
                )
            }
        };
        let status = concerning(&feature, d.source());
        rows.push(vec![feature, d.source().to_owned(), derivation, status]);
    }
    for (i, f) in findings.iter().enumerate() {
        if !used[i] {
            rows.push(vec![
                String::new(),
                f.subject.clone(),
                String::new(),
                if compact {
                    f.kind.to_string()
                } else {
                    format!("{}: {}", f.kind, f.message)
                },
            ]);
        }
    }
    Ok(Schedule {
        kind: "board_drive",
        title: "Board-driven features".into(),
        revision: s.session.revision(),
        source: project_file::ContentHash::document(document)?,
        notes: vec![
            "Panel coordinates: u = board_x + x, v = board_y + y (board Y up)".into(),
            "Findings are review items, not manufacturing approval".into(),
        ],
        columns: ["Feature", "Source", "Derivation", "Findings"]
            .map(String::from)
            .to_vec(),
        rows,
    })
}
/// BOM, bend, hardware and board-drive schedules of the current revision.
/// `compact` shortens the board-drive findings for the drawing table.
pub fn schedules(s: &Scenario, compact: bool) -> Result<Vec<(&'static str, Schedule)>, Error> {
    let session = &s.session;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let bom = model.bom(session.document(), &BomOptions::default())?;
    let sheets = model.sheet_metal(session.document())?;
    Ok(vec![
        ("bom", bom_schedule(&bom)),
        (
            "bend-schedule",
            bend_schedule(session.document(), session.revision(), &sheets)?,
        ),
        ("hardware", hardware_schedule(&model, session)?),
        ("board-drive", findings_schedule(s, compact)?),
    ])
}

fn view(
    name: &str,
    entities: Vec<EntityId>,
    frame: ReferenceFrame,
    at: [f64; 2],
    denominator: u32,
) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model { entities },
        frame,
        kind: ViewKind::Projection,
        position_mm: at,
        scale: Scale {
            numerator: 1,
            denominator,
        },
        hidden_lines: true,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.02,
        guide: None,
        equipment_categories: None,
    }
}
const FRONT: ReferenceFrame = ReferenceFrame {
    origin_mm: [0.; 3],
    u: [1., 0., 0.],
    v: [0., 0., 1.],
};
const TOP: ReferenceFrame = ReferenceFrame {
    origin_mm: [0.; 3],
    u: [1., 0., 0.],
    v: [0., 1., 0.],
};
const SIDE: ReferenceFrame = ReferenceFrame {
    origin_mm: [0.; 3],
    u: [0., 1., 0.],
    v: [0., 0., 1.],
};
fn info(title: &str) -> DrawingInfo {
    DrawingInfo {
        title: title.into(),
        author: "MultiCAD".into(),
        project: "Control desk".into(),
        revision: "B".into(),
        checker: "Pending human review".into(),
    }
}
const ASSUMPTIONS: &str = "Bend allowance from K = 0.44 and R1.5 (stock values); no press-brake, \
    tooling or springback data. Hardware: nominal ISO values and user catalogue entries.";

/// Four A3 pages: folded views with the width dimension; the panel top
/// view with the bend schedule; the BOM and hardware; the board-driven
/// features and their findings.
fn drawings(s: &mut Scenario) -> Result<(), Error> {
    let model = Compiler::default().compile(s.session.document(), s.session.revision())?;
    let mut desk = vec![s.sheet.body, s.cheeks[0].body, s.cheeks[1].body];
    desk.extend(s.hardware.iter().map(|(.., p)| *p));
    let mut stack = desk.clone();
    stack.push(s.stack);
    let page = |number: &str, title: &str| {
        let mut page = Page::new(number);
        page.settings.paper = PaperSize::A3;
        page.info = info(title);
        page
    };

    let mut folded = page("CD-101", "Control desk: folded panel and stack");
    folded.settings.scale = Scale {
        numerator: 1,
        denominator: 2,
    };
    let front = view("Front", stack.clone(), FRONT, [110., 200.], 2);
    let top_view = view("Top", desk.clone(), TOP, [110., 150.], 2);
    let side = view("Side", stack, SIDE, [300., 150.], 2);
    let w = face(&model, s.sheet.body, [-1., 0., 0.], 0.)?;
    let e = face(&model, s.sheet.body, [1., 0., 0.], 240.)?;
    let width = Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Dimension {
            view: top_view.id,
            measure: Box::new(DimensionMeasure::Linear {
                a: w,
                b: e,
                direction: LinearDirection::Horizontal,
            }),
            offset_mm: 12.,
            precision: 0,
            reference: false,
        },
    };
    s.width_dimension = width.id;
    folded.annotations.push(width);
    folded.annotations.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: [20., 25.],
            text: ASSUMPTIONS.into(),
        },
    });
    folded.views.extend([front, top_view, side]);

    let mut bends = page("CD-102", "Control panel: top view and bends");
    bends.settings.scale = Scale {
        numerator: 1,
        denominator: 2,
    };
    bends
        .views
        .push(view("Panel top", vec![s.sheet.body], TOP, [100., 170.], 2));
    bends.annotations.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: [20., 40.],
            text: "Developed blank: control-panel-developed.dxf at 1:1 (CUT, BEND_UP/DOWN, DRILL)"
                .into(),
        },
    });
    let parts = page("CD-103", "Control desk: BOM and purchased hardware");
    let board = page("CD-104", "Control panel: board-driven features");

    let pages = [folded.id, bends.id, parts.id, board.id];
    let (book, prepared) = prepare_create(
        &s.session,
        s.session.revision(),
        actor(),
        "Control desk",
        Some(folded),
    )?;
    checked(&mut s.session, prepared)?;
    for page in [bends, parts, board] {
        let prepared = prepare(
            &s.session,
            s.session.revision(),
            actor(),
            book,
            PageAction::Add(page),
        )?;
        checked(&mut s.session, prepared)?;
    }
    s.book = book;
    s.pages = pages;
    refresh_tables(s)
}
/// Table placement: (page index, schedule name, lower-left corner).
const TABLES: [(usize, &str, [f64; 2]); 4] = [
    (1, "bend-schedule", [215., 200.]),
    (2, "bom", [15., 150.]),
    (2, "hardware", [15., 45.]),
    (3, "board-drive", [15., 60.]),
];
/// Replace the schedule tables with fresh ones from the current revision.
pub fn refresh_tables(s: &mut Scenario) -> Result<(), Error> {
    let fresh: BTreeMap<_, _> = schedules(s, true)?.into_iter().collect();
    for index in 1..4 {
        let mut page = multicad_core::drawings::read(s.session.document(), s.book)?
            .pages
            .into_iter()
            .find(|p| p.id == s.pages[index])
            .ok_or("Drawing page is missing")?;
        page.annotations
            .retain(|a| !matches!(a.kind, AnnotationKind::Table { .. }));
        for (_, name, at) in TABLES.iter().filter(|(i, ..)| *i == index) {
            page.annotations.push(Annotation {
                id: EntityId::new(),
                kind: fresh[name].table(*at),
            });
        }
        let prepared = prepare(
            &s.session,
            s.session.revision(),
            actor(),
            s.book,
            PageAction::Replace(page),
        )?;
        checked(&mut s.session, prepared)?;
    }
    Ok(())
}
/// The placed tables by schedule name.
pub fn placed_tables(s: &Scenario) -> Result<BTreeMap<&'static str, AnnotationKind>, Error> {
    let book = multicad_core::drawings::read(s.session.document(), s.book)?;
    let mut out = BTreeMap::new();
    for (index, name, _) in TABLES {
        let page = book
            .pages
            .iter()
            .find(|p| p.id == s.pages[index])
            .ok_or("Drawing page is missing")?;
        let title = match name {
            "bend-schedule" => "Bend schedule",
            "hardware" => "Purchased hardware",
            "bom" => "Bill of materials",
            _ => "Board-driven features",
        };
        let table = page
            .annotations
            .iter()
            .find(|a| matches!(&a.kind, AnnotationKind::Table { title: t, .. } if t.starts_with(title)))
            .ok_or_else(|| format!("No {name} table"))?;
        out.insert(name, table.kind.clone());
    }
    Ok(out)
}

fn sets(s: &mut Scenario) -> Result<(), Error> {
    let tagged = |tag: &str| Scope {
        root: ScopeRoot::Project,
        tags: Some(TagQuery {
            all: [tag.to_string()].into(),
            ..Default::default()
        }),
    };
    let drawings = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: vec![s.sheet.body],
            },
            tags: None,
        },
        pages: Pages::Project,
        recipes: vec![
            Recipe {
                id: EntityId::new(),
                name: "PDF".into(),
                output: RecipeOutput::Pdf,
            },
            Recipe {
                id: EntityId::new(),
                name: "Drawing DXF".into(),
                output: RecipeOutput::DrawingDxf {
                    units: LengthUnit::Millimeter,
                    layers: Layers::default(),
                },
            },
        ],
        ..Default::default()
    };
    let step = SetDefinition {
        scope: tagged(FABRICATED),
        recipes: vec![Recipe::step("STEP")],
        ..Default::default()
    };
    let print = SetDefinition {
        scope: tagged(PRINTED),
        recipes: vec![Recipe {
            id: EntityId::new(),
            name: "3MF".into(),
            output: RecipeOutput::Mesh {
                format: MeshFormat::ThreeMf,
                units: LengthUnit::Millimeter,
                hierarchy: HierarchyMode::SeparateParts,
                names: NameHandling::ProductNames,
                aids: AidPolicy::IncludeAll,
                quality: MeshQuality::default(),
            },
        }],
        ..Default::default()
    };
    let mut ids = vec![];
    for (name, definition) in [
        ("Control desk drawings", drawings),
        ("Fabricated parts STEP", step),
        ("Printed guide 3MF", print),
    ] {
        let id = EntityId::new();
        let prepared = prepare_set(
            &s.session,
            s.session.revision(),
            actor(),
            id,
            name,
            &definition,
        )?;
        s.session.commit(prepared)?;
        ids.push(id);
    }
    [s.drawing_set, s.step_set, s.print_set] = [ids[0], ids[1], ids[2]];
    Ok(())
}

/// Preview and run an export set into `root`; returns the receipt.
pub fn export(session: &Session, set: EntityId, root: &Path) -> Result<RunReceipt, Error> {
    let exporter = Exporter::default();
    let preview = exporter.preview(session.document(), session.revision(), set, &actor(), None)?;
    if !preview.can_run() {
        return Err(format!("Export set cannot run: {:?}", preview.recipes).into());
    }
    Ok(exporter.run(
        session.document(),
        session.revision(),
        &preview.key,
        root,
        &actor(),
        None,
    )?)
}
/// The developed panel DXF, checked against the current revision.
pub fn developed_dxf(s: &Scenario) -> Result<multicad_model::export::sheet_dxf::SheetDxf, Error> {
    let dxf = Exporter::default().developed_sheet_dxf(
        s.session.document(),
        s.session.revision(),
        s.sheet.body,
        LengthUnit::Millimeter,
        &SheetDxfLayers::default(),
        None,
    )?;
    dxf.check_current(s.session.document(), s.session.revision())?;
    Ok(dxf)
}

pub fn archive(session: &Session) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
}

/// Set a project parameter's expression as one transaction.
pub fn set_parameter(session: &mut Session, id: EntityId, expression: &str) -> Result<(), Error> {
    let mut entity = session
        .document()
        .get(id)
        .ok_or("Missing parameter")?
        .clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        return Err("Not a parameter".into());
    };
    *e = expression.into();
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Edit parameter".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    checked(session, prepared)
}

fn json_findings(findings: &[Finding]) -> serde_json::Value {
    findings
        .iter()
        .map(|f| serde_json::json!({ "kind": f.kind, "subject": f.subject, "message": f.message }))
        .collect()
}
fn sha256(path: &Path) -> Result<String, Error> {
    Ok(project_file::ContentHash::of(&std::fs::read(path)?).to_string())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let sources = directory.join("sources");
    let mut s = build(&sources).map_err(|e| format!("Build: {e}"))?;
    let revision_a = s.findings().map_err(|e| format!("Findings A: {e}"))?;
    let plan = reimport(&mut s, &sources).map_err(|e| format!("Reimport: {e}"))?;
    let after_reimport = s.findings().map_err(|e| format!("Findings B: {e}"))?;
    repair(&mut s, &sources)?;
    let findings = s.findings()?;

    let project = directory.join("control-desk.multicad");
    write_new(&project, &archive(&s.session)?)?;
    let exports = directory.join("exports");
    std::fs::create_dir(&exports)?;
    let exports = exports.canonicalize()?;
    let mut files = vec![project.clone()];
    for set in [s.drawing_set, s.step_set, s.print_set] {
        let receipt = export(&s.session, set, &exports)?;
        files.extend(
            receipt
                .files
                .iter()
                .map(|f| Path::new(&receipt.directory).join(&f.artifact.output.filename)),
        );
    }
    let dxf = developed_dxf(&s)?;
    let developed = exports.join("control-panel-developed.dxf");
    write_new(&developed, &dxf.bytes)?;
    files.push(developed);
    for (name, schedule) in schedules(&s, false)? {
        let path = exports.join(format!("{name}.csv"));
        write_new(&path, schedule.csv().as_bytes())?;
        files.push(path);
    }
    let model = Compiler::default().compile(s.session.document(), s.session.revision())?;
    let sheet = model
        .sheet_metal(s.session.document())?
        .into_iter()
        .find(|r| r.part == s.sheet.body)
        .ok_or("The panel is not evaluated")?;
    let pattern = &dxf.pattern;
    let manifest = serde_json::json!({
        "generator": "control_desk_review",
        "project": project,
        "revision": s.session.revision(),
        "board": {
            "file": BOARD_FILE,
            "reimport": {
                "moved": plan.moved,
                "added": plan.added,
                "missing": plan.missing,
                "holes_changed": plan.holes_changed,
                "outline_changed": plan.outline_changed,
            },
            "missing_models_at_import": s.missing_models,
            "unsupported": s.unsupported,
        },
        "panel": {
            "thickness_mm": sheet.thickness_mm,
            "inside_radius_mm": sheet.inside_radius_mm,
            "k_factor": sheet.k_factor,
            "volume_mm3": sheet.volume_mm3,
            "flat_size_mm": sheet.flat_size_mm,
            "flat_area_mm2": sheet.flat_area_mm2,
            "bends": pattern.bends.iter().map(|b| &b.id).collect::<Vec<_>>(),
            "drills": pattern.drills.len(),
            "diagnostics": pattern.diagnostics.iter().map(|d| serde_json::json!({
                "kind": d.kind, "scope": d.scope.label(), "subject": d.subject, "message": d.message,
            })).collect::<Vec<_>>(),
        },
        "findings": {
            "revision_a": json_findings(&revision_a),
            "after_reimport": json_findings(&after_reimport),
            "after_repair": json_findings(&findings),
        },
        "files": files.iter().map(|f| Ok(serde_json::json!({
            "path": f,
            "sha256": sha256(f)?,
        }))).collect::<Result<Vec<_>, Error>>()?,
        "verification": {
            "native_geometry": true,
            "interactive_macos": false,
            "windows_linux_runtime": false,
            "human_acceptance": false,
            "press_brake_tooling_or_springback": false,
            "physical_fit_or_manufacture": false,
            "real_kicad_project": false,
        },
    });
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Usage: cargo run -p multicad-app --example control_desk_review -- NEW_DIRECTORY")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&generate(Path::new(&path))?)?
    );
    Ok(())
}
