//! The shipped C-channel front panel demo (CAD-184) in one native editable
//! project, built through the same typed transaction API the UI and MCP use:
//!
//! - a bent aluminium front panel: a flat front on XZ with a top and a bottom
//!   90° flange bent backwards (a C section), parameter-driven width, height,
//!   flange depth, inside bend radius and K-factor, and its developed blank;
//! - six openings for six placements of one illustrative 12 mm push-button
//!   Part, and a window plus four standoffs for one illustrative 16×2
//!   character display module;
//! - left and right printed PETG end parts, each sketched and extruded in
//!   its print orientation (outer face on the build plate, +Z up) and
//!   rotated into place; their plugs fit inside the C and receive M3 screws
//!   through clearance holes in both flanges;
//! - one assembly holding every Part, drawings (assembled, exploded
//!   arrangement, sheet with bend table, printed parts), export sets for the
//!   drawings, the sheet STEP and one print set per end part, and file
//!   metadata (name, description, a rendered preview).
//!
//! `cargo run -p multicad-app --example front_panel_demo -- OUTPUT.tcd [EXPORT_DIR]`
//! writes the project (the shipped copy is `examples/front-panel.tcd`) and,
//! with an export directory, every deliverable plus a manifest. The app test
//! `tests/front_panel.rs` opens the shipped file and rebuilds this project.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{
        ConstraintKind, Dimension,
        binding::{self, DimensionInput},
        source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    exchange::*,
    materials::*,
    model::{
        self, AssemblyRequest, BendDirection, BodyInput, BodyOutput, BooleanRequest,
        ComponentPlacementRequest, Cutout, CutoutFace, CutoutShape, Extent, ExtrudeParameters,
        ExtrudeRequest, Flange, FlangeKind, HardwareItem, HardwarePreset, HardwareSide,
        MountingPattern, NumericTransform, Operation, PanelPlane, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, SheetBase, SheetDefinition, SheetRequest,
        Termination, Thread, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    references::ReferenceFrame,
    sketch::Datum,
};
use multicad_kernel::Solid;
use multicad_model::{
    Compiler, Model,
    export::{Exporter, sheet_dxf::SheetDxfLayers},
    schedules::{BomOptions, Schedule, bend_schedule, bom_schedule},
};
use multicad_ui::egui::{Color32, ColorImage};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

pub type Error = Box<dyn std::error::Error>;

pub const PROJECT_NAME: &str = "C-channel front panel";
pub const FILE_NAME: &str = "front-panel.tcd";
pub const PLACED_ASSEMBLY: &str = "Assembled front panel";
pub const PLACED_SHEET: &str = "Sheet-metal front panel";
pub const PLACED_DISPLAY: &str = "16×2 display module";
pub const PLACED_ENDS: [&str; 2] = ["Left printed end plate", "Right printed end plate"];
pub const DESCRIPTION: &str = "Bent 1.5 mm aluminium front panel with a C section (top and \
bottom flanges), six push-button openings, a 16×2 display window on four standoffs and \
printed PETG left and right end parts screwed through both flanges. Edit panel_width, \
panel_height, flange_depth, bend_radius, k_factor or the button and display positions in \
Parameters; buttons and display are illustrative models, not cited hardware.";
/// The shipped project, relative to the repository root.
pub const SHIPPED: &str = "examples/front-panel.tcd";

pub const FABRICATED: &str = "fabricated";
pub const PRINTED: &str = "printed";
pub const PURCHASED: &str = "purchased";
pub const ILLUSTRATIVE: &str = "illustrative";
pub const LEFT_END: &str = "left-end";
pub const RIGHT_END: &str = "right-end";

/// Panel button grid: two rows of three.
pub const BUTTON_ROWS: usize = 2;
pub const BUTTON_COLUMNS: usize = 3;
pub const BUTTONS: usize = BUTTON_ROWS * BUTTON_COLUMNS;
/// Illustrative 16×2 character display module (typical nominal values, not
/// a cited datasheet): board, mounting-hole pitch, display frame, window.
pub const DISPLAY_BOARD: [f64; 3] = [80., 1.6, 36.];
pub const DISPLAY_HOLES: [f64; 2] = [75., 31.];
pub const DISPLAY_FRAME: [f64; 3] = [71., 7., 24.];
pub const DISPLAY_WINDOW: [f64; 2] = [72., 25.];
/// Illustrative 12 mm push button: cap, bushing and nut radii and lengths.
pub const BUTTON_CAP: [f64; 2] = [7.5, 4.];
pub const BUTTON_BUSHING_R: f64 = 6.;
pub const BUTTON_BODY_L: f64 = 20.;
pub const BUTTON_NUT: [f64; 2] = [8., 2.];
/// Printed end parts: insert pilot hole and the right part's cable hole.
pub const INSERT_PILOT_D: f64 = 4.;
pub const INSERT_DEPTH: f64 = 8.;
pub const CABLE_HOLE_D: f64 = 8.;
pub const END_SCREW_LENGTH: f64 = 8.;

/// Mounting pattern indices on the sheet.
pub const DISPLAY_STANDOFFS: usize = 0;
pub const BOTTOM_SCREWS: usize = 1;
pub const TOP_SCREWS: usize = 2;

