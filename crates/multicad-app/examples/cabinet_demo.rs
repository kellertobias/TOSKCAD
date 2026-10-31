//! The shipped 2 m cabinet demo (CAD-182), built only through the typed
//! transactional API that the window and the MCP server share.
//!
//! Run `cargo run -p multicad-app --example cabinet_demo -- examples` to
//! regenerate `examples/cabinet.tcd` and the hardware library
//! `examples/libraries/cabinet-hardware-library.tcd` (listed in the
//! cabinet's `requires` in `examples/examples.json`). Identities are fresh on
//! every run, so the files are reproducible in content, not byte for byte.
//!
//! The cabinet, from bottom to top:
//!
//! - a door compartment behind a left-hand and a right-hand full-overlay
//!   door, with one adjustable shelf on System 32 pins;
//! - two drawers on side-mount full-extension runners;
//! - open adjustable shelving with two shelves on System 32 pins.
//!
//! The carcass is birch plywood joined with Domino-style loose tenons; the
//! drawer boxes are finger jointed. Runners, hinges and mounting plates are
//! pinned symbols imported from the hardware library file. Doors and
//! drawers move through revolute and slider joints with saved poses.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    drawings::*,
    inspection::GeometryReference,
    materials::*,
    model::{
        self, AssemblyRequest, Back, BodyOutput, CarcassDefinition, CarcassRequest,
        ComponentPlacementRequest, Construction, GrainAxis, HardwareHole,
        HardwarePatternDefinition, InstanceMoveRequest, JointAnchor, JointKind, JointLimit,
        JointLimits, JointParameters, JointPoseValue, JointRequest, JointValues, LooseTenonSpec,
        MachiningDefinition, MachiningFace, MachiningKind, NamedPoseRequest, NumericTransform,
        PanelAxis, PanelDefinition, PanelEdges, PanelJointDefinition, PanelJointKind,
        PanelJointPair, PanelPlane, PanelRequest, PanelRole, PanelSource, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, RowEdge, ShelfLayout, ShelfPinRow, Shelves, angle,
        length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
    schedules::{ScheduleKind, ScheduleSpec},
    symbols::{self, SymbolAuthorRequest, SymbolDefinition, SymbolKind, SymbolPlaceRequest},
};
use multicad_model::{
    Compiler, DrawingProviders, Model,
    schedules::{self, Schedule},
};
use std::{collections::BTreeMap, io::Write, path::Path};

pub type Error = Box<dyn std::error::Error>;

pub const CABINET_FILE: &str = "cabinet.tcd";
/// In a subfolder, so the Examples gallery lists it only as the cabinet's
/// requirement, not as a demo of its own.
pub const LIBRARY_FILE: &str = "libraries/cabinet-hardware-library.tcd";
pub const CABINET_NAME: &str = "2 m cabinet with doors, drawers and System 32 shelves";
pub const LIBRARY_NAME: &str = "Cabinet hardware library";
pub const PURCHASED: &str = "purchased";

/// Carcass and drawer-box stock thicknesses; neither stock is edited.
pub const CARCASS_T: f64 = 18.;
pub const BOX_T: f64 = 12.;
/// The runner's cabinet and drawer members, each half the 12.7 mm side gap.
pub const RUNNER_T: f64 = 6.35;
pub const RUNNER_H: f64 = 45.;
/// Drawer box side to carcass side: both runner members.
pub const SIDE_GAP: f64 = CARCASS_T + 2. * RUNNER_T;
pub const BOX_H: f64 = 150.;
/// The drawer box floor above each drawer opening's bottom.
pub const BOX_LIFT: f64 = 20.;
/// Runner bottom above the drawer box bottom, centring it on the box side.
pub const RUNNER_LIFT: f64 = (BOX_H - RUNNER_H) / 2.;
/// Hinge cup center from the door's hinge edge and from its ends.
pub const CUP_EDGE: f64 = 22.5;
pub const CUP_END: f64 = 100.;
/// System 32 setback of hinge plates and runner screws from the front edge.
pub const SYSTEM32_SETBACK: f64 = 37.;
/// Lower and upper shelf-pin rows: first hole above the side bottom / the
/// upper divider, and holes per row.
pub const LOWER_PIN_START: f64 = 230.;
pub const LOWER_PIN_COUNT: u32 = 12;
pub const UPPER_PIN_RISE: f64 = 100.;
pub const UPPER_PIN_COUNT: u32 = 15;
pub const PIN_D: f64 = 5.;
/// Hole indices (from 0) that carry the adjustable shelves.
pub const LOWER_SHELF_HOLE: u32 = 5;
pub const UPPER_SHELF_HOLES: [u32; 2] = [4, 10];

pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}

/// Commit a typed transaction. The core prepare step has already validated
/// the source; the full native evaluation runs at each [`milestone`], which
/// keeps generation of this ~80 Part model practical.
pub fn checked(session: &mut Session, prepared: Prepared) -> Result<(), Error> {
    session.commit(prepared)?;
    Ok(())
}
/// Full native evaluation of the current source with every reference
/// resolved; fails the build at the step group that broke it.
pub fn milestone(session: &Session, label: &str) -> Result<Model, Error> {
    let start = std::time::Instant::now();
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .map_err(|e| format!("{label}: {e}"))?;
    if !model
        .snapshot()
        .references
        .for_model(session.document())?
        .all_resolved()
    {
        return Err(format!("{label}: native references are unresolved").into());
    }
    if std::env::var_os("CABINET_DEMO_TRACE").is_some() {
        eprintln!("{label}: native evaluation {:?}", start.elapsed());
    }
    Ok(model)
}
/// Native preflight before committing, for single checked edits.
pub fn preflight(session: &mut Session, prepared: Prepared) -> Result<(), Error> {
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
fn parameter(name: &str, expression: &str, unit: &str, description: &str) -> Entity {
    let mut e = Entity::new(
        name,
        EntityData::Parameter {
            expression: expression.into(),
            unit: unit.into(),
            exposed: true,
            inputs: vec![],
        },
    );
    e.description = description.into();
    e
}
/// "#120" is a literal millimeter value, "=…" binds an expression.
pub fn dim(session: &Session, text: &str) -> Result<Dimension, Error> {
    dim_in(session.document(), text, "mm")
}
fn dim_in(document: &Document, text: &str, unit: &str) -> Result<Dimension, Error> {
    if let Some(stripped) = text.strip_prefix('#') {
        let value: f64 = stripped.parse()?;
        return Ok(if unit == "deg" {
            angle(value)
        } else {
            length(value)
        });
    }
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(document, text, unit, None)?,
    })
}
fn point(session: &Session, p: [&str; 3]) -> Result<[Dimension; 3], Error> {
    Ok([
        dim(session, p[0])?,
        dim(session, p[1])?,
        dim(session, p[2])?,
    ])
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
fn panel_stock(
    material: EntityId,
    name: &str,
    thickness: f64,
    sheet: [f64; 2],
    grain: Grain,
) -> Result<Entity, Error> {
    Ok(LibraryDefinition::Stock(Stock {
        material,
        specification: StockKind::Panel {
            thickness_mm: thickness,
            length_mm: sheet[0],
            width_mm: sheet[1],
            grain,
        },
    })
    .entity(EntityId::new(), name)?)
}
fn tag(session: &mut Session, parts: &[EntityId], tag: &str) -> Result<(), Error> {
    let prepared = prepare_tags(
        session,
        session.revision(),
        actor(),
        parts,
        &TagEdit::Add([tag.to_string()].into()),
    )?;
    checked(session, prepared)
}

// ---------------------------------------------------------------------------
// Hardware library
// ---------------------------------------------------------------------------

/// The generic hardware the demo uses. These are documented example
/// dimensions, not a compatibility claim for a particular product.
pub struct Hardware {
    pub runner_cabinet: SymbolDefinition,
    pub runner_drawer: SymbolDefinition,
    pub hinge: SymbolDefinition,
    pub plate: SymbolDefinition,
    /// The library's exposed runner length parameter (template identity).
    pub runner_length: EntityId,
}
pub struct Library {
    pub session: Session,
    pub hardware: Hardware,
}

fn primitive(
    session: &mut Session,
    name: &str,
    parameters: PrimitiveParameters,
) -> Result<EntityId, Error> {
    let request = PrimitiveRequest::new(name, parameters);
    let prepared = model::prepare_primitive(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request.body)
}
fn boxed(origin: [Dimension; 3], size: [Dimension; 3]) -> PrimitiveParameters {
    PrimitiveParameters::Box {
        size,
        frame: PrimitiveFrame {
            origin,
            ..Default::default()
        },
    }
}
fn place(
    session: &mut Session,
    component: EntityId,
    name: &str,
    translation: [Dimension; 3],
) -> Result<EntityId, Error> {
    let mut request = ComponentPlacementRequest::new(
        component,
        NumericTransform {
            translation,
            ..Default::default()
        },
    );
    request.name = name.into();
    let prepared = model::prepare_place_component(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request.instance)
}
/// The hardware library: runner members, a concealed hinge and its plate,
/// authored as pinned symbols in their own project file.
pub fn build_library(runner_length_mm: f64, revision: &str) -> Result<Library, Error> {
    let runner_length = parameter(
        "runner_length",
        &runner_length_mm.to_string(),
        "mm",
        "Nominal runner length (closed), also its full-extension travel",
    );
    let runner_id = runner_length.id;
    let mut session =
        Session::new(Document::new(LIBRARY_NAME)?.edited(&[Mutation::Insert(runner_length)])?)?;
    let len = dim(&session, "=$runner_length")?;
    // Side-mount full-extension runner, split into its cabinet member and
    // drawer member (each 6.35 mm of the 12.7 mm side clearance, 45 mm high).
    let cabinet = primitive(
        &mut session,
        "Runner cabinet member",
        boxed(
            [length(0.), length(0.), length(0.)],
            [length(RUNNER_T), len.clone(), length(RUNNER_H)],
        ),
    )?;
    let drawer = primitive(
        &mut session,
        "Runner drawer member",
        boxed(
            [length(0.), length(0.), length(0.)],
            [length(RUNNER_T), len.clone(), length(RUNNER_H)],
        ),
    )?;
    // Concealed 35 mm cup hinge for a full-overlay door. Local origin: the
    // cup center on the door's back face; +X away from the hinge side, +Y
    // into the cabinet. The cup sits in a Ø35 × 13 mm boring.
    let cup = primitive(
        &mut session,
        "Hinge cup",
        PrimitiveParameters::Cylinder {
            radius: length(17.5),
            height: length(12.5),
            frame: PrimitiveFrame {
                origin: [length(0.), length(0.), length(0.)],
                u: [1., 0., 0.],
                v: [0., 0., 1.],
            },
        },
    )?;
    let arm = primitive(
        &mut session,
        "Hinge arm",
        boxed(
            [length(-3.5), length(0.), length(-7.)],
            [length(6.), length(43.), length(14.)],
        ),
    )?;
    // Cruciform mounting plate on the side's inner face: local origin at the
    // plate's center on that face, +X into the cabinet.
    let plate = primitive(
        &mut session,
        "Hinge mounting plate",
        boxed(
            [length(0.), length(-20.), length(-25.)],
            [length(3.), length(40.), length(50.)],
        ),
    )?;
    tag(&mut session, &[cabinet, drawer, cup, arm, plate], PURCHASED)?;
    let cup_member = place(
        &mut session,
        cup,
        "Hinge cup member",
        [length(0.), length(0.), length(0.)],
    )?;
    let arm_member = place(
        &mut session,
        arm,
        "Hinge arm member",
        [length(0.), length(0.), length(0.)],
    )?;
    let hinge = AssemblyRequest::new(
        "Concealed hinge 110°",
        vec![cup_member, arm_member],
        vec![cup, arm],
    );
    let prepared = model::prepare_assembly(&session, session.revision(), actor(), &hinge)?;
    checked(&mut session, prepared.prepared)?;

    // Stable symbol identities across library revisions, so a later
    // revision updates the same symbols instead of adding new ones.
    let mut ids = library_symbol_ids().into_iter();
    let mut author_at = |name: &str, root, kind, exposed: Vec<EntityId>| {
        let mut request = SymbolAuthorRequest::new(name, root, kind, exposed, None);
        request.revision = revision.into();
        request.symbol = ids.next().expect("four symbols");
        request
    };
    let requests = [
        author_at(
            "Full-extension runner — cabinet member",
            cabinet,
            SymbolKind::Dynamic,
            vec![runner_id],
        ),
        author_at(
            "Full-extension runner — drawer member",
            drawer,
            SymbolKind::Dynamic,
            vec![runner_id],
        ),
        author_at(
            "Concealed cup hinge 110°, full overlay",
            hinge.assembly,
            SymbolKind::Static,
            vec![],
        ),
        author_at(
            "Hinge mounting plate, cruciform",
            plate,
            SymbolKind::Static,
            vec![],
        ),
    ];
    let mut defs = vec![];
    for request in &requests {
        let prepared = symbols::prepare_author(&session, session.revision(), actor(), request)?;
        checked(&mut session, prepared.prepared)?;
        defs.push(symbols::read_definition(
            session.document(),
            request.definition,
        )?);
    }
    // One placed copy of the hinge, so the library file shows it too.
    place(
        &mut session,
        hinge.assembly,
        "Concealed hinge 110° (shown)",
        zero(),
    )?;
    milestone(&session, "Hardware library")?;
    let mut defs = defs.into_iter();
    Ok(Library {
        session,
        hardware: Hardware {
            runner_cabinet: defs.next().ok_or("missing")?,
            runner_drawer: defs.next().ok_or("missing")?,
            hinge: defs.next().ok_or("missing")?,
            plate: defs.next().ok_or("missing")?,
            runner_length: runner_id,
        },
    })
}

/// Fixed symbol identities, so a later library revision updates the same
/// symbols rather than adding new ones.
pub fn library_symbol_ids() -> [EntityId; 4] {
    [
        "6b1f0c52-9a51-4d1e-8a0a-0c1b8f3e2a01",
        "6b1f0c52-9a51-4d1e-8a0a-0c1b8f3e2a02",
        "6b1f0c52-9a51-4d1e-8a0a-0c1b8f3e2a03",
        "6b1f0c52-9a51-4d1e-8a0a-0c1b8f3e2a04",
    ]
    .map(|s| EntityId::parse(s).expect("valid UUID"))
}

// ---------------------------------------------------------------------------
// Cabinet
// ---------------------------------------------------------------------------

pub struct Params {
    pub width: EntityId,
    pub depth: EntityId,
    pub height: EntityId,
    pub door_zone: EntityId,
    pub drawer_zone: EntityId,
    pub door_max: EntityId,
    pub drawer_travel: EntityId,
}
pub struct Stocks {
    pub ply18: EntityId,
    pub ply12: EntityId,
    pub ply6: EntityId,
    pub hdf8: EntityId,
}

fn start_cabinet() -> Result<(Session, Params, Stocks), Error> {
    let width = parameter("cabinet_width", "800", "mm", "Outside width of the carcass");
    let depth = parameter(
        "cabinet_depth",
        "500",
        "mm",
        "Outside depth of the carcass, front edges to the rear edges",
    );
    let height = parameter(
        "cabinet_height",
        "2000",
        "mm",
        "Outside height, floor to top",
    );
    let door_zone = parameter(
        "door_zone",
        "800",
        "mm",
        "Clear height of the door compartment (bottom to the lower divider)",
    );
    let drawer_zone = parameter(
        "drawer_zone",
        "400",
        "mm",
        "Clear height of the drawer compartment (two equal drawer openings)",
    );
    let door_max = parameter(
        "door_max_angle",
        "110",
        "deg",
        "Opening limit of the doors (the hinge's nominal 110°)",
    );
    let drawer_travel = parameter(
        "drawer_travel",
        "450",
        "mm",
        "Drawer travel: full extension of the 450 mm runners",
    );
    let params = Params {
        width: width.id,
        depth: depth.id,
        height: height.id,
        door_zone: door_zone.id,
        drawer_zone: drawer_zone.id,
        door_max: door_max.id,
        drawer_travel: drawer_travel.id,
    };
    let mut session = Session::new(Document::new(CABINET_NAME)?.edited(&[
        Mutation::Insert(width),
        Mutation::Insert(depth),
        Mutation::Insert(height),
        Mutation::Insert(door_zone),
        Mutation::Insert(drawer_zone),
        Mutation::Insert(door_max),
        Mutation::Insert(drawer_travel),
    ])?)?;
    let birch = material("Birch plywood", 680., [214, 186, 140, 255]);
    let hdf = material("HDF", 850., [150, 120, 95, 255]);
    let ply18 = panel_stock(
        birch.id,
        "Birch ply 18",
        18.,
        [2500., 1250.],
        Grain::AlongLength,
    )?;
    let ply12 = panel_stock(
        birch.id,
        "Birch ply 12",
        12.,
        [2500., 1250.],
        Grain::AlongLength,
    )?;
    let ply6 = panel_stock(
        birch.id,
        "Birch ply 6",
        6.,
        [2500., 1250.],
        Grain::AlongLength,
    )?;
    let hdf8 = panel_stock(hdf.id, "HDF 8", 8., [2800., 2070.], Grain::None)?;
    let stocks = Stocks {
        ply18: ply18.id,
        ply12: ply12.id,
        ply6: ply6.id,
        hdf8: hdf8.id,
    };
    let prepared = prepare_library(&session, session.revision(), actor(), &[birch, hdf])?;
    session.commit(prepared)?;
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[ply18, ply12, ply6, hdf8],
    )?;
    session.commit(prepared)?;
    Ok((session, params, stocks))
}