/// Project parameters: name, expression, unit, description.
pub const PARAMETERS: [(&str, &str, &str, &str); 16] = [
    (
        "panel_width",
        "260",
        "mm",
        "Front width along X between the printed end parts",
    ),
    (
        "panel_height",
        "80",
        "mm",
        "Flat front height between the bends; the outer height adds 2 × (bend_radius + sheet_t)",
    ),
    (
        "flange_depth",
        "40",
        "mm",
        "Straight leg of the top and bottom flanges beyond each bend",
    ),
    (
        "bend_radius",
        "1.5",
        "mm",
        "Inside bend radius of both bends (overrides the stock value)",
    ),
    (
        "k_factor",
        "0.44",
        "ratio",
        "Neutral-axis K-factor of the bend allowance (overrides the stock value)",
    ),
    (
        "sheet_t",
        "1.5",
        "mm",
        "Sheet thickness used by the printed end parts and button caps; keep it equal to the \
         sheet stock thickness",
    ),
    (
        "display_x",
        "70",
        "mm",
        "Display window centre from the left panel end",
    ),
    (
        "display_standoff",
        "8",
        "mm",
        "Standoff length from the panel back face to the display board",
    ),
    (
        "button_x",
        "150",
        "mm",
        "First button column centre from the left panel end",
    ),
    ("button_pitch", "35", "mm", "Button column pitch along X"),
    (
        "button_rows",
        "30",
        "mm",
        "Distance between the two button rows",
    ),
    (
        "button_hole",
        "12.5",
        "mm",
        "Button mounting hole diameter for the 12 mm bushing",
    ),
    ("end_plate", "4", "mm", "Printed end plate thickness"),
    (
        "plug_length",
        "16",
        "mm",
        "Length of the printed plug inside the C section",
    ),
    (
        "end_clearance",
        "0.3",
        "mm",
        "Fit clearance between each printed plug and the sheet",
    ),
    (
        "end_screw_x",
        "8",
        "mm",
        "End screw position from each panel end along X",
    ),
];

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
/// A length: a plain number or a `=` expression bound to parameters.
fn mm(document: &Document, text: &str) -> Result<Dimension, Error> {
    if text.starts_with('=') {
        Ok(Dimension::Expression {
            expression: BoundExpression::bind(document, text, "mm", None)?,
        })
    } else {
        Ok(length(text.parse()?))
    }
}
fn mm3(document: &Document, v: [&str; 3]) -> Result<[Dimension; 3], Error> {
    Ok([
        mm(document, v[0])?,
        mm(document, v[1])?,
        mm(document, v[2])?,
    ])
}
/// Centre of button `index` (row-major, top row first) in panel (u, v).
pub fn button_center(index: usize) -> [String; 2] {
    let (row, column) = (index / BUTTON_COLUMNS, index % BUTTON_COLUMNS);
    let u = if column == 0 {
        "=$button_x".to_string()
    } else {
        format!("=$button_x + {column} * $button_pitch")
    };
    let v = if row == 0 {
        "=$panel_height / 2 + $button_rows / 2"
    } else {
        "=$panel_height / 2 - $button_rows / 2"
    };
    [u, v.into()]
}

// ---------------------------------------------------------------------------
// The scenario.

pub struct Scenario {
    pub session: Session,
    pub parameters: BTreeMap<String, EntityId>,
    pub aluminium: EntityId,
    pub petg: EntityId,
    pub stock: EntityId,
    pub sheet: SheetRequest,
    pub button: EntityId,
    pub buttons: Vec<EntityId>,
    pub display: EntityId,
    pub display_member: EntityId,
    /// Left, right.
    pub ends: [EntityId; 2],
    pub end_sketches: [EntityId; 2],
    pub end_members: [EntityId; 2],
    pub sheet_member: EntityId,
    /// (pattern, item, hardware Part) and its member Instance.
    pub hardware: Vec<(u32, HardwareItem, EntityId, EntityId)>,
    pub assembly: EntityId,
    pub root: EntityId,
    pub book: EntityId,
    pub pages: [EntityId; 4],
    pub drawing_set: EntityId,
    pub sheet_set: EntityId,
    /// Left, right.
    pub print_sets: [EntityId; 2],
}
impl Scenario {
    pub fn parameter(&self, name: &str) -> EntityId {
        self.parameters[name]
    }
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
fn primitive(session: &mut Session, request: &PrimitiveRequest) -> Result<BodyInput, Error> {
    let prepared = model::prepare_primitive(session, session.revision(), actor(), request)?;
    checked(session, prepared.prepared)?;
    Ok(BodyInput {
        body: request.body,
        producer: request.feature,
    })
}
/// A cylinder of `radius` along +Y from `origin`.
fn y_cylinder(
    session: &mut Session,
    name: &str,
    radius: f64,
    height: &str,
    origin: [&str; 3],
) -> Result<BodyInput, Error> {
    let d = session.document().clone();
    primitive(
        session,
        &PrimitiveRequest::new(
            name,
            PrimitiveParameters::Cylinder {
                radius: length(radius),
                height: mm(&d, height)?,
                frame: PrimitiveFrame {
                    origin: mm3(&d, origin)?,
                    u: [0., 0., 1.],
                    v: [1., 0., 0.],
                },
            },
        ),
    )
}
fn block(
    session: &mut Session,
    name: &str,
    size: [&str; 3],
    origin: [&str; 3],
) -> Result<BodyInput, Error> {
    let d = session.document().clone();
    primitive(
        session,
        &PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: mm3(&d, size)?,
                frame: PrimitiveFrame {
                    origin: mm3(&d, origin)?,
                    ..Default::default()
                },
            },
        ),
    )
}
/// Join or cut `tools` into `target`; returns the new head.
fn boolean(
    session: &mut Session,
    name: &str,
    operation: Operation,
    target: BodyInput,
    tools: Vec<BodyInput>,
) -> Result<BodyInput, Error> {
    let body = target.body;
    let mut request = BooleanRequest::new(operation, target, tools);
    request.name = name.into();
    let prepared = model::prepare_boolean(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(BodyInput {
        body,
        producer: request.feature,
    })
}

/// The illustrative 12 mm push button, local origin on the panel back face
/// at the hole centre, axis +Y (into the C): a cap on the front face, the
/// bushing and switch body through the hole and a nut on the back face.
fn button_part(session: &mut Session) -> Result<EntityId, Error> {
    let cap = y_cylinder(
        session,
        "Push button",
        BUTTON_CAP[0],
        &BUTTON_CAP[1].to_string(),
        [
            "0",
            &format!("=0 mm - $sheet_t - {} mm", BUTTON_CAP[1]),
            "0",
        ],
    )?;
    let body = y_cylinder(
        session,
        "Button bushing and body",
        BUTTON_BUSHING_R,
        &format!("=$sheet_t + {BUTTON_BODY_L} mm"),
        ["0", "=0 mm - $sheet_t", "0"],
    )?;
    let nut = y_cylinder(
        session,
        "Button nut",
        BUTTON_NUT[0],
        &BUTTON_NUT[1].to_string(),
        ["0", "0", "0"],
    )?;
    let part = cap.body;
    boolean(
        session,
        "Button union",
        Operation::Join,
        cap,
        vec![body, nut],
    )?;
    Ok(part)
}

/// The illustrative display module, local origin on the panel back face at
/// the window centre: the board on the standoffs with four mounting holes
/// and the display frame facing the window.
fn display_part(session: &mut Session) -> Result<EntityId, Error> {
    let [bw, bt, bh] = DISPLAY_BOARD;
    let [fw, ft, fh] = DISPLAY_FRAME;
    let board = block(
        session,
        "Display module",
        [&bw.to_string(), &bt.to_string(), &bh.to_string()],
        [
            &(-bw / 2.).to_string(),
            "=$display_standoff",
            &(-bh / 2.).to_string(),
        ],
    )?;
    let frame = block(
        session,
        "Display frame",
        [&fw.to_string(), &ft.to_string(), &fh.to_string()],
        [
            &(-fw / 2.).to_string(),
            &format!("=$display_standoff - {ft} mm"),
            &(-fh / 2.).to_string(),
        ],
    )?;
    let part = board.body;
    let head = boolean(
        session,
        "Display union",
        Operation::Join,
        board,
        vec![frame],
    )?;
    let mut holes = vec![];
    for (i, (x, z)) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]
        .into_iter()
        .enumerate()
    {
        holes.push(y_cylinder(
            session,
            &format!("Display hole {}", i + 1),
            1.6,
            &(bt + 2.).to_string(),
            [
                &(x * DISPLAY_HOLES[0] / 2.).to_string(),
                "=$display_standoff - 1 mm",
                &(z * DISPLAY_HOLES[1] / 2.).to_string(),
            ],
        )?);
    }
    boolean(
        session,
        "Display mounting holes",
        Operation::Cut,
        head,
        holes,
    )?;
    Ok(part)
}