#[allow(clippy::too_many_arguments)]
fn rect_panel(
    session: &mut Session,
    name: &str,
    stock: EntityId,
    plane: PanelPlane,
    origin: [&str; 3],
    size: [&str; 2],
    grain: GrainAxis,
) -> Result<EntityId, Error> {
    let request = PanelRequest::new(
        name,
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Rectangle {
                stock,
                plane,
                origin: point(session, origin)?,
                length: dim(session, size[0])?,
                width: dim(session, size[1])?,
            },
            grain,
            edges: PanelEdges::default(),
            oversize: None,
        },
    );
    let prepared = model::prepare_panel(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request.body)
}
fn machining(
    session: &mut Session,
    name: &str,
    kind: MachiningKind,
    face: MachiningFace,
    targets: Vec<EntityId>,
) -> Result<EntityId, Error> {
    let prepared = model::prepare_machining(
        session,
        session.revision(),
        actor(),
        name,
        &MachiningDefinition {
            kind,
            face,
            targets,
            suppressed: false,
        },
    )?;
    let id = prepared.operation;
    checked(session, prepared.model.prepared)?;
    Ok(id)
}
fn hardware_pattern(
    session: &mut Session,
    name: &str,
    holes: &[([f64; 2], f64, f64)],
) -> Result<EntityId, Error> {
    let (id, prepared) = model::prepare_hardware(
        session,
        session.revision(),
        actor(),
        name,
        &HardwarePatternDefinition {
            holes: holes
                .iter()
                .map(|(at, d, depth)| HardwareHole {
                    at: at.map(length),
                    diameter: length(*d),
                    depth: Some(length(*depth)),
                })
                .collect(),
            allowances: vec![],
        },
    )?;
    checked(session, prepared)?;
    Ok(id)
}
fn hardware_at(
    session: &mut Session,
    name: &str,
    pattern: EntityId,
    at: [&str; 2],
    quarter_turns: u8,
    face: MachiningFace,
    targets: Vec<EntityId>,
) -> Result<EntityId, Error> {
    let kind = MachiningKind::Hardware {
        pattern,
        at: [dim(session, at[0])?, dim(session, at[1])?],
        quarter_turns,
    };
    machining(session, name, kind, face, targets)
}
/// Import a library symbol definition (pinned) and return its embedded id.
fn import(session: &mut Session, def: &SymbolDefinition) -> Result<EntityId, Error> {
    let prepared = symbols::prepare_import(session, session.revision(), actor(), def)?;
    let id = prepared.definition;
    checked(session, prepared.prepared)?;
    Ok(id)
}
/// Place a symbol, then drive its outer Instance with a parametric
/// placement (optionally turned 180° about +Y for the right-hand side).
fn place_symbol(
    session: &mut Session,
    definition: EntityId,
    name: &str,
    translation: [&str; 3],
    mirrored: bool,
) -> Result<EntityId, Error> {
    let request = SymbolPlaceRequest::new(
        session.document(),
        definition,
        name,
        BTreeMap::new(),
        Placement::default(),
    )?;
    let prepared = symbols::prepare_place(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    let mut transform = NumericTransform {
        translation: point(session, translation)?,
        ..Default::default()
    };
    if mirrored {
        transform.axis = [0., 1., 0.];
        transform.angle = angle(180.);
    }
    let prepared = model::prepare_instance_moves(
        session,
        session.revision(),
        actor(),
        &[InstanceMoveRequest::new(request.instance, transform)],
    )?;
    checked(session, prepared.prepared)?;
    Ok(request.instance)
}
fn assembly(
    session: &mut Session,
    name: &str,
    members: Vec<EntityId>,
    definition_parts: Vec<EntityId>,
) -> Result<(EntityId, EntityId), Error> {
    let request = AssemblyRequest::new(name, members, definition_parts);
    let prepared = model::prepare_assembly(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    let root = place(
        session,
        request.assembly,
        name,
        [length(0.), length(0.), length(0.)],
    )?;
    Ok((request.assembly, root))
}
fn frame(
    session: &Session,
    origin: [&str; 3],
    u: [f64; 3],
    v: [f64; 3],
) -> Result<JointAnchor, Error> {
    Ok(JointAnchor::Frame {
        frame: Box::new(PrimitiveFrame {
            origin: point(session, origin)?,
            u,
            v,
        }),
    })
}
fn zero() -> [Dimension; 3] {
    [length(0.), length(0.), length(0.)]
}

/// A drawer: its front, finger-jointed box with a grooved bottom, the
/// runner members and their screw holes, and its slider.
pub struct Drawer {
    pub front: EntityId,
    pub box_front: EntityId,
    pub back: EntityId,
    pub left: EntityId,
    pub right: EntityId,
    pub bottom: EntityId,
    pub fingers: EntityId,
    pub groove: EntityId,
    pub runner_cabinet: [EntityId; 2],
    pub runner_drawer: [EntityId; 2],
    pub cabinet_screws: EntityId,
    pub drawer_screws: [EntityId; 2],
    pub assembly: EntityId,
    pub root: EntityId,
    pub slider: EntityId,
}
impl Drawer {
    pub fn parts(&self) -> [EntityId; 6] {
        [
            self.front,
            self.box_front,
            self.back,
            self.left,
            self.right,
            self.bottom,
        ]
    }
}
/// A door leaf with its hinges (moving), plates (fixed), borings and hinge.
pub struct Door {
    pub panel: EntityId,
    pub hinges: [EntityId; 2],
    pub plates: [EntityId; 2],
    pub cups: [EntityId; 2],
    pub assembly: EntityId,
    pub root: EntityId,
    pub hinge_joint: EntityId,
}
pub struct Cabinet {
    pub session: Session,
    pub params: Params,
    pub stocks: Stocks,
    pub carcass: EntityId,
    pub panels: Vec<(PanelRole, EntityId)>,
    pub dominos: EntityId,
    pub lower_pins: EntityId,
    pub upper_pins: EntityId,
    pub plate_holes: [EntityId; 2],
    pub shelves: [EntityId; 3],
    pub pin_part: EntityId,
    pub pin_copies: Vec<EntityId>,
    pub doors: [Door; 2],
    pub drawers: [Drawer; 2],
    /// Embedded definitions: runner cabinet member, runner drawer member,
    /// hinge, plate.
    pub symbols: [EntityId; 4],
    pub patterns: BTreeMap<&'static str, EntityId>,
    pub poses: BTreeMap<&'static str, EntityId>,
    pub book: EntityId,
    pub pages: Vec<EntityId>,
    pub dimensions: [EntityId; 3],
}
impl Cabinet {
    pub fn role(&self, role: PanelRole) -> EntityId {
        self.panels
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, p)| *p)
            .expect("known carcass role")
    }
}

/// Festool DOMINO 8 × 22 × 40 mm tenons with the depths and plays of
/// docs/architecture/cabinetry.md, three per joint 60 mm from the ends.
pub fn domino_spec() -> LooseTenonSpec {
    LooseTenonSpec {
        thickness: length(8.),
        width: length(22.),
        length: length(40.),
        count: 3,
        inset: length(60.),
        offset: None,
        face_depth: length(15.),
        edge_depth: length(28.),
        face_play: length(6.),
        edge_play: length(0.),
        fit: length(0.),
    }
}

/// Build the whole cabinet against the given hardware library.
pub fn build_cabinet(hardware: &Hardware) -> Result<Cabinet, Error> {
    let (mut session, params, stocks) = start_cabinet()?;
    let s = &mut session;
    // Carcass: full-height sides, an inset HDF back and two fixed dividers
    // (carcass shelves without setback or clearance) that separate the door,
    // drawer and open-shelf compartments.
    let carcass = model::prepare_carcass(
        s,
        s.revision(),
        actor(),
        &CarcassRequest::new(
            "Cabinet",
            CarcassDefinition {
                origin: zero(),
                rotation: angle(0.),
                width: dim(s, "=$cabinet_width")?,
                height: dim(s, "=$cabinet_height")?,
                depth: dim(s, "=$cabinet_depth")?,
                stock: stocks.ply18,
                construction: Construction::SidesFullHeight,
                back: Back::Inset {
                    stock: stocks.hdf8,
                    recess: length(10.),
                },
                shelves: Shelves {
                    layout: ShelfLayout::Offsets {
                        offsets: vec![
                            dim(s, "=$door_zone")?,
                            dim(s, "=$door_zone + $drawer_zone + 18 mm")?,
                        ],
                    },
                    setback: length(0.),
                    clearance: length(0.),
                },
                front_band: None,
                group: None,
            },
        ),
    )?;
    let carcass_id = carcass.carcass;
    checked(s, carcass.model.prepared)?;
    let panels: Vec<(PanelRole, EntityId)> = model::carcass_panels(s.document(), carcass_id)?
        .into_iter()
        .map(|(role, _, part)| (role, part))
        .collect();
    let role = |r: PanelRole| {
        panels
            .iter()
            .find(|(x, _)| *x == r)
            .map(|(_, p)| *p)
            .ok_or("missing carcass role")
    };
    let (left, right) = (role(PanelRole::LeftSide)?, role(PanelRole::RightSide)?);
    let horizontal = [
        role(PanelRole::Top)?,
        role(PanelRole::Bottom)?,
        role(PanelRole::Shelf { index: 0 })?,
        role(PanelRole::Shelf { index: 1 })?,
    ];

    // Domino carcass joints: every horizontal panel to both sides.
    let joint = model::prepare_panel_joint(
        s,
        s.revision(),
        actor(),
        "Carcass Dominos",
        &PanelJointDefinition {
            kind: PanelJointKind::LooseTenon(Box::new(domino_spec())),
            pairs: horizontal
                .iter()
                .flat_map(|h| {
                    [
                        PanelJointPair { a: left, b: *h },
                        PanelJointPair { a: right, b: *h },
                    ]
                })
                .collect(),
        },
    )?;
    let dominos = joint.joint;
    checked(s, joint.model.prepared)?;
    milestone(s, "Carcass and Dominos")?;

    // System 32 shelf-pin rows on both inner side faces.
    let rows = vec![
        ShelfPinRow {
            from: RowEdge::Start,
            offset: length(SYSTEM32_SETBACK),
        },
        ShelfPinRow {
            from: RowEdge::End,
            offset: length(SYSTEM32_SETBACK),
        },
    ];
    let pins = |start: Dimension, count: u32| MachiningKind::ShelfPins {
        along: PanelAxis::V,
        rows: rows.clone(),
        pitch: length(32.),
        start,
        end: length(100.),
        count: Some(count),
        diameter: length(PIN_D),
        depth: length(10.),
    };
    let lower_pins = machining(
        s,
        "Shelf pins, door compartment",
        pins(length(LOWER_PIN_START), LOWER_PIN_COUNT),
        MachiningFace::Inside,
        vec![left, right],
    )?;
    let upper_start = dim(
        s,
        &format!(
            "=$door_zone + $drawer_zone + {} mm",
            36. + CARCASS_T + UPPER_PIN_RISE
        ),
    )?;
    let upper_pins = machining(
        s,
        "Shelf pins, open shelving",
        pins(upper_start, UPPER_PIN_COUNT),
        MachiningFace::Inside,
        vec![left, right],
    )?;

    // Hardware patterns (reusable drilling templates).
    let mut patterns = BTreeMap::new();
    let plate_pattern = hardware_pattern(
        s,
        "Hinge plate holes (System 32, 2 × Ø5 × 12)",
        &[([0., -16.], 5., 12.), ([0., 16.], 5., 12.)],
    )?;
    let cup_pattern = hardware_pattern(
        s,
        "Hinge cup boring Ø35 × 13 with 2 × Ø2.5 screw holes (45/9.5)",
        &[
            ([0., 0.], 35., 13.),
            ([9.5, -22.5], 2.5, 10.),
            ([9.5, 22.5], 2.5, 10.),
        ],
    )?;
    let runner_holes = [37., 165., 293., 421.];
    let cabinet_screws = hardware_pattern(
        s,
        "Runner cabinet-member screws (4 × Ø3 × 10)",
        &runner_holes.map(|u| ([u, 0.], 3., 10.)),
    )?;
    let drawer_screws = hardware_pattern(
        s,
        "Runner drawer-member screws (4 × Ø2.5 × 8)",
        &runner_holes.map(|u| ([u, 0.], 2.5, 8.)),
    )?;
    patterns.insert("plate", plate_pattern);
    patterns.insert("cup", cup_pattern);
    patterns.insert("runner_cabinet", cabinet_screws);
    patterns.insert("runner_drawer", drawer_screws);

    // Hinge heights: 100 mm from each door end.
    let hinge_z = [
        format!("#{}", 2. + CUP_END),
        format!("=$door_zone + {} mm", 25.5 - CUP_END),
    ];
    let setback = format!("#{SYSTEM32_SETBACK}");
    let plate_holes = [
        hardware_at(
            s,
            "Hinge plate holes, lower",
            plate_pattern,
            [&setback, &hinge_z[0]],
            0,
            MachiningFace::Inside,
            vec![left, right],
        )?,
        hardware_at(
            s,
            "Hinge plate holes, upper",
            plate_pattern,
            [&setback, &hinge_z[1]],
            0,
            MachiningFace::Inside,
            vec![left, right],
        )?,
    ];

    milestone(s, "Shelf pins and hinge plate drilling")?;
    // Adjustable shelves resting on pins, and the pins themselves.
    let shelf_size = ["=$cabinet_width - 38 mm", "#460"];
    let pin_z = |index: u32| LOWER_PIN_START + 32. * index as f64;
    let upper_hole = |index: u32| 36. + CARCASS_T + UPPER_PIN_RISE + 32. * index as f64;
    let lower_shelf = rect_panel(
        s,
        "Adjustable shelf, door compartment",
        stocks.ply18,
        PanelPlane::Xy,
        [
            "#19",
            "#20",
            &format!("#{}", pin_z(LOWER_SHELF_HOLE) + PIN_D / 2.),
        ],
        shelf_size,
        GrainAxis::U,
    )?;
    let mut upper = vec![];
    for (i, hole) in UPPER_SHELF_HOLES.iter().enumerate() {
        upper.push(rect_panel(
            s,
            &format!("Adjustable shelf, open shelving {}", i + 1),
            stocks.ply18,
            PanelPlane::Xy,
            [
                "#19",
                "#20",
                &format!(
                    "=$door_zone + $drawer_zone + {} mm",
                    upper_hole(*hole) + PIN_D / 2.
                ),
            ],
            shelf_size,
            GrainAxis::U,
        )?);
    }
    let shelves = [lower_shelf, upper[0], upper[1]];
    let pin_part = primitive(
        s,
        "Shelf support pin Ø5 × 16",
        PrimitiveParameters::Cylinder {
            radius: length(PIN_D / 2.),
            height: length(16.),
            frame: PrimitiveFrame {
                origin: [
                    length(CARCASS_T - 8.),
                    length(SYSTEM32_SETBACK),
                    length(pin_z(LOWER_SHELF_HOLE)),
                ],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
        },
    )?;
    tag(s, &[pin_part], PURCHASED)?;
    let mut pin_copies = vec![];
    let lifts = [
        "#0".to_string(),
        format!(
            "=$door_zone + $drawer_zone + {} mm",
            upper_hole(UPPER_SHELF_HOLES[0]) - pin_z(LOWER_SHELF_HOLE)
        ),
        format!(
            "=$door_zone + $drawer_zone + {} mm",
            upper_hole(UPPER_SHELF_HOLES[1]) - pin_z(LOWER_SHELF_HOLE)
        ),
    ];
    let rear = format!("=$cabinet_depth - {} mm", 2. * SYSTEM32_SETBACK);
    let across = format!("=$cabinet_width - {} mm", 2. * CARCASS_T);
    for (si, lift) in lifts.iter().enumerate() {
        for (xi, x) in ["#0", across.as_str()].iter().enumerate() {
            for (yi, y) in ["#0", rear.as_str()].iter().enumerate() {
                if si == 0 && xi == 0 && yi == 0 {
                    continue;
                }
                let name = format!("Shelf support pin {}", pin_copies.len() + 2);
                let t = point(s, [x, y, lift])?;
                pin_copies.push(place(s, pin_part, &name, t)?);
            }
        }
    }

    milestone(s, "Adjustable shelves and pins")?;
    // Library hardware, imported as pinned symbol definitions.
    let symbols = [
        import(s, &hardware.runner_cabinet)?,
        import(s, &hardware.runner_drawer)?,
        import(s, &hardware.hinge)?,
        import(s, &hardware.plate)?,
    ];

    // Doors: full overlay, 3 mm gaps, hinged on the outer sides.
    let door_size = ["=$cabinet_width / 2 - 3.5 mm", "=$door_zone + 23.5 mm"];
    let mut doors = vec![];
    for (name, x, hinge_x, plate_x, mirrored) in [
        ("Left door", "#2", "#24.5", "#18", false),
        (
            "Right door",
            "=$cabinet_width / 2 + 1.5 mm",
            "=$cabinet_width - 24.5 mm",
            "=$cabinet_width - 18 mm",
            true,
        ),
    ] {
        let panel = rect_panel(
            s,
            name,
            stocks.ply18,
            PanelPlane::Xz,
            [x, "#0", "#2"],
            door_size,
            GrainAxis::V,
        )?;
        let cup_u = if mirrored {
            format!("=$cabinet_width / 2 - {} mm", 3.5 + CUP_EDGE)
        } else {
            format!("#{CUP_EDGE}")
        };
        let cup_v = [
            format!("#{CUP_END}"),
            format!("=$door_zone + {} mm", 23.5 - CUP_END),
        ];
        let mut cups = vec![];
        let mut hinges = vec![];
        let mut plates = vec![];
        for (h, (v, z)) in cup_v.iter().zip(&hinge_z).enumerate() {
            let which = if h == 0 { "lower" } else { "upper" };
            cups.push(hardware_at(
                s,
                &format!("{name} hinge boring, {which}"),
                cup_pattern,
                [&cup_u, v],
                if mirrored { 2 } else { 0 },
                MachiningFace::Bottom,
                vec![panel],
            )?);
            hinges.push(place_symbol(
                s,
                symbols[2],
                &format!("{name} hinge, {which}"),
                [hinge_x, "#0", z],
                mirrored,
            )?);
            plates.push(place_symbol(
                s,
                symbols[3],
                &format!("{name} hinge plate, {which}"),
                [plate_x, &setback, z],
                mirrored,
            )?);
        }
        let leaf = place(s, panel, &format!("{name} leaf"), zero())?;
        let (asm, root) = assembly(s, name, vec![leaf, hinges[0], hinges[1]], vec![panel])?;
        // The door swings outwards about a vertical axis on the carcass
        // front corner of its hinge side.
        let (pivot_x, u, v) = if mirrored {
            ("=$cabinet_width", [1., 0., 0.], [0., 1., 0.])
        } else {
            ("#0", [1., 0., 0.], [0., -1., 0.])
        };
        let anchor = frame(s, [pivot_x, "#0", "#0"], u, v)?;
        let joint = JointRequest::new(
            root,
            JointParameters {
                kind: JointKind::Revolute,
                parent: None,
                moving: anchor.clone(),
                target: anchor,
                values: JointValues::default(),
                limits: JointLimits {
                    translation: Default::default(),
                    rotation: Some(JointLimit {
                        min: angle(0.),
                        max: dim_in(s.document(), "=$door_max_angle", "deg")?,
                    }),
                },
            },
            vec![],
        );
        let prepared = model::prepare_joint(s, s.revision(), actor(), &joint)?;
        checked(s, prepared.prepared)?;
        doors.push(Door {
            panel,
            hinges: [hinges[0], hinges[1]],
            plates: [plates[0], plates[1]],
            cups: [cups[0], cups[1]],
            assembly: asm,
            root,
            hinge_joint: joint.driver,
        });
    }

    milestone(s, "Doors, hinges and borings")?;
    // Drawers.
    let mut drawers = vec![];
    for i in 0..2 {
        let half = if i == 0 { "" } else { " + $drawer_zone / 2" };
        let zb = format!("=$door_zone{half} + {} mm", 36. + BOX_LIFT);
        let n = i + 1;
        let front = rect_panel(
            s,
            &format!("Drawer {n} front"),
            stocks.ply18,
            PanelPlane::Xz,
            [
                "#2",
                "#0",
                &format!(
                    "=$door_zone{half} + {} mm",
                    if i == 0 { 28.5 } else { 37.5 }
                ),
            ],
            ["=$cabinet_width - 4 mm", "=$drawer_zone / 2 + 6 mm"],
            GrainAxis::U,
        )?;
        let inner = format!("=$cabinet_width - {} mm", 2. * SIDE_GAP);
        let right_x = format!("=$cabinet_width - {} mm", SIDE_GAP + BOX_T);
        let gap = format!("#{SIDE_GAP}");
        let height = format!("#{BOX_H}");
        let box_front = rect_panel(
            s,
            &format!("Drawer {n} box front"),
            stocks.ply12,
            PanelPlane::Xz,
            [&gap, &format!("#{BOX_T}"), &zb],
            [&inner, &height],
            GrainAxis::U,
        )?;
        let back = rect_panel(
            s,
            &format!("Drawer {n} box back"),
            stocks.ply12,
            PanelPlane::Xz,
            [&gap, "#450", &zb],
            [&inner, &height],
            GrainAxis::U,
        )?;
        let left_side = rect_panel(
            s,
            &format!("Drawer {n} box left side"),
            stocks.ply12,
            PanelPlane::Yz,
            [&gap, "#0", &zb],
            ["#450", &height],
            GrainAxis::U,
        )?;
        let right_side = rect_panel(
            s,
            &format!("Drawer {n} box right side"),
            stocks.ply12,
            PanelPlane::Yz,
            [&right_x, "#0", &zb],
            ["#450", &height],
            GrainAxis::U,
        )?;
        let joint = model::prepare_panel_joint(
            s,
            s.revision(),
            actor(),
            &format!("Drawer {n} box fingers"),
            &PanelJointDefinition {
                kind: PanelJointKind::Finger {
                    count: 5,
                    fit: length(0.2),
                },
                pairs: [box_front, back]
                    .iter()
                    .flat_map(|a| {
                        [
                            PanelJointPair {
                                a: *a,
                                b: left_side,
                            },
                            PanelJointPair {
                                a: *a,
                                b: right_side,
                            },
                        ]
                    })
                    .collect(),
            },
        )?;
        let fingers = joint.joint;
        checked(s, joint.model.prepared)?;
        let groove = machining(
            s,
            &format!("Drawer {n} bottom groove"),
            MachiningKind::Groove {
                along: PanelAxis::U,
                offset: length(8.),
                width: length(6.5),
                depth: length(6.),
            },
            MachiningFace::Inside,
            vec![box_front, back, left_side, right_side],
        )?;
        let bottom = rect_panel(
            s,
            &format!("Drawer {n} box bottom"),
            stocks.ply6,
            PanelPlane::Xy,
            [
                &format!("#{}", SIDE_GAP + 6.5),
                "#6.5",
                &format!("=$door_zone{half} + {} mm", 36. + BOX_LIFT + 8.),
            ],
            [
                &format!("=$cabinet_width - {} mm", 2. * (SIDE_GAP + 6.5)),
                "#437",
            ],
            GrainAxis::U,
        )?;
        // Runners centred on the box sides, flush with the carcass front.
        let zr = format!("=$door_zone{half} + {} mm", 36. + BOX_LIFT + RUNNER_LIFT);
        let zmid = format!(
            "=$door_zone{half} + {} mm",
            36. + BOX_LIFT + RUNNER_LIFT + RUNNER_H / 2.
        );
        let runner_cabinet = [
            place_symbol(
                s,
                symbols[0],
                &format!("Drawer {n} runner, left cabinet member"),
                [&format!("#{CARCASS_T}"), "#0", &zr],
                false,
            )?,
            place_symbol(
                s,
                symbols[0],
                &format!("Drawer {n} runner, right cabinet member"),
                [
                    &format!("=$cabinet_width - {} mm", CARCASS_T + RUNNER_T),
                    "#0",
                    &zr,
                ],
                false,
            )?,
        ];
        let runner_drawer = [
            place_symbol(
                s,
                symbols[1],
                &format!("Drawer {n} runner, left drawer member"),
                [&format!("#{}", CARCASS_T + RUNNER_T), "#0", &zr],
                false,
            )?,
            place_symbol(
                s,
                symbols[1],
                &format!("Drawer {n} runner, right drawer member"),
                [&format!("=$cabinet_width - {SIDE_GAP} mm"), "#0", &zr],
                false,
            )?,
        ];
        let cabinet_holes = hardware_at(
            s,
            &format!("Drawer {n} runner screws, carcass sides"),
            cabinet_screws,
            ["#0", &zmid],
            0,
            MachiningFace::Inside,
            vec![left, right],
        )?;
        let box_mid = format!("#{}", RUNNER_LIFT + RUNNER_H / 2.);
        let drawer_holes = [
            hardware_at(
                s,
                &format!("Drawer {n} runner screws, left box side"),
                drawer_screws,
                ["#0", &box_mid],
                0,
                MachiningFace::Bottom,
                vec![left_side],
            )?,
            hardware_at(
                s,
                &format!("Drawer {n} runner screws, right box side"),
                drawer_screws,
                ["#0", &box_mid],
                0,
                MachiningFace::Top,
                vec![right_side],
            )?,
        ];
        let parts = [front, box_front, back, left_side, right_side, bottom];
        let mut members = vec![];
        for part in parts {
            let name = s.document().get(part).ok_or("part")?.name.clone();
            members.push(place(s, part, &format!("{name} member"), zero())?);
        }
        members.extend(runner_drawer);
        let (asm, root) = assembly(s, &format!("Drawer {n}"), members, parts.to_vec())?;
        // Slides out along −Y (the frame's normal is u × v = −Y).
        let anchor = frame(s, ["#0", "#0", "#0"], [1., 0., 0.], [0., 0., 1.])?;
        let joint = JointRequest::new(
            root,
            JointParameters {
                kind: JointKind::Slider,
                parent: None,
                moving: anchor.clone(),
                target: anchor,
                values: JointValues::default(),
                limits: JointLimits {
                    translation: [
                        None,
                        None,
                        Some(JointLimit {
                            min: length(0.),
                            max: dim(s, "=$drawer_travel")?,
                        }),
                    ],
                    rotation: None,
                },
            },
            vec![],
        );
        let prepared = model::prepare_joint(s, s.revision(), actor(), &joint)?;
        checked(s, prepared.prepared)?;
        milestone(s, &format!("Drawer {n}"))?;
        drawers.push(Drawer {
            front,
            box_front,
            back,
            left: left_side,
            right: right_side,
            bottom,
            fingers,
            groove,
            runner_cabinet,
            runner_drawer,
            cabinet_screws: cabinet_holes,
            drawer_screws: drawer_holes,
            assembly: asm,
            root,
            slider: joint.driver,
        });
    }
    let doors: [Door; 2] = doors.try_into().map_err(|_| "two doors")?;
    let drawers: [Drawer; 2] = drawers.try_into().map_err(|_| "two drawers")?;
    Ok(Cabinet {
        session,
        params,
        stocks,
        carcass: carcass_id,
        panels,
        dominos,
        lower_pins,
        upper_pins,
        plate_holes,
        shelves,
        pin_part,
        pin_copies,
        doors,
        drawers,
        symbols,
        patterns,
        poses: BTreeMap::new(),
        book: EntityId::nil(),
        pages: vec![],
        dimensions: [EntityId::nil(); 3],
    })
}

// ---------------------------------------------------------------------------
// Movement: named poses
// ---------------------------------------------------------------------------

pub const POSE_CLOSED: &str = "Closed";
pub const POSE_DOORS: &str = "Doors open 90°";
pub const POSE_DRAWERS: &str = "Drawers open";
pub const POSE_ALL: &str = "Doors and drawers open";

fn add_poses(c: &mut Cabinet) -> Result<(), Error> {
    let s = &mut c.session;
    for (name, door, drawer) in [
        (POSE_CLOSED, "#0", "#0"),
        (POSE_DOORS, "#90", "#0"),
        (POSE_DRAWERS, "#0", "=$drawer_travel"),
        (POSE_ALL, "#90", "=$drawer_travel"),
    ] {
        let mut values = vec![];
        for d in &c.doors {
            values.push(JointPoseValue {
                joint: d.hinge_joint,
                values: JointValues {
                    rotation: dim_in(s.document(), door, "deg")?,
                    ..Default::default()
                },
            });
        }
        for d in &c.drawers {
            values.push(JointPoseValue {
                joint: d.slider,
                values: JointValues {
                    translation: [length(0.), length(0.), dim(s, drawer)?],
                    ..Default::default()
                },
            });
        }
        let pose = NamedPoseRequest::new(name, values);
        let prepared = model::prepare_named_pose(s, s.revision(), actor(), &pose)?;
        checked(s, prepared.prepared)?;
        c.poses.insert(name, pose.pose);
    }
    Ok(())
}
/// Apply a saved pose as one transaction.
pub fn apply_pose(c: &mut Cabinet, name: &str) -> Result<(), Error> {
    let pose = *c.poses.get(name).ok_or("unknown pose")?;
    let s = &mut c.session;
    let prepared = model::prepare_apply_pose(s, s.revision(), actor(), pose)?;
    checked(s, prepared.prepared)
}

// ---------------------------------------------------------------------------
// Schedules and drawings
// ---------------------------------------------------------------------------

/// The parts and hardware schedule: library hardware grouped by its pinned
/// symbol, plus pins and tenons, each with its mounting drilling.
pub fn hardware_schedule(c: &Cabinet) -> Result<Schedule, Error> {
    let doc = c.session.document();
    let revision = c.session.revision();
    let mut rows = vec![];
    let drilling = [
        "Side: 4 × Ø3 × 10 at 37/165/293/421",
        "Box side: 4 × Ø2.5 × 8, same spacing",
        "Door: cup Ø35 × 13 + 2 × Ø2.5 × 10",
        "Side: 2 × Ø5 × 12, 37 mm, 32 apart",
    ];
    let spec = [
        "450 × 45 × 6.35 mm, full extension",
        "450 × 45 × 6.35 mm, full extension",
        "35 mm cup, 110°, full overlay",
        "Cruciform, 0 mm, 2 screws",
    ];
    for (i, definition) in c.symbols.iter().enumerate() {
        let def = symbols::read_definition(doc, *definition)?;
        let count = symbols::instances(doc, *definition)?.len();
        rows.push(vec![
            def.name.clone(),
            count.to_string(),
            format!("library rev {}", def.revision),
            spec[i].into(),
            drilling[i].into(),
        ]);
    }
    rows.push(vec![
        "Shelf support pin Ø5 × 16".into(),
        (1 + c.pin_copies.len()).to_string(),
        "project Part".into(),
        "Steel pin, 8 mm in the hole".into(),
        "System 32: Ø5 × 10, 37 mm, pitch 32".into(),
    ]);
    let joint = model::read_panel_joint(doc, c.dominos)?;
    let tenons = joint.pairs.len() as u32 * domino_spec().count;
    rows.push(vec![
        "Domino tenon 8 × 22 × 40".into(),
        tenons.to_string(),
        "project Parts".into(),
        "Festool DOMINO 8 × 22 × 40, 3 per joint".into(),
        "Mortises 15 face / 28 end".into(),
    ]);
    Ok(Schedule {
        kind: "hardware",
        title: "Hardware schedule".into(),
        revision,
        source: project_file::ContentHash::document(doc)?,
        notes: vec![
            "Library hardware is pinned; update only through an explicit symbol update".into(),
            "Generic example dimensions, not a product compatibility claim".into(),
        ],
        columns: [
            "Item",
            "Qty",
            "Source",
            "Specification",
            "Mounting drilling",
        ]
        .map(String::from)
        .to_vec(),
        rows,
    })
}

/// A planar face of a standalone Part whose outward normal is `normal` and
/// whose plane passes through `at` along it.
pub fn face(
    model: &Model,
    entity: EntityId,
    normal: [f64; 3],
    at: f64,
) -> Result<GeometryReference, Error> {
    let feature = model.snapshot().bodies[&entity];
    let mut topology = model.snapshot().shapes[&feature].topology.clone();
    topology.owner = entity;
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
        source: entity,
        intent: GeometryIntent::capture(&topology, element.key)?,
    })
}
fn dimension(
    view: EntityId,
    a: GeometryReference,
    b: GeometryReference,
    direction: LinearDirection,
    offset: f64,
) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Dimension {
            view,
            measure: Box::new(DimensionMeasure::Linear { a, b, direction }),
            offset_mm: offset,
            precision: 0,
            reference: false,
        },
    }
}
#[allow(clippy::too_many_arguments)]
fn view(
    name: &str,
    entities: Vec<EntityId>,
    origin: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    position: [f64; 2],
    denominator: u32,
    kind: ViewKind,
) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model { entities },
        frame: ReferenceFrame {
            origin_mm: origin,
            u,
            v,
        },
        kind,
        position_mm: position,
        scale: Scale {
            numerator: 1,
            denominator,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.05,
        guide: None,
    }
}
fn section(depth: Option<f64>) -> ViewKind {
    ViewKind::Section {
        offset_mm: 0.,
        range: depth.map_or(SectionRange::Unlimited, |depth_mm| SectionRange::Depth {
            depth_mm,
        }),
        window: Default::default(),
    }
}
fn a3(number: &str, title: &str, denominator: u32) -> Page {
    let mut page = Page::new(number);
    page.settings.paper = PaperSize::A3;
    page.settings.scale = Scale {
        numerator: 1,
        denominator,
    };
    page.info = DrawingInfo {
        title: title.into(),
        author: "MultiCAD".into(),
        project: CABINET_NAME.into(),
        revision: "A".into(),
        checker: "Pending human review".into(),
    };
    page
}
fn note(position: [f64; 2], text: &str) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: position,
            text: text.into(),
        },
    }
}
fn table(kind: AnnotationKind) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind,
    }
}