/// One printed end part in print orientation: the end plate sketched on XY
/// (x = depth from the panel front, y = height from the lowest outer face)
/// and extruded by `end_plate`, the plug joined on top of it and two insert
/// pilot holes cut from the plug's flange faces. The right part also gets a
/// cable hole through plate and plug.
fn end_part(session: &mut Session, name: &str, cable: bool) -> Result<(EntityId, EntityId), Error> {
    let (sketch, edits) = Source::create(Datum::Xy, &format!("{name} outline"))?;
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: format!("{name} outline sketch"),
        edits,
    })?;
    session.commit(prepared)?;
    let mut source = Source::read(session.document(), sketch)?.0;
    let bottom = source.add_rectangle([0., 0.], [43., 86.])?;
    source.add_constraint(ConstraintKind::Fix {
        point: source.sketch.points[0].id,
    });
    let depth = source.add_constraint(ConstraintKind::Length {
        line: bottom,
        dimension: length(43.),
    });
    let height = source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: length(86.),
    });
    for (id, text) in [
        (depth, "=$sheet_t + $bend_radius + $flange_depth"),
        (height, "=$panel_height + 2 * ($bend_radius + $sheet_t)"),
    ] {
        source = binding::draft_dimension(
            session.document(),
            &source,
            id,
            &DimensionInput::Expression {
                text: text.into(),
                unit: "mm".into(),
            },
            false,
        )?;
    }
    let edit = multicad_solver::prepare_edit(session, session.revision(), actor(), sketch, source)?;
    session.commit(edit.preview.ok_or("The end plate outline did not solve")?)?;
    let evaluated = multicad_solver::evaluate_document_sketch(
        session.document(),
        session.revision(),
        sketch,
        None,
    )?;
    let regions = evaluated.regions()?;
    if regions.regions.len() != 1 {
        return Err("Expected one end plate region".into());
    }
    let d = session.document().clone();
    let mut extrusion = ExtrudeRequest::new(
        sketch,
        regions.regions.iter().map(|r| r.intent()).collect(),
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: mm(&d, "=$end_plate")?,
                },
            },
            ..Default::default()
        },
    );
    extrusion.outputs[0].name = name.into();
    let prepared = model::prepare_extrude(session, session.revision(), actor(), &extrusion)?;
    let plate = BodyInput {
        body: prepared.outputs[0].body,
        producer: prepared.outputs[0].feature,
    };
    let part = plate.body;
    checked(session, prepared.prepared)?;
    let plug = block(
        session,
        &format!("{name} plug"),
        [
            "=$flange_depth - $end_clearance",
            "=$panel_height + 2 * $bend_radius - 2 * $end_clearance",
            "=$plug_length",
        ],
        [
            "=$sheet_t + $bend_radius + $end_clearance",
            "=$sheet_t + $end_clearance",
            "=$end_plate",
        ],
    )?;
    let head = boolean(
        session,
        &format!("{name} plug join"),
        Operation::Join,
        plate,
        vec![plug],
    )?;
    let x = "=$sheet_t + $bend_radius + $flange_depth / 2";
    let z = "=$end_plate + $end_screw_x";
    let mut holes = vec![
        y_cylinder(
            session,
            &format!("{name} bottom insert hole"),
            INSERT_PILOT_D / 2.,
            &(INSERT_DEPTH + 1.).to_string(),
            [x, "=$sheet_t + $end_clearance - 1 mm", z],
        )?,
        y_cylinder(
            session,
            &format!("{name} top insert hole"),
            INSERT_PILOT_D / 2.,
            &(INSERT_DEPTH + 1.).to_string(),
            [
                x,
                &format!(
                    "=$sheet_t + $panel_height + 2 * $bend_radius - $end_clearance - {INSERT_DEPTH} mm"
                ),
                z,
            ],
        )?,
    ];
    if cable {
        let d = session.document().clone();
        holes.push(primitive(
            session,
            &PrimitiveRequest::new(
                format!("{name} cable hole"),
                PrimitiveParameters::Cylinder {
                    radius: length(CABLE_HOLE_D / 2.),
                    height: mm(&d, "=$end_plate + $plug_length + 2 mm")?,
                    frame: PrimitiveFrame {
                        origin: mm3(
                            &d,
                            [
                                "=$sheet_t + $bend_radius + $flange_depth - 12 mm",
                                "=$sheet_t + $bend_radius + $panel_height / 2",
                                "-1",
                            ],
                        )?,
                        ..Default::default()
                    },
                },
            ),
        )?);
    }
    boolean(
        session,
        &format!("{name} holes"),
        Operation::Cut,
        head,
        holes,
    )?;
    Ok((part, sketch))
}