/// Every physical top-level item: standalone Parts that are not assembly
/// definitions, and placed root Instances.
pub fn physical_roots(c: &Cabinet, model: &Model) -> Result<Vec<EntityId>, Error> {
    let doc = c.session.document();
    let definitions = model::assembly_definition_parts(doc)?;
    let members = model::assembly_member_instances(doc)?;
    let mut out: Vec<EntityId> = model
        .snapshot()
        .bodies
        .keys()
        .filter(|p| !definitions.contains(p) && !model.snapshot().consumed_bodies.contains(p))
        .copied()
        .collect();
    out.extend(
        doc.entities()
            .filter(|e| matches!(e.data, EntityData::Instance { .. }) && !members.contains(&e.id))
            .map(|e| e.id),
    );
    Ok(out)
}

fn add_drawings(c: &mut Cabinet) -> Result<(), Error> {
    let model = milestone(&c.session, "Drawings input")?;
    let everything = physical_roots(c, &model)?;
    let left = c.role(PanelRole::LeftSide);
    let right = c.role(PanelRole::RightSide);
    let divider = c.role(PanelRole::Shelf { index: 0 });
    let tenon = model::joint_tenons(c.session.document(), c.dominos)
        .into_iter()
        .find(|(pair, _, _)| *pair == 4)
        .map(|(_, _, part)| part)
        .ok_or("divider tenon Part")?;

    // C-101: assembly, three views and the overall dimensions.
    let mut assembly = a3("C-101", "Cabinet assembly", 15);
    let front = view(
        "Front",
        everything.clone(),
        [400., 0., 1000.],
        [1., 0., 0.],
        [0., 0., 1.],
        [85., 125.],
        15,
        ViewKind::Projection,
    );
    let side = view(
        "Right side",
        everything.clone(),
        [800., 250., 1000.],
        [0., 1., 0.],
        [0., 0., 1.],
        [175., 125.],
        15,
        ViewKind::Projection,
    );
    let top = view(
        "Top",
        everything.clone(),
        [400., 250., 2000.],
        [1., 0., 0.],
        [0., 1., 0.],
        [260., 70.],
        15,
        ViewKind::Projection,
    );
    let width = dimension(
        front.id,
        face(&model, left, [-1., 0., 0.], 0.)?,
        face(&model, right, [1., 0., 0.], 800.)?,
        LinearDirection::Horizontal,
        -12.,
    );
    let height = dimension(
        front.id,
        face(&model, left, [0., 0., -1.], 0.)?,
        face(&model, left, [0., 0., 1.], 2000.)?,
        LinearDirection::Vertical,
        -14.,
    );
    let depth = dimension(
        side.id,
        face(&model, left, [0., -1., 0.], 0.)?,
        face(&model, left, [0., 1., 0.], 500.)?,
        LinearDirection::Horizontal,
        -12.,
    );
    c.dimensions = [width.id, height.id, depth.id];
    assembly.views = vec![front, side, top];
    assembly.annotations = vec![
        width,
        height,
        depth,
        note(
            [225., 120.],
            "Bottom to top: door compartment (left- and right-hand doors, 1 shelf), two drawers, open shelving (2 shelves)",
        ),
        note(
            [225., 128.],
            "Birch ply 18 carcass, Domino 8 × 22 × 40 joints; finger-jointed birch ply 12 drawer boxes",
        ),
        note(
            [225., 136.],
            "Poses: Closed, Doors open 90°, Drawers open, Doors and drawers open (Model → assembly poses)",
        ),
        note(
            [225., 144.],
            "Door axes: vertical, at the carcass front outer corners. Drawer axis: −Y, 0–450 mm",
        ),
        note(
            [225., 152.],
            "Geometry records only: not a structural, load or product-compatibility certification",
        ),
    ];
    let (book, prepared) = prepare_create(
        &c.session,
        c.session.revision(),
        actor(),
        "Cabinet drawings",
        Some(assembly.clone()),
    )?;
    checked(&mut c.session, prepared)?;
    c.book = book;
    c.pages.push(assembly.id);

    // C-102: cutaway sections and joint details.
    let mut joints = a3("C-102", "Cutaway sections and joint details", 10);
    let z_divider = 800. + CARCASS_T + CARCASS_T / 2.;
    joints.views = vec![
        view(
            "Section A-A: lower divider with Domino tenons",
            everything.clone(),
            [400., 250., z_divider],
            [1., 0., 0.],
            [0., 1., 0.],
            [70., 60.],
            10,
            section(Some(300.)),
        ),
        view(
            "Section B-B: cutaway at the cabinet centre, looking at the left side",
            everything.clone(),
            [400., 250., 1000.],
            [0., 1., 0.],
            [0., 0., 1.],
            [160., 140.],
            15,
            section(None),
        ),
        view(
            "Detail C: Domino joint, left side to lower divider",
            vec![left, divider, tenon],
            [9., 250., z_divider],
            [1., 0., 0.],
            [0., 1., 0.],
            [225., 75.],
            5,
            ViewKind::Section {
                offset_mm: 0.,
                range: SectionRange::Depth { depth_mm: 30. },
                window: SectionWindow {
                    u_mm: Some([-20., 150.]),
                    v_mm: Some([-250., 250.]),
                },
            },
        ),
        view(
            "Detail D: drawer 1 box, finger joints (top)",
            vec![c.drawers[0].root],
            [400., 225., 1100.],
            [1., 0., 0.],
            [0., 1., 0.],
            [330., 75.],
            5,
            ViewKind::Projection,
        ),
    ];
    joints.annotations = vec![
        note(
            [15., 272.],
            "Section A-A cuts the lower divider at mid thickness: three Domino tenons per side",
        ),
        note(
            [15., 279.],
            "Finger joints: 5 fingers, 0.2 mm fit; 6 mm bottom in a 6.5 × 6 mm groove 8 mm up",
        ),
    ];

    // C-103: drilling details of the side, door and drawer box side.
    let mut drilling = a3("C-103", "Drilling details", 10);
    drilling.views = vec![
        view(
            "Left side, inner face",
            vec![left],
            [18., 250., 1000.],
            [0., 1., 0.],
            [0., 0., 1.],
            [70., 140.],
            10,
            ViewKind::Projection,
        ),
        view(
            "Left door with hinges, inner face",
            vec![c.doors[0].root],
            [200., 0., 414.],
            [-1., 0., 0.],
            [0., 0., 1.],
            [200., 120.],
            5,
            ViewKind::Projection,
        ),
        view(
            "Drawer 1 box left side, outer face (runner cut away)",
            vec![c.drawers[0].root],
            [30.6, 225., 931.],
            [0., -1., 0.],
            [0., 0., 1.],
            [330., 60.],
            5,
            section(Some(12.)),
        ),
    ];
    drilling.annotations = vec![
        note(
            [250., 220.],
            "Side: System 32 rows Ø5 × 10 at 37 mm from front and rear, 32 mm pitch",
        ),
        note(
            [250., 227.],
            "Side: hinge plates 2 × Ø5 × 12; runner screws 4 × Ø3 × 10 per drawer",
        ),
        note(
            [250., 234.],
            "Door: Ø35 × 13 cups 22.5 mm from the hinge edge, 100 mm from the ends",
        ),
        note([250., 241.], "Box side: runner screws 4 × Ø2.5 × 8"),
    ];
    for page in [joints, drilling] {
        c.pages.push(page.id);
        let prepared = prepare(
            &c.session,
            c.session.revision(),
            actor(),
            book,
            PageAction::Add(page),
        )?;
        checked(&mut c.session, prepared)?;
    }
    refresh_tables(c)
}