fn sheet_definition(document: &Document, stock: EntityId) -> Result<SheetDefinition, Error> {
    let flange = |edge| -> Result<Flange, Error> {
        Ok(Flange {
            edge,
            kind: FlangeKind::Bend { angle: angle(90.) },
            direction: BendDirection::Down,
            length: mm(document, "=$flange_depth")?,
            inside_radius: None,
            start_inset: None,
            end_inset: None,
            relief: None,
        })
    };
    let center = "=$panel_height / 2";
    let mut cutouts = vec![Cutout {
        face: CutoutFace::Base,
        shape: CutoutShape::Rectangle {
            center: [mm(document, "=$display_x")?, mm(document, center)?],
            size: [length(DISPLAY_WINDOW[0]), length(DISPLAY_WINDOW[1])],
        },
    }];
    for i in 0..BUTTONS {
        let [u, v] = button_center(i);
        cutouts.push(Cutout {
            face: CutoutFace::Base,
            shape: CutoutShape::Circle {
                center: [mm(document, &u)?, mm(document, &v)?],
                diameter: mm(document, "=$button_hole")?,
            },
        });
    }
    Ok(SheetDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        stock,
        base: SheetBase::Rectangle {
            plane: PanelPlane::Xz,
            origin: [length(0.), length(0.), length(0.)],
            length: mm(document, "=$panel_width")?,
            width: mm(document, "=$panel_height")?,
        },
        reverse: false,
        inside_radius: Some(mm(document, "=$bend_radius")?),
        k_factor: Some(Dimension::Expression {
            expression: BoundExpression::bind(document, "=$k_factor", "ratio", None)?,
        }),
        flanges: vec![flange(0)?, flange(2)?],
        cutouts,
        fasteners: vec![],
    })
}
/// Display standoffs behind the panel and one M3 screw per flange and end,
/// head outside, into the printed plugs.
fn mounting(document: &Document) -> Result<Vec<MountingPattern>, Error> {
    let mut standoffs = vec![];
    for (x, z) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
        let sign = |s: f64| if s < 0. { "-" } else { "+" };
        standoffs.push([
            mm(
                document,
                &format!("=$display_x {} {} mm", sign(x), DISPLAY_HOLES[0] / 2.),
            )?,
            mm(
                document,
                &format!(
                    "=$panel_height / 2 {} {} mm",
                    sign(z),
                    DISPLAY_HOLES[1] / 2.
                ),
            )?,
        ]);
    }
    let screws = |index| -> Result<MountingPattern, Error> {
        Ok(MountingPattern {
            preset: HardwarePreset::Screw {
                thread: Thread::M3,
                length: length(END_SCREW_LENGTH),
                washers: false,
                nut: false,
            },
            face: CutoutFace::Flange { index },
            side: HardwareSide::Material,
            points: vec![
                [
                    mm(document, "=$end_screw_x")?,
                    mm(document, "=$flange_depth / 2")?,
                ],
                [
                    mm(document, "=$panel_width - $end_screw_x")?,
                    mm(document, "=$flange_depth / 2")?,
                ],
            ],
        })
    };
    Ok(vec![
        MountingPattern {
            preset: HardwarePreset::Standoff {
                thread: Thread::M3,
                length: mm(document, "=$display_standoff")?,
                across_flats: length(5.5),
            },
            face: CutoutFace::Base,
            side: HardwareSide::Base,
            points: standoffs,
        },
        screws(0)?,
        screws(1)?,
    ])
}

pub fn hardware_placement_name(pattern: usize, item: HardwareItem) -> Result<&'static str, Error> {
    match (pattern, item) {
        (DISPLAY_STANDOFFS, HardwareItem::Standoff) => Ok("Display standoffs (M3)"),
        (BOTTOM_SCREWS, HardwareItem::Screw) => Ok("Bottom flange screws (M3)"),
        (TOP_SCREWS, HardwareItem::Screw) => Ok("Top flange screws (M3)"),
        _ => Err("Unexpected front panel fastener pattern or item".into()),
    }
}

fn place(
    session: &mut Session,
    part: EntityId,
    name: impl Into<String>,
    transform: NumericTransform,
) -> Result<EntityId, Error> {
    let mut member = ComponentPlacementRequest::new(part, transform);
    member.name = name.into();
    let prepared = model::prepare_place_component(session, session.revision(), actor(), &member)?;
    session.commit(prepared.prepared)?;
    Ok(member.instance)
}
fn translate(document: &Document, at: [&str; 3]) -> Result<NumericTransform, Error> {
    Ok(NumericTransform {
        translation: mm3(document, at)?,
        ..Default::default()
    })
}

pub fn build() -> Result<Scenario, Error> {
    let entities: Vec<Entity> = PARAMETERS
        .iter()
        .map(|(n, e, u, d)| parameter(n, e, u, d))
        .collect();
    let parameters = entities.iter().map(|e| (e.name.clone(), e.id)).collect();
    let mut session = Session::new(
        Document::new(PROJECT_NAME)?.edited(
            &entities
                .into_iter()
                .map(Mutation::Insert)
                .collect::<Vec<_>>(),
        )?,
    )?;

    // Materials and the 1.5 mm aluminium sheet stock.
    let aluminium = material("EN AW-5754 H22", 2670., [196, 200, 206, 255]);
    let petg = material("PETG", 1270., [24, 26, 29, 255]);
    let button_material = material("Push button (illustrative)", 1100., [200, 40, 40, 255]);
    let display_material = material("Display module (illustrative)", 1850., [30, 110, 70, 255]);
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
        &[
            aluminium.clone(),
            petg.clone(),
            button_material.clone(),
            display_material.clone(),
            stock,
        ],
    )?;
    session.commit(prepared)?;

    // The bent front panel and its mounting hardware.
    let sheet = SheetRequest::new(
        "Front panel sheet",
        sheet_definition(session.document(), stock_id)?,
    );
    let prepared = model::prepare_sheet(&session, session.revision(), actor(), &sheet)?;
    checked(&mut session, prepared.prepared)?;
    let patterns = mounting(session.document())?;
    let prepared = model::prepare_sheet_fasteners(
        &session,
        session.revision(),
        actor(),
        sheet.feature,
        &patterns,
    )?;
    let hardware: Vec<_> = prepared
        .hardware
        .iter()
        .map(|(p, i, o)| (*p, *i, o.body))
        .collect();
    checked(&mut session, prepared.model.prepared)?;
    tag(&mut session, &[sheet.body], FABRICATED)?;
    let hardware_parts: Vec<_> = hardware.iter().map(|(.., p)| *p).collect();
    tag(&mut session, &hardware_parts, PURCHASED)?;

    // Illustrative purchased components.
    let button = button_part(&mut session)?;
    let display = display_part(&mut session)?;
    assign(&mut session, &[button], button_material.id)?;
    assign(&mut session, &[display], display_material.id)?;
    for t in [PURCHASED, ILLUSTRATIVE] {
        tag(&mut session, &[button, display], t)?;
    }

    // Printed end parts.
    let (left, left_sketch) = end_part(&mut session, "Left end part", false)?;
    let (right, right_sketch) = end_part(&mut session, "Right end part", true)?;
    assign(&mut session, &[left, right], petg.id)?;
    tag(&mut session, &[left, right], PRINTED)?;
    tag(&mut session, &[left], LEFT_END)?;
    tag(&mut session, &[right], RIGHT_END)?;

    // One assembly: the sheet and its hardware where they were built, six
    // buttons and the display at the openings, the end parts rotated from
    // print orientation onto the panel ends.
    let d = session.document().clone();
    let sheet_member = place(
        &mut session,
        sheet.body,
        PLACED_SHEET,
        NumericTransform::default(),
    )?;
    let mut members = vec![sheet_member];
    let mut parts = vec![sheet.body];
    let mut placed_hardware = vec![];
    for (p, i, part) in &hardware {
        let member = place(
            &mut session,
            *part,
            hardware_placement_name(*p as usize, *i)?,
            NumericTransform::default(),
        )?;
        members.push(member);
        parts.push(*part);
        placed_hardware.push((*p, *i, *part, member));
    }
    let mut buttons = vec![];
    for i in 0..BUTTONS {
        let [u, v] = button_center(i);
        let member = place(
            &mut session,
            button,
            format!("Push button {}", i + 1),
            translate(&d, [&u, "0", &v])?,
        )?;
        buttons.push(member);
        members.push(member);
    }
    parts.push(button);
    let display_member = place(
        &mut session,
        display,
        PLACED_DISPLAY,
        translate(&d, ["=$display_x", "0", "=$panel_height / 2"])?,
    )?;
    members.push(display_member);
    parts.push(display);
    let left_member = place(
        &mut session,
        left,
        PLACED_ENDS[0],
        NumericTransform {
            translation: mm3(
                &d,
                [
                    "=0 mm - $end_plate",
                    "=0 mm - $sheet_t",
                    "=0 mm - $bend_radius - $sheet_t",
                ],
            )?,
            axis: [1., 1., 1.],
            angle: angle(120.),
            ..Default::default()
        },
    )?;
    let right_member = place(
        &mut session,
        right,
        PLACED_ENDS[1],
        NumericTransform {
            translation: mm3(
                &d,
                [
                    "=$panel_width + $end_plate",
                    "=0 mm - $sheet_t",
                    "=$panel_height + $bend_radius + $sheet_t",
                ],
            )?,
            axis: [-1., -1., 1.],
            angle: angle(120.),
            ..Default::default()
        },
    )?;
    members.extend([left_member, right_member]);
    parts.extend([left, right]);
    let assembly = AssemblyRequest::new("Front panel assembly", members, parts);
    let prepared = model::prepare_assembly(&session, session.revision(), actor(), &assembly)?;
    checked(&mut session, prepared.prepared)?;
    let root = place(
        &mut session,
        assembly.assembly,
        PLACED_ASSEMBLY,
        NumericTransform::default(),
    )?;

    let mut s = Scenario {
        session,
        parameters,
        aluminium: aluminium.id,
        petg: petg.id,
        stock: stock_id,
        sheet,
        button,
        buttons,
        display,
        display_member,
        ends: [left, right],
        end_sketches: [left_sketch, right_sketch],
        end_members: [left_member, right_member],
        sheet_member,
        hardware: placed_hardware,
        assembly: assembly.assembly,
        root,
        book: EntityId::new(),
        pages: [EntityId::new(); 4],
        drawing_set: EntityId::new(),
        sheet_set: EntityId::new(),
        print_sets: [EntityId::new(); 2],
    };
    drawings(&mut s)?;
    sets(&mut s)?;
    file_info(&mut s)?;
    Ok(s)
}