/// Rows per table page, so a table stays above the title block.
pub const TABLE_ROWS: usize = 36;
/// Split a schedule into page-sized parts with numbered titles.
pub fn chunks(schedule: &Schedule) -> Vec<Schedule> {
    let parts: Vec<_> = schedule.rows.chunks(TABLE_ROWS).collect();
    let n = parts.len().max(1);
    if n == 1 {
        return vec![schedule.clone()];
    }
    parts
        .iter()
        .enumerate()
        .map(|(i, rows)| Schedule {
            title: format!("{} ({}/{n})", schedule.title, i + 1),
            rows: rows.to_vec(),
            ..schedule.clone()
        })
        .collect()
}
/// Every schedule the drawing book shows, built from the current model.
pub fn tables(c: &Cabinet) -> Result<Vec<Schedule>, Error> {
    let model = milestone(&c.session, "Schedules")?;
    let (doc, revision) = (c.session.document(), c.session.revision());
    let build = |kind| schedules::build(&model, doc, revision, ScheduleSpec::new(kind));
    let mut out = vec![hardware_schedule(c)?];
    for kind in [
        ScheduleKind::Bom,
        ScheduleKind::PanelCutList,
        ScheduleKind::Machining,
    ] {
        out.extend(chunks(&build(kind)?));
    }
    Ok(out)
}