// ---------------------------------------------------------------------------
// Drawings and export sets.

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
/// Looking at the panel front (from −Y).
pub const FRONT: ReferenceFrame = ReferenceFrame {
    origin_mm: [0.; 3],
    u: [1., 0., 0.],
    v: [0., 0., 1.],
};
pub const TOP: ReferenceFrame = ReferenceFrame {
    origin_mm: [0.; 3],
    u: [1., 0., 0.],
    v: [0., 1., 0.],
};
/// Looking at the C section from the left end (from −X).
pub const SIDE: ReferenceFrame = ReferenceFrame {
    origin_mm: [0.; 3],
    u: [0., -1., 0.],
    v: [0., 0., 1.],
};
fn info(title: &str) -> DrawingInfo {
    DrawingInfo {
        title: title.into(),
        author: "MultiCAD".into(),
        project: PROJECT_NAME.into(),
        revision: "A".into(),
        checker: "Pending human review".into(),
    }
}
pub const SHEET_NOTE: [&str; 3] = [
    "1.5 mm EN AW-5754 H22. Bend allowance from the part's K-factor and inside radius (k_factor, bend_radius).",
    "No press-brake tooling or springback data. Developed blank: front-panel-developed.dxf at 1:1.",
    "Flat-pattern drawing views are not available in this build; the developed DXF is the flat output.",
];
pub const PRINT_NOTE: [&str; 4] = [
    "End parts: PETG, printed flat on the outer face as modeled (+Z build direction), 0.2 mm layers,",
    "no supports: the plug is a straight prism and the Ø4 insert pilot holes bridge horizontally.",
    "M3 heat-set inserts in the Ø4 × 8 pilot holes (illustrative; check the insert datasheet).",
    "Solid and export validity are checked natively; no physical print or fit has been validated.",
];
pub const COMPONENT_NOTE: [&str; 3] = [
    "Buttons (12 mm bushing) and the 16×2 display module are illustrative envelopes with typical",
    "nominal sizes, not cited manufacturer data. Verify hole sizes and mounting pitch against the",
    "purchased parts. Hardware: nominal ISO values (ISO 4762 M3 × 8; M3 hex standoffs, AF 5.5).",
];
pub const EXPLODED_NOTE: [&str; 3] = [
    "Exploded arrangement: separate views of the left end part, the panel with its components and",
    "the right end part, spaced apart along X. The viewport Explode tool shows the same separation",
    "in 3D; exploded drawing views and exploded geometry are not stored in the project.",
];