/// Replace the schedule pages (C-104 onwards) with tables built from the
/// current model; drawing pages C-101 to C-103 are left alone.
pub fn refresh_tables(c: &mut Cabinet) -> Result<(), Error> {
    let tables = tables(c)?;
    let book = multicad_core::drawings::read(c.session.document(), c.book)?;
    for page in book.pages.iter().skip(3) {
        let prepared = prepare(
            &c.session,
            c.session.revision(),
            actor(),
            c.book,
            PageAction::Remove(page.id),
        )?;
        checked(&mut c.session, prepared)?;
    }
    c.pages.truncate(3);
    for (i, t) in tables.iter().enumerate() {
        let mut page = a3(&format!("C-{}", 104 + i), &t.title, 10);
        page.annotations = vec![table(t.table([15., 20.]))];
        c.pages.push(page.id);
        let prepared = prepare(
            &c.session,
            c.session.revision(),
            actor(),
            c.book,
            PageAction::Add(page),
        )?;
        checked(&mut c.session, prepared)?;
    }
    Ok(())
}
/// Placed tables whose cells differ from the current schedules.
pub fn stale_tables(c: &Cabinet) -> Result<Vec<String>, Error> {
    let fresh = tables(c)?;
    let book = multicad_core::drawings::read(c.session.document(), c.book)?;
    let placed: Vec<_> = book
        .pages
        .iter()
        .flat_map(|p| p.annotations.iter())
        .filter(|a| matches!(a.kind, AnnotationKind::Table { .. }))
        .collect();
    let mut stale = vec![];
    if placed.len() != fresh.len() {
        stale.push(format!(
            "{} tables placed, {} expected",
            placed.len(),
            fresh.len()
        ));
    }
    for (a, t) in placed.iter().zip(&fresh) {
        if let Err(e) = t.check_table(&a.kind) {
            stale.push(e);
        }
    }
    Ok(stale)
}