/// One Note per line, from the top line down (paper millimetres, y down).
fn notes(lines: &[&str]) -> Vec<Annotation> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Note {
                position_mm: [20., 226. + 5. * i as f64],
                text: (*line).into(),
            },
        })
        .collect()
}
/// Four A3 pages: the assembled panel; the exploded arrangement; the sheet
/// with its bend table; the printed end parts with the BOM.
fn drawings(s: &mut Scenario) -> Result<(), Error> {
    let page = |number: &str, title: &str| {
        let mut page = Page::new(number);
        page.settings.paper = PaperSize::A3;
        page.settings.scale = Scale {
            numerator: 1,
            denominator: 2,
        };
        page.info = info(title);
        page
    };
    let all = vec![s.root];

    let mut assembled = page("FP-101", "Front panel: assembled views");
    let front = view("Front", all.clone(), FRONT, [130., 125.], 2);
    let top = view("Top", all.clone(), TOP, [130., 215.], 2);
    let side = view("Left end", all.clone(), SIDE, [340., 125.], 2);
    assembled.annotations.extend(notes(&COMPONENT_NOTE));
    assembled.views.extend([front, top, side]);

    let mut exploded = page("FP-102", "Front panel: exploded arrangement");
    let mut middle = vec![s.sheet_member, s.display_member];
    middle.extend(&s.buttons);
    middle.extend(s.hardware.iter().map(|h| h.3));
    for (row, frame, y) in [(0, FRONT, 125.), (1, TOP, 215.)] {
        let suffix = if row == 0 { "front" } else { "top" };
        exploded.views.extend([
            view(
                &format!("Left end part, {suffix}"),
                vec![s.end_members[0]],
                frame,
                [70., y],
                2,
            ),
            view(
                &format!("Panel and components, {suffix}"),
                middle.clone(),
                frame,
                [130., y],
                2,
            ),
            view(
                &format!("Right end part, {suffix}"),
                vec![s.end_members[1]],
                frame,
                [190., y],
                2,
            ),
        ]);
    }
    exploded.annotations.extend(notes(&EXPLODED_NOTE));

    let mut sheet = page("FP-103", "Front panel sheet: folded views and bends");
    let sheet_front = view("Sheet front", vec![s.sheet.body], FRONT, [130., 125.], 2);
    sheet.views.extend([
        sheet_front,
        view("Sheet top", vec![s.sheet.body], TOP, [130., 215.], 2),
        view(
            "Sheet section end",
            vec![s.sheet.body],
            SIDE,
            [350., 140.],
            1,
        ),
    ]);
    sheet.annotations.extend(notes(&SHEET_NOTE));

    let mut printed = page("FP-104", "Printed end parts: print orientation");
    printed.settings.scale = Scale {
        numerator: 1,
        denominator: 1,
    };
    printed.views.extend([
        view(
            "Left end part, build plate",
            vec![s.ends[0]],
            TOP,
            [50., 205.],
            1,
        ),
        view(
            "Left end part, side",
            vec![s.ends[0]],
            FRONT,
            [50., 100.],
            1,
        ),
        view(
            "Right end part, build plate",
            vec![s.ends[1]],
            TOP,
            [140., 205.],
            1,
        ),
        view(
            "Right end part, side",
            vec![s.ends[1]],
            FRONT,
            [140., 100.],
            1,
        ),
    ]);
    printed.annotations.extend(notes(&PRINT_NOTE));

    let pages = [assembled.id, exploded.id, sheet.id, printed.id];
    let (book, prepared) = prepare_create(
        &s.session,
        s.session.revision(),
        actor(),
        PROJECT_NAME,
        Some(assembled),
    )?;
    checked(&mut s.session, prepared)?;
    for page in [exploded, sheet, printed] {
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
/// Table placement: (page index, schedule name). Each table sits in the
/// top-right corner of its A3 page (paper millimetres, y down).
const TABLES: [(usize, &str); 3] = [(0, "hardware"), (2, "bend-schedule"), (3, "bom")];
fn top_right(schedule: &Schedule) -> AnnotationKind {
    let mut kind = schedule.table([0., 0.]);
    if let AnnotationKind::Table {
        position_mm,
        widths_mm,
        ..
    } = &mut kind
    {
        *position_mm = [420. - 12. - widths_mm.iter().sum::<f64>(), 12.];
    }
    kind
}
/// The purchased sheet hardware with pieces counted from its solids.
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
                format!("{}", (h.hole_mm * 1000.).round() / 1000.),
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
            "Nominal ISO dimensions or user values; no torque or strength data".into(),
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
/// BOM, bend and hardware schedules of the current revision.
pub fn schedules(session: &Session) -> Result<Vec<(&'static str, Schedule)>, Error> {
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
    ])
}
/// Replace the schedule tables with fresh ones from the current revision.
pub fn refresh_tables(s: &mut Scenario) -> Result<(), Error> {
    let fresh: BTreeMap<_, _> = schedules(&s.session)?.into_iter().collect();
    for index in [0, 2, 3] {
        let mut page = multicad_core::drawings::read(s.session.document(), s.book)?
            .pages
            .into_iter()
            .find(|p| p.id == s.pages[index])
            .ok_or("Drawing page is missing")?;
        page.annotations
            .retain(|a| !matches!(a.kind, AnnotationKind::Table { .. }));
        for (_, name) in TABLES.iter().filter(|(i, _)| *i == index) {
            page.annotations.push(Annotation {
                id: EntityId::new(),
                kind: top_right(&fresh[name]),
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

fn three_mf() -> Recipe {
    Recipe {
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
    }
}
fn sets(s: &mut Scenario) -> Result<(), Error> {
    // Tag queries reach the leaves inside the assembly in their assembled
    // placement; the tagged Parts are the export selection.
    let tagged = |tags: &[&str]| Scope {
        root: ScopeRoot::Project,
        tags: Some(TagQuery {
            all: tags.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }),
    };
    let drawings = SetDefinition {
        scope: tagged(&[FABRICATED]),
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
    let sheet = SetDefinition {
        scope: tagged(&[FABRICATED]),
        recipes: vec![Recipe::step("STEP")],
        ..Default::default()
    };
    let print = |side: &str| SetDefinition {
        scope: tagged(&[PRINTED, side]),
        recipes: vec![three_mf(), Recipe::step("STEP")],
        ..Default::default()
    };
    let mut ids = vec![];
    for (name, definition) in [
        ("Front panel drawings", drawings),
        ("Front panel sheet STEP", sheet),
        ("Left end part print", print(LEFT_END)),
        ("Right end part print", print(RIGHT_END)),
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
    s.drawing_set = ids[0];
    s.sheet_set = ids[1];
    s.print_sets = [ids[2], ids[3]];
    Ok(())
}

// ---------------------------------------------------------------------------
// File information and the rendered preview.

/// Every physical solid of the assembly in world coordinates with its RGB.
pub fn physical_solids<'m>(
    s: &Scenario,
    model: &'m Model,
) -> Result<Vec<(&'m Solid, [u8; 3])>, Error> {
    let document = s.session.document();
    let color = |part: EntityId| -> [u8; 3] {
        let material = multicad_core::materials::resolve(document, part)
            .ok()
            .and_then(|p| p.material.or_else(|| p.stock.map(|(_, s)| s.material)));
        let rgba = material
            .and_then(|m| document.get(m))
            .and_then(|m| match m.data {
                EntityData::Material { color_rgba, .. } => Some(color_rgba),
                _ => None,
            });
        rgba.map_or([90, 92, 98], |c| [c[0], c[1], c[2]])
    };
    let mut out = vec![];
    let mut add = |member: EntityId, part: EntityId| -> Result<(), Error> {
        let solid = model
            .solid(member)
            .ok_or_else(|| format!("Member {member} is not evaluated"))?;
        out.push((solid, color(part)));
        Ok(())
    };
    add(s.sheet_member, s.sheet.body)?;
    for (.., part, member) in &s.hardware {
        add(*member, *part)?;
    }
    for member in &s.buttons {
        add(*member, s.button)?;
    }
    add(s.display_member, s.display)?;
    add(s.end_members[0], s.ends[0])?;
    add(s.end_members[1], s.ends[1])?;
    Ok(out)
}

/// A shaded isometric software rendering of the assembled panel, from the
/// front-left-top; deterministic and GPU-free.
pub fn render_preview(solids: &[(&Solid, [u8; 3])], size: [usize; 2]) -> Result<ColorImage, Error> {
    let [w, h] = size;
    let norm = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v.map(|x| x / l)
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    // The eye looks along `view` (from the front, left, above).
    let view = norm([0.55, 1., -0.6]);
    let right = norm(cross(view, [0., 0., 1.]));
    let up = cross(right, view);
    let light = norm([-0.4, -1., 0.9]);
    let mut triangles = vec![];
    for (solid, rgb) in solids {
        let mesh = solid.mesh(multicad_kernel::MeshOptions {
            linear_deflection_mm: 0.2,
            angular_deflection_rad: 0.4,
            maximum_triangles: 400_000,
        })?;
        let p = |i: u32| {
            let i = i as usize * 3;
            [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
        };
        for t in mesh.indices.as_chunks::<3>().0 {
            triangles.push(([p(t[0]), p(t[1]), p(t[2])], *rgb));
        }
    }
    let project = |p: [f64; 3]| [dot(p, right), dot(p, up), dot(p, view)];
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (t, _) in &triangles {
        for p in t {
            let q = project(*p);
            for k in 0..2 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
    }
    let margin = 0.06;
    let scale = ((w as f64) * (1. - 2. * margin) / (hi[0] - lo[0]))
        .min((h as f64) * (1. - 2. * margin) / (hi[1] - lo[1]));
    let offset = [
        (w as f64 - (hi[0] - lo[0]) * scale) / 2.,
        (h as f64 - (hi[1] - lo[1]) * scale) / 2.,
    ];
    let screen = |p: [f64; 3]| {
        let q = project(p);
        [
            offset[0] + (q[0] - lo[0]) * scale,
            h as f64 - (offset[1] + (q[1] - lo[1]) * scale),
            q[2],
        ]
    };
    let background = |y: usize| {
        let t = y as f64 / h as f64;
        let c = |a: f64, b: f64| (a + (b - a) * t).round() as u8;
        Color32::from_rgb(c(236., 214.), c(239., 219.), c(243., 226.))
    };
    let mut pixels: Vec<Color32> = (0..w * h).map(|i| background(i / w)).collect();
    let mut depth = vec![f64::MAX; w * h];
    for (t, rgb) in &triangles {
        let n = cross(
            [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]],
            [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]],
        );
        let length = dot(n, n).sqrt();
        if length < 1e-12 {
            continue;
        }
        let mut n = n.map(|x| x / length);
        if dot(n, view) > 0. {
            n = n.map(|x| -x);
        }
        let shade = 0.35 + 0.65 * dot(n, light).max(0.);
        let color = Color32::from_rgb(
            (f64::from(rgb[0]) * shade).min(255.) as u8,
            (f64::from(rgb[1]) * shade).min(255.) as u8,
            (f64::from(rgb[2]) * shade).min(255.) as u8,
        );
        let [a, b, c] = t.map(screen);
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-12 {
            continue;
        }
        let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.) as usize;
        let x1 = (a[0].max(b[0]).max(c[0]).ceil() as usize).min(w - 1);
        let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.) as usize;
        let y1 = (a[1].max(b[1]).max(c[1]).ceil() as usize).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                let w2 = 1. - w0 - w1;
                if w0 < 0. || w1 < 0. || w2 < 0. {
                    continue;
                }
                let z = w0 * a[2] + w1 * b[2] + w2 * c[2];
                let i = y * w + x;
                if z < depth[i] {
                    depth[i] = z;
                    pixels[i] = color;
                }
            }
        }
    }
    Ok(ColorImage::new([w, h], pixels))
}

/// Name, description and the rendered preview as one typed transaction.
pub fn file_info(s: &mut Scenario) -> Result<(), Error> {
    let model = Compiler::default().compile(s.session.document(), s.session.revision())?;
    let image = render_preview(&physical_solids(s, &model)?, [960, 600])?;
    let preview = multicad_ui::preview_capture::preview_from(&image)?;
    let edits = project_file::info::edits(
        s.session.document(),
        &FileInfo {
            name: PROJECT_NAME.into(),
            description: DESCRIPTION.into(),
            preview: Some(preview),
            capture_on_save: true,
        },
    )?;
    let prepared = s.session.prepare(Request {
        expected_revision: s.session.revision(),
        actor: actor(),
        label: "File information".into(),
        edits,
    })?;
    s.session.commit(prepared)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Deliverables.

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
pub fn developed_dxf(
    session: &Session,
    sheet: EntityId,
) -> Result<multicad_model::export::sheet_dxf::SheetDxf, Error> {
    let dxf = Exporter::default().developed_sheet_dxf(
        session.document(),
        session.revision(),
        sheet,
        LengthUnit::Millimeter,
        &SheetDxfLayers::default(),
        None,
    )?;
    dxf.check_current(session.document(), session.revision())?;
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

/// Every deliverable of `s` into the new directory `directory`, with a
/// manifest of files, measured values and what is not verified.
pub fn deliverables(s: &Scenario, directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let directory = directory.canonicalize()?;
    let mut files: Vec<PathBuf> = vec![];
    for set in [s.drawing_set, s.sheet_set, s.print_sets[0], s.print_sets[1]] {
        let receipt = export(&s.session, set, &directory)?;
        files.extend(
            receipt
                .files
                .iter()
                .map(|f| Path::new(&receipt.directory).join(&f.artifact.output.filename)),
        );
    }
    let dxf = developed_dxf(&s.session, s.sheet.body)?;
    let developed = directory.join("front-panel-developed.dxf");
    write_new(&developed, &dxf.bytes)?;
    files.push(developed);
    for (name, schedule) in schedules(&s.session)? {
        let path = directory.join(format!("{name}.csv"));
        write_new(&path, schedule.csv().as_bytes())?;
        files.push(path);
    }
    let model = Compiler::default().compile(s.session.document(), s.session.revision())?;
    let sheet = model
        .sheet_metal(s.session.document())?
        .into_iter()
        .find(|r| r.part == s.sheet.body)
        .ok_or("The panel is not evaluated")?;
    let manifest = serde_json::json!({
        "generator": "front_panel_demo",
        "revision": s.session.revision(),
        "sheet": {
            "thickness_mm": sheet.thickness_mm,
            "inside_radius_mm": sheet.inside_radius_mm,
            "k_factor": sheet.k_factor,
            "volume_mm3": sheet.volume_mm3,
            "flat_size_mm": sheet.flat_size_mm,
            "flat_area_mm2": sheet.flat_area_mm2,
            "bends": sheet.bends.len(),
            "drills": dxf.pattern.drills.len(),
        },
        "files": files.iter().map(|f| Ok(serde_json::json!({
            "path": f,
            "sha256": sha256(f)?,
        }))).collect::<Result<Vec<_>, Error>>()?,
        "unsupported": [
            "Flat-pattern and exploded drawing views: no producer is registered in this build; \
             the developed DXF and a separate-view exploded arrangement are provided instead",
        ],
        "verification": {
            "native_geometry": true,
            "interactive_macos": false,
            "windows_linux_runtime": false,
            "human_acceptance": false,
            "press_brake_tooling_or_springback": false,
            "physical_print_or_fit": false,
            "cited_component_datasheets": false,
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
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(args.next().ok_or(
        "Usage: cargo run -p multicad-app --example front_panel_demo -- OUTPUT.tcd [EXPORT_DIR]",
    )?);
    let s = build()?;
    let bytes = archive(&s.session)?;
    let temporary = output.with_extension("tcd.tmp");
    std::fs::write(&temporary, &bytes)?;
    std::fs::rename(&temporary, &output)?;
    println!("Wrote {} ({} bytes)", output.display(), bytes.len());
    if let Some(directory) = args.next() {
        let manifest = deliverables(&s, Path::new(&directory))?;
        println!("{}", serde_json::to_string_pretty(&manifest)?);
    }
    Ok(())
}