/// Every page evaluated and checked, with each page's layout conflicts.
pub fn page_report(session: &Session, book: EntityId) -> Result<Vec<(String, Vec<String>)>, Error> {
    page_report_inner(session, book).map_err(|e| {
        let text = e.to_string();
        let named: Vec<String> = session
            .document()
            .entities()
            .filter(|x| text.contains(&x.id.to_string()))
            .map(|x| format!("{:?} {}", std::mem::discriminant(&x.data), x.name))
            .collect();
        format!("{text} ({})", named.join(", ")).into()
    })
}
fn page_report_inner(
    session: &Session,
    book: EntityId,
) -> Result<Vec<(String, Vec<String>)>, Error> {
    let evaluation = Compiler::default()
        .compile(session.document(), session.revision())?
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )
        .map_err(|e| {
            let text = e.to_string();
            let named = session
                .document()
                .entities()
                .find(|x| text.contains(&x.id.to_string()))
                .map(|x| x.name.clone())
                .unwrap_or_default();
            format!("{text} ({named})")
        })?;
    let pages = multicad_core::drawings::read(session.document(), book)?.pages;
    let total = pages.len();
    let mut out = vec![];
    for (i, page) in pages.iter().enumerate() {
        evaluation.checked_page(session.document(), session.revision(), book, page.id)?;
        out.push((
            page.number.clone(),
            page_conflicts(page, i, total, &evaluation)?,
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Preview: a small software rendering of the native solids
// ---------------------------------------------------------------------------

fn color_for(name: &str) -> [f32; 3] {
    let n = name.to_lowercase();
    if n.contains("runner") || n.contains("hinge") || n.contains("pin") || n.contains("plate") {
        [0.62, 0.64, 0.68]
    } else if n.contains("back") && n.contains("cabinet") {
        [0.55, 0.42, 0.32]
    } else if n.contains("tenon") {
        [0.85, 0.72, 0.45]
    } else if n.contains("door") || n.contains("front") && !n.contains("box") {
        [0.80, 0.66, 0.48]
    } else if n.contains("drawer") {
        [0.90, 0.80, 0.62]
    } else {
        [0.86, 0.75, 0.56]
    }
}

/// Render every physical leaf with a z-buffer from an isometric front-left
/// view, flat shaded, into a checked PNG file preview.
/// `spread` lays physical roots out side by side along X (library items
/// all sit at their local origin).
pub fn render_preview(
    document: &Document,
    model: &Model,
    spread: bool,
) -> Result<project_file::info::PreviewImage, Error> {
    use multicad_model::export::mesh::tessellate;
    use multicad_ui::egui::{Color32, ColorImage};
    let (w, h) = (480usize, 640usize);
    // View basis: looking from the front left, slightly above.
    let yaw: f64 = (-35f64).to_radians();
    let pitch: f64 = 20f64.to_radians();
    let right = [yaw.cos(), yaw.sin(), 0.];
    let fwd = [
        -yaw.sin() * pitch.cos(),
        yaw.cos() * pitch.cos(),
        -pitch.sin(),
    ];
    let up = [
        right[1] * fwd[2] - right[2] * fwd[1],
        right[2] * fwd[0] - right[0] * fwd[2],
        right[0] * fwd[1] - right[1] * fwd[0],
    ];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let light = [-0.4, -0.7, 0.6];
    let ln = dot(light, light).sqrt();
    let mut tris = vec![];
    let mut roots: Vec<EntityId> = vec![];
    for (path, leaf) in &model.snapshot().leaf_occurrences {
        let index = roots
            .iter()
            .position(|r| *r == path.root)
            .unwrap_or_else(|| {
                roots.push(path.root);
                roots.len() - 1
            });
        let dx = if spread { index as f64 * 60. } else { 0. };
        let Some(solid) = model.leaf_solid(path) else {
            continue;
        };
        let name = document
            .get(leaf.component)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let mesh = tessellate(solid, &Default::default(), &name)?;
        let color = color_for(&name);
        for t in &mesh.triangles {
            let p = t.map(|i| {
                let v = mesh.vertices[i as usize];
                [v[0] + dx, v[1], v[2]]
            });
            tris.push((p, color));
        }
    }
    let project = |p: [f64; 3]| (dot(p, right), dot(p, up), dot(p, fwd));
    let (mut min, mut max) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (p, _) in &tris {
        for q in p {
            let (x, y, _) = project(*q);
            min = [min[0].min(x), min[1].min(y)];
            max = [max[0].max(x), max[1].max(y)];
        }
    }
    let scale = ((w as f64 - 40.) / (max[0] - min[0])).min((h as f64 - 40.) / (max[1] - min[1]));
    let ox = (w as f64 - (max[0] - min[0]) * scale) / 2.;
    let oy = (h as f64 - (max[1] - min[1]) * scale) / 2.;
    let mut depth = vec![f64::MAX; w * h];
    let mut pixels = vec![Color32::from_rgb(244, 244, 242); w * h];
    for (p, color) in &tris {
        let a = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let b = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let nl = dot(n, n).sqrt();
        if nl < 1e-12 {
            continue;
        }
        let shade = 0.45 + 0.55 * (dot(n, light) / (nl * ln)).abs();
        let rgb = color.map(|c| (c as f64 * shade * 255.).clamp(0., 255.) as u8);
        let v = p.map(|q| {
            let (x, y, z) = project(q);
            (
                ox + (x - min[0]) * scale,
                h as f64 - (oy + (y - min[1]) * scale),
                z,
            )
        });
        let x0 = v
            .iter()
            .map(|q| q.0)
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let x1 = v
            .iter()
            .map(|q| q.0)
            .fold(f64::MIN, f64::max)
            .ceil()
            .min(w as f64 - 1.) as usize;
        let y0 = v
            .iter()
            .map(|q| q.1)
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let y1 = v
            .iter()
            .map(|q| q.1)
            .fold(f64::MIN, f64::max)
            .ceil()
            .min(h as f64 - 1.) as usize;
        let area = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
        if area.abs() < 1e-9 {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((v[1].0 - px) * (v[2].1 - py) - (v[2].0 - px) * (v[1].1 - py)) / area;
                let w1 = ((v[2].0 - px) * (v[0].1 - py) - (v[0].0 - px) * (v[2].1 - py)) / area;
                let w2 = 1. - w0 - w1;
                if w0 < 0. || w1 < 0. || w2 < 0. {
                    continue;
                }
                let z = w0 * v[0].2 + w1 * v[1].2 + w2 * v[2].2;
                let i = y * w + x;
                if z < depth[i] {
                    depth[i] = z;
                    pixels[i] = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                }
            }
        }
    }
    let image = ColorImage::new([w, h], pixels);
    Ok(multicad_ui::preview_capture::preview_from(&image)?)
}

fn set_info(
    session: &mut Session,
    name: &str,
    description: &str,
    preview: project_file::info::PreviewImage,
) -> Result<(), Error> {
    let info = FileInfo {
        name: name.into(),
        description: description.into(),
        preview: Some(preview),
        capture_on_save: true,
    };
    let edits = project_file::info::edits(session.document(), &info)?;
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Set file information".into(),
        edits,
    })?;
    session.commit(prepared)?;
    Ok(())
}

pub const CABINET_DESCRIPTION: &str = "Editable 2000 × 800 × 500 mm birch plywood cabinet. Bottom to top: a door compartment behind a left-hand and a right-hand full-overlay door on concealed 35 mm hinges, with one adjustable shelf; two drawers on side-mount full-extension runners with finger-jointed boxes; open adjustable shelving on System 32 shelf-pin rows. The carcass is joined with Domino 8 × 22 × 40 loose tenons. Runners, hinges and plates are pinned symbols from the Cabinet hardware library. Parameters: cabinet_width, cabinet_depth, cabinet_height, door_zone, drawer_zone, door_max_angle, drawer_travel. Poses: Closed, Doors open 90°, Drawers open, Doors and drawers open. Drawings C-101 to C-106 hold the assembly, cutaway sections, joint and drilling details and the parts, hardware, cut-list and machining schedules.";
pub const LIBRARY_DESCRIPTION: &str = "Reusable cabinet hardware as pinned symbols: a 450 mm side-mount full-extension drawer runner (cabinet and drawer members, runner_length exposed), a concealed 35 mm cup hinge (110°, full overlay) and its cruciform mounting plate. Generic example dimensions, not a product compatibility claim. Matching drilling patterns are defined in the cabinet demo.";

/// The complete demo: library, cabinet, poses, drawings and file info,
/// left in the Closed pose.
pub fn build() -> Result<(Library, Cabinet), Error> {
    let mut library = build_library(450., "1")?;
    let mut cabinet = build_cabinet(&library.hardware)?;
    add_poses(&mut cabinet)?;
    add_drawings(&mut cabinet)?;
    // Preview with doors and drawers open, then save in the Closed pose.
    apply_pose(&mut cabinet, POSE_ALL)?;
    let open = milestone(&cabinet.session, "Open pose")?;
    let preview = render_preview(cabinet.session.document(), &open, false)?;
    apply_pose(&mut cabinet, POSE_CLOSED)?;
    set_info(
        &mut cabinet.session,
        CABINET_NAME,
        CABINET_DESCRIPTION,
        preview,
    )?;
    let lib_model = milestone(&library.session, "Library preview")?;
    let preview = render_preview(library.session.document(), &lib_model, true)?;
    set_info(
        &mut library.session,
        LIBRARY_NAME,
        LIBRARY_DESCRIPTION,
        preview,
    )?;
    milestone(&cabinet.session, "Final cabinet")?;
    Ok((library, cabinet))
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
fn write_new_or_replace(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let directory = path.parent().ok_or("no directory")?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    let file = file.persist(path)?;
    let mut permissions = file.metadata()?.permissions();
    #[cfg(unix)]
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o644);
    file.set_permissions(permissions)?;
    Ok(())
}

/// Build the demo and write both files into `directory`, after checking
/// that each reopens to the identical source.
pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir_all(directory)?;
    let (library, cabinet) = build()?;
    let pages = page_report(&cabinet.session, cabinet.book)?;
    for (session, file) in [
        (&library.session, LIBRARY_FILE),
        (&cabinet.session, CABINET_FILE),
    ] {
        let bytes = archive(session)?;
        let opened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )?;
        if opened.project.document != *session.document() {
            return Err(format!("{file} changed on reopen").into());
        }
        write_new_or_replace(&directory.join(file), &bytes)?;
        // Optional copy of the stored preview for visual review.
        if let Some(dump) = std::env::var_os("CABINET_DEMO_PREVIEW_DIR")
            && let Some(preview) = project_file::info::read(session.document())?.preview
        {
            let name = file.replace('/', "-");
            std::fs::write(Path::new(&dump).join(format!("{name}.png")), preview.png())?;
        }
    }
    Ok(serde_json::json!({
        "generator": "cabinet_demo",
        "files": [CABINET_FILE, LIBRARY_FILE],
        "revision": cabinet.session.revision(),
        "pages": pages.iter().map(|(n, c)| serde_json::json!({ "page": n, "conflicts": c })).collect::<Vec<_>>(),
    }))
}

#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Usage: cargo run -p multicad-app --example cabinet_demo -- DIRECTORY")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&generate(Path::new(&path))?)?
    );
    Ok(())
}
