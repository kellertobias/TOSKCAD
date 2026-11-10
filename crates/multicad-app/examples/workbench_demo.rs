//! Generator for the shipped workbench with a welded frame (CAD-183):
//! `examples/welded-workbench.tcd` at the repository root.
//!
//! Everything is built through the typed transaction API that the window
//! and the MCP server share, and every transaction is compiled by the native
//! kernel before it commits. Project parameters drive the whole bench:
//!
//! - an underframe of RHS 60×40×3 steel tube: a top frame of four rails
//!   with mitered corners, four legs butted under the front and back rails,
//!   two side stretchers butted between the legs and a long stretcher butted
//!   between the side stretchers;
//! - four flat bar 40×5 mounting tabs butted to the inside of the front and
//!   back rails, flush with the frame top;
//! - 22 connections, each with a 3 mm fillet weld, in an authored weld
//!   sequence;
//! - a 40 mm solid beech worktop resting on the frame with a 10 × 10 grid of
//!   through holes whose diameter, pitch and edge offsets are parameters,
//!   and four blind screw pilot holes over the tabs;
//! - an under-bench woodworking vise with two beech jaws, metal guide rods,
//!   a bored housing, simplified lead screw and sliding handle;
//! - a drawing book (assembly, weld plan, worktop drilling, schedules) whose
//!   tables come from `multicad_model::schedules`, and export sets for the
//!   frame and the worktop;
//! - file name, description and a preview image rendered headlessly from
//!   the native solids.
//!
//! Usage: `cargo run -p multicad-app --example workbench_demo -- OUTPUT.tcd`
//! (the output must not exist). The app test `tests/workbench_demo.rs`
//! opens the shipped file and compares it with a fresh build.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    drawings::*,
    exchange::*,
    inspection::GeometryReference,
    materials::*,
    model::{
        self, AxisAnchor, BodyOutput, ConnectionDefinition, ConnectionKind, GrainAxis,
        MachiningDefinition, MachiningFace, MachiningKind, MemberDefinition, MemberPath,
        MemberRequest, PanelDefinition, PanelEdges, PanelPlane, PanelRequest, PanelSource,
        SequenceDefinition, SequenceStep, WeldDefinition, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile, info::FileInfo},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
};
use multicad_model::{
    Compiler, DrawingProviders, Model,
    export::Exporter,
    group_cuts,
    schedules::{self, BomOptions, Schedule},
    weld_plan_annotations,
};
use multicad_ui::egui::ColorImage;

#[path = "workbench_demo/upgrade.rs"]
mod upgrade;
#[path = "workbench_demo/vise.rs"]
mod vise;

pub type Error = Box<dyn std::error::Error>;
pub const DOCUMENT_NAME: &str = "Workbench with a welded frame";
pub const DESCRIPTION: &str = "Workbench demo: a welded underframe of RHS 60×40×3 steel tube \
(mitered top frame, legs, stretchers and mounting tabs, 22 fillet welds in a sequence) carrying \
a 40 mm solid beech worktop with a 10 × 10 grid of Ø20 through holes and an under-bench \
woodworking vise with wooden jaws, guide rods, a sliding handle and a simplified lead screw. \
Wood, Weld and Common tools share this project. Edit top_length, \
top_depth, frame_height, hole_diameter, hole_pitch or the edge offsets in Parameters; the frame, \
the grid, the drawings and the schedules follow. Geometry demo only: no load rating, weld \
strength or fabrication approval.";
/// Where the shipped file lives, relative to the repository root.
pub const SHIPPED_PATH: &str = "examples/welded-workbench.tcd";
pub const FRAME_TAG: &str = "frame";
pub const TOP_TAG: &str = "worktop";
/// Rows and columns of the worktop hole grid.
pub const GRID: usize = 10;
/// Worktop stock thickness; the stock is not edited by the demo.
pub const TOP_MM: f64 = 40.;
/// RHS section: 40 wide (horizontal), 60 high (vertical) on rails.
pub const RHS: [f64; 2] = [40., 60.];
pub const PILOT_DIAMETER_MM: f64 = 4.;
pub const PILOT_DEPTH_MM: f64 = 25.;

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
/// A dimension: "#120" is a literal millimeter value, "=…" binds an expression.
fn dim(session: &Session, text: &str) -> Result<Dimension, Error> {
    if let Some(stripped) = text.strip_prefix('#') {
        return Ok(length(stripped.parse()?));
    }
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(session.document(), text, "mm", None)?,
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
fn profile(
    material: EntityId,
    designation: &str,
    size: [f64; 2],
    wall: Option<f64>,
) -> Result<Entity, Error> {
    Ok(LibraryDefinition::Stock(Stock {
        material,
        specification: StockKind::Profile {
            designation: designation.into(),
            length_mm: 6000.,
            width_mm: size[0],
            height_mm: size[1],
            wall_mm: wall,
            shape: ProfileShape::Rectangular,
        },
    })
    .entity(EntityId::new(), designation)?)
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

/// Parameter entities by name.
#[derive(Clone, Copy, Debug)]
pub struct Parameters {
    pub top_length: EntityId,
    pub top_depth: EntityId,
    pub top_overhang: EntityId,
    pub frame_height: EntityId,
    pub stretcher_height: EntityId,
    pub hole_diameter: EntityId,
    pub hole_pitch: EntityId,
    pub hole_edge_x: EntityId,
    pub hole_edge_y: EntityId,
}
/// Member Parts by role.
#[derive(Clone, Debug)]
pub struct Frame {
    /// Top rails: front, right, back, left.
    pub rails: Vec<EntityId>,
    /// Legs: front left, front right, back right, back left.
    pub legs: Vec<EntityId>,
    /// Side stretchers: left, right.
    pub stretchers: Vec<EntityId>,
    pub long_stretcher: EntityId,
    /// Mounting tabs: front left, front right, back right, back left.
    pub tabs: Vec<EntityId>,
    /// Foot plates under the legs, in leg order.
    pub feet: Vec<EntityId>,
    /// Connections in weld order, with their welds.
    pub connections: Vec<EntityId>,
    pub welds: Vec<EntityId>,
    pub sequence: EntityId,
}
impl Frame {
    pub fn members(&self) -> Vec<EntityId> {
        let mut all = self.rails.clone();
        all.extend(&self.legs);
        all.extend(&self.stretchers);
        all.push(self.long_stretcher);
        all.extend(&self.tabs);
        all.extend(&self.feet);
        all
    }
}
pub struct Scenario {
    pub session: Session,
    pub parameters: Parameters,
    pub steel: EntityId,
    pub beech: EntityId,
    pub rhs: EntityId,
    pub flat: EntityId,
    pub plate: EntityId,
    pub top_stock: EntityId,
    pub frame: Frame,
    pub worktop: EntityId,
    pub vise: Vec<EntityId>,
    pub grid: EntityId,
    pub pilots: EntityId,
    pub book: EntityId,
    pub assembly_page: EntityId,
    pub weld_page: EntityId,
    pub drilling_page: EntityId,
    pub schedule_page: EntityId,
    pub frame_set: EntityId,
    pub worktop_set: EntityId,
}
impl Scenario {
    pub fn parts(&self) -> Vec<EntityId> {
        let mut all = self.frame.members();
        all.push(self.worktop);
        all.extend(&self.vise);
        all
    }
}

/// The frame corner and leg positions as expressions.
const X0: &str = "=$top_overhang + 20 mm";
const X1: &str = "=$top_length - $top_overhang - 20 mm";
const Y0: &str = "=$top_overhang + 20 mm";
const Y1: &str = "=$top_depth - $top_overhang - 20 mm";
const LEG_X0: &str = "=$top_overhang + 70 mm";
const LEG_X1: &str = "=$top_length - $top_overhang - 70 mm";
const TAB_X0: &str = "=$top_overhang + 150 mm";
const TAB_X1: &str = "=$top_length - $top_overhang - 150 mm";
const TAB_Y0: &str = "=$top_overhang + 90 mm";
const TAB_Y1: &str = "=$top_depth - $top_overhang - 90 mm";
const FH: &str = "=$frame_height";
const SH: &str = "=$stretcher_height";

fn member(
    session: &Session,
    name: &str,
    stock: EntityId,
    start: [&str; 3],
    end: [&str; 3],
    anchor: [AxisAnchor; 2],
    rotation: Option<f64>,
) -> Result<MemberRequest, Error> {
    Ok(MemberRequest::new(
        name,
        MemberDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock,
            path: MemberPath::Points {
                start: point(session, start)?,
                end: point(session, end)?,
            },
            anchor,
            rotation: rotation.map(angle),
            extend_start: None,
            extend_end: None,
        },
    ))
}

fn build_frame(
    session: &mut Session,
    rhs: EntityId,
    flat: EntityId,
    plate: EntityId,
) -> Result<Frame, Error> {
    // Horizontal members hang below the frame top: their section's top face
    // is on the path at frame_height.
    let hang = [AxisAnchor::Center, AxisAnchor::Max];
    let center = [AxisAnchor::Center; 2];
    let corners = [[X0, Y0], [X1, Y0], [X1, Y1], [X0, Y1]];
    let mut requests = vec![];
    for i in 0..4 {
        let [ax, ay] = corners[i];
        let [bx, by] = corners[(i + 1) % 4];
        requests.push(member(
            session,
            &format!("Top rail {}", i + 1),
            rhs,
            [ax, ay, FH],
            [bx, by, FH],
            hang,
            None,
        )?);
    }
    // Legs turned a quarter so the 60 mm side runs along the rails.
    let legs = [[LEG_X0, Y0], [LEG_X1, Y0], [LEG_X1, Y1], [LEG_X0, Y1]];
    for (i, [x, y]) in legs.into_iter().enumerate() {
        requests.push(member(
            session,
            &format!("Leg {}", i + 1),
            rhs,
            [x, y, "#0"],
            [x, y, FH],
            center,
            Some(90.),
        )?);
    }
    for (name, x) in [("Side stretcher L", LEG_X0), ("Side stretcher R", LEG_X1)] {
        requests.push(member(
            session,
            name,
            rhs,
            [x, Y0, SH],
            [x, Y1, SH],
            center,
            None,
        )?);
    }
    requests.push(member(
        session,
        "Long stretcher",
        rhs,
        [LEG_X0, "=$top_depth / 2", SH],
        [LEG_X1, "=$top_depth / 2", SH],
        center,
        None,
    )?);
    // Tabs run from the rail centerline inward; the butt joint trims them
    // to the rail's inner face.
    let tabs = [
        [TAB_X0, Y0, TAB_Y0],
        [TAB_X1, Y0, TAB_Y0],
        [TAB_X1, Y1, TAB_Y1],
        [TAB_X0, Y1, TAB_Y1],
    ];
    for (i, [x, y, end]) in tabs.into_iter().enumerate() {
        requests.push(member(
            session,
            &format!("Mounting tab {}", i + 1),
            flat,
            [x, y, FH],
            [x, end, FH],
            hang,
            None,
        )?);
    }
    // Foot plates on the floor, centered under the legs.
    let feet = [
        ["=$top_overhang + 30 mm", "=$top_overhang + 110 mm", Y0],
        [
            "=$top_length - $top_overhang - 110 mm",
            "=$top_length - $top_overhang - 30 mm",
            Y0,
        ],
        [
            "=$top_length - $top_overhang - 110 mm",
            "=$top_length - $top_overhang - 30 mm",
            Y1,
        ],
        ["=$top_overhang + 30 mm", "=$top_overhang + 110 mm", Y1],
    ];
    for (i, [a, b, y]) in feet.into_iter().enumerate() {
        requests.push(member(
            session,
            &format!("Foot plate {}", i + 1),
            plate,
            [a, y, "#0"],
            [b, y, "#0"],
            [AxisAnchor::Center, AxisAnchor::Min],
            None,
        )?);
    }
    let prepared = model::prepare_members(session, session.revision(), actor(), &requests, None)?;
    checked(session, prepared.prepared)?;
    let body = |i: usize| requests[i].body;
    let rails: Vec<_> = (0..4).map(body).collect();
    let legs: Vec<_> = (4..8).map(body).collect();
    let stretchers = vec![body(8), body(9)];
    let long_stretcher = body(10);
    let tabs: Vec<_> = (11..15).map(body).collect();
    let feet: Vec<_> = (15..19).map(body).collect();

    // Connections in the order they are welded, with step notes.
    let mut joints: Vec<(String, ConnectionKind, [EntityId; 2], &str, bool)> = vec![];
    for i in 0..4 {
        joints.push((
            format!("Top corner {}", i + 1),
            ConnectionKind::Miter,
            [rails[i], rails[(i + 1) % 4]],
            if i == 0 {
                "Top frame flat on the table; tack corners, check diagonals"
            } else {
                ""
            },
            true,
        ));
    }
    for (i, leg) in legs.iter().enumerate() {
        // Legs stand under the front (0) and back (2) rails.
        let rail = if i < 2 { rails[0] } else { rails[2] };
        joints.push((
            format!("Leg {} top", i + 1),
            ConnectionKind::Butt,
            [*leg, rail],
            if i == 0 {
                "Frame inverted; square each leg, alternate corners"
            } else {
                ""
            },
            true,
        ));
    }
    for (i, (leg, foot)) in legs.iter().zip(&feet).enumerate() {
        joints.push((
            format!("Leg {} foot plate", i + 1),
            ConnectionKind::Butt,
            [*leg, *foot],
            if i == 0 {
                "Frame inverted; plates square to the legs"
            } else {
                ""
            },
            true,
        ));
    }
    for (i, (name, front, back)) in [("L", legs[0], legs[3]), ("R", legs[1], legs[2])]
        .into_iter()
        .enumerate()
    {
        for (end, leg) in [("front", front), ("back", back)] {
            joints.push((
                format!("Side stretcher {name} {end}"),
                ConnectionKind::Butt,
                [stretchers[i], leg],
                if i == 0 && end == "front" {
                    "Clamp stretchers at height; check legs plumb"
                } else {
                    ""
                },
                true,
            ));
        }
    }
    for (i, (end, side)) in [("left", stretchers[0]), ("right", stretchers[1])]
        .into_iter()
        .enumerate()
    {
        joints.push((
            format!("Long stretcher {end}"),
            ConnectionKind::Butt,
            [long_stretcher, side],
            if i == 0 {
                "Center the long stretcher"
            } else {
                ""
            },
            true,
        ));
    }
    for (i, tab) in tabs.iter().enumerate() {
        let rail = if i < 2 { rails[0] } else { rails[2] };
        joints.push((
            format!("Tab {}", i + 1),
            ConnectionKind::Butt,
            [*tab, rail],
            if i == 0 {
                "Tabs flush with the frame top; weld the tab end only"
            } else {
                ""
            },
            false,
        ));
    }
    let mut connections = vec![];
    let mut welds = vec![];
    let mut steps = vec![];
    for (i, (name, kind, members, step_note, all_around)) in joints.into_iter().enumerate() {
        let (id, prepared) = model::prepare_connection(
            session,
            session.revision(),
            actor(),
            &name,
            &ConnectionDefinition {
                kind,
                members,
                gap: None,
            },
        )?;
        checked(session, prepared)?;
        connections.push(id);
        let (weld, prepared) = model::prepare_weld(
            session,
            session.revision(),
            actor(),
            &WeldDefinition {
                connection: id,
                mark: format!("W{}", i + 1),
                symbol: WeldSymbol::Fillet,
                side: WeldSide::Arrow,
                size: Some(length(3.)),
                length: (!all_around).then(|| length(40.)),
                all_around,
                field: false,
                process: "135 MAG".into(),
                notes: String::new(),
            },
        )?;
        // A weld record changes no geometry; the next native check covers it.
        session.commit(prepared)?;
        welds.push(weld);
        steps.push(SequenceStep {
            weld,
            note: step_note.into(),
        });
    }
    let (sequence, prepared) = model::prepare_sequence(
        session,
        session.revision(),
        actor(),
        None,
        "Workbench frame sequence",
        &SequenceDefinition { steps },
    )?;
    session.commit(prepared)?;
    let frame = Frame {
        rails,
        legs,
        stretchers,
        long_stretcher,
        tabs,
        feet,
        connections,
        welds,
        sequence,
    };
    tag(session, &frame.members(), FRAME_TAG)?;
    Ok(frame)
}

/// The grid hole centers in worktop face coordinates as expressions: row
/// `r` along Y, column `c` along X.
pub fn grid_holes(session: &Session) -> Result<Vec<[Dimension; 2]>, Error> {
    let mut holes = vec![];
    for r in 0..GRID {
        for c in 0..GRID {
            holes.push([
                dim(session, &format!("=$hole_edge_x + {c} * $hole_pitch"))?,
                dim(session, &format!("=$hole_edge_y + {r} * $hole_pitch"))?,
            ]);
        }
    }
    Ok(holes)
}

/// A planar face of a Part whose outward normal matches `normal` and whose
/// plane passes through coordinate `at` along that normal.
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
        intent: GeometryIntent::capture(&topology, element.key)
            .map_err(|e| format!("Face {normal:?} at {at}: {e}"))?,
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
fn view(
    name: &str,
    entities: Vec<EntityId>,
    origin: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    position: [f64; 2],
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
        kind: ViewKind::Projection,
        position_mm: position,
        scale: Scale {
            numerator: 1,
            denominator: 10,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.05,
        guide: None,
        equipment_categories: None,
    }
}
fn a3(number: &str, title: &str) -> Page {
    let mut page = Page::new(number);
    page.settings.paper = PaperSize::A3;
    page.settings.scale = Scale {
        numerator: 1,
        denominator: 10,
    };
    page.info = DrawingInfo {
        title: title.into(),
        author: "ToskCAD".into(),
        project: DOCUMENT_NAME.into(),
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

/// Every schedule placed on the drawing pages.
pub struct Tables {
    pub panels: Schedule,
    pub machining: Schedule,
    pub members: Schedule,
    pub bom: Schedule,
    pub welds: Schedule,
}
/// Every schedule of the current model, from the shared builders.
pub fn tables(session: &Session) -> Result<Tables, Error> {
    let (document, revision) = (session.document(), session.revision());
    let model = Compiler::default().compile(document, revision)?;
    let groups = group_cuts(&model.member_cut_list(document)?);
    let bom = model.bom(document, &BomOptions::default())?;
    Ok(Tables {
        panels: schedules::cut_list_schedule(document, revision, &model.cut_list(document)?)?,
        machining: schedules::machining_schedule(
            document,
            revision,
            &model
                .machining(document)?
                .into_iter()
                .filter(|r| document.get(r.part).is_some_and(|p| p.name == "Worktop"))
                .collect::<Vec<_>>(),
        )?,
        members: schedules::member_cut_schedule(document, revision, &groups)?,
        bom: schedules::bom_schedule(&bom),
        welds: schedules::weld_schedule(document, revision, &model.weld_schedule(document)?)?,
    })
}
const DISCLAIMER: &str = "Geometry demo only: not a load rating, weld strength, welding procedure or fabrication approval";
/// Annotations of the weld plan, drilling and schedule pages.
fn table_annotations(
    session: &Session,
    weld_view: &DrawingView,
) -> Result<[Vec<Annotation>; 3], Error> {
    let t = tables(session)?;
    let table = |s: &Schedule, at: [f64; 2]| Annotation {
        id: EntityId::new(),
        kind: s.table(at),
    };
    let rows = |s: &Schedule| (s.rows.len() + 2) as f64 * 5.;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let mut welds = weld_plan_annotations(
        &model.weld_schedule(session.document())?,
        weld_view,
        [15., 205.],
    );
    for left in [true, false] {
        let mut entries: Vec<_> = welds
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match &a.kind {
                AnnotationKind::Weld { points_mm, .. }
                    if (points_mm[0][0] < weld_view.position_mm[0]) == left =>
                {
                    Some((i, points_mm[0][1]))
                }
                _ => None,
            })
            .collect();
        entries.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (row, (index, _)) in entries.iter().enumerate() {
            if let AnnotationKind::Weld {
                points_mm, tail, ..
            } = &mut welds[*index].kind
            {
                *points_mm = vec![
                    points_mm[0],
                    [
                        if left { 15. } else { 245. },
                        28. + row as f64
                            * (162. / (entries.len().saturating_sub(1).max(1)) as f64).min(17.),
                    ],
                ];
                *tail = tail.split_whitespace().next().unwrap_or("").to_string();
            }
        }
    }
    for a in &mut welds {
        if let AnnotationKind::Table {
            columns,
            rows,
            widths_mm,
            ..
        } = &mut a.kind
            && columns.last().is_some_and(|c| c == "Notes")
        {
            columns.pop();
            widths_mm.pop();
            for row in rows {
                row.pop();
            }
        }
    }
    welds.push(note([15., 355.], DISCLAIMER));
    let drilling = vec![
        table(&t.machining, [15., 175.]),
        table(&t.panels, [15., 185. + rows(&t.machining)]),
        note(
            [15., 264.],
            "Grid: 10 rows × 10 columns, through; diameter, pitch and edge offsets are the hole_* parameters",
        ),
        note(
            [15., 272.],
            "Pilot holes from below for wood screws through the frame tabs; tab holes are drilled on assembly",
        ),
    ];
    let schedule = vec![
        table(&t.members, [15., 20.]),
        table(&t.bom, [15., 30. + rows(&t.members)]),
        note([15., 272.], DISCLAIMER),
    ];
    Ok([welds, drilling, schedule])
}
/// Page numbers of the drawing book, in order.
pub const PAGES: [&str; 4] = ["WB-101", "WB-102", "WB-103", "WB-104"];
/// The page of `book` with this number.
pub fn page(session: &Session, book: EntityId, number: &str) -> Result<Page, Error> {
    Ok(multicad_core::drawings::read(session.document(), book)?
        .pages
        .into_iter()
        .find(|p| p.number == number)
        .ok_or_else(|| format!("Page {number} is missing"))?)
}
/// Replace the weld plan, drilling and schedule tables with ones from the
/// current model; views and associative dimensions are left alone.
pub fn refresh_tables(session: &mut Session, book: EntityId) -> Result<(), Error> {
    let mut weld_page = page(session, book, PAGES[1])?;
    weld_page.settings.orientation = Orientation::Portrait;
    weld_page.views[0].position_mm = [140., 120.];
    let [welds, drilling, schedule] = table_annotations(session, &weld_page.views[0])?;
    for (number, annotations) in [
        (PAGES[1], welds),
        (PAGES[2], drilling),
        (PAGES[3], schedule),
    ] {
        let mut page = if number == PAGES[1] {
            weld_page.clone()
        } else {
            page(session, book, number)?
        };
        page.info.project = DOCUMENT_NAME.into();
        page.info.author = "ToskCAD".into();
        page.annotations = annotations;
        let prepared = prepare(
            session,
            session.revision(),
            actor(),
            book,
            PageAction::Replace(page),
        )?;
        checked(session, prepared)?;
    }
    Ok(())
}
/// Placed schedule tables that no longer match the current model.
pub fn stale_tables(session: &Session, book: EntityId) -> Result<Vec<String>, Error> {
    let t = tables(session)?;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let mut stale = vec![];
    for page in &multicad_core::drawings::read(session.document(), book)?.pages {
        for a in &page.annotations {
            let AnnotationKind::Table { title, rows, .. } = &a.kind else {
                continue;
            };
            let fresh = [&t.panels, &t.machining, &t.members, &t.bom]
                .into_iter()
                .find(|s| title.starts_with(&s.title));
            if let Some(fresh) = fresh {
                if let Err(e) = fresh.check_table(&a.kind) {
                    stale.push(e);
                }
            } else if title == "Weld sequence" {
                let mut expected = weld_plan_annotations(
                    &model.weld_schedule(session.document())?,
                    &page.views[0],
                    [0., 0.],
                );
                for a in &mut expected {
                    if let AnnotationKind::Table {
                        columns,
                        rows,
                        widths_mm,
                        ..
                    } = &mut a.kind
                        && columns.last().is_some_and(|c| c == "Notes")
                    {
                        columns.pop();
                        widths_mm.pop();
                        for row in rows {
                            row.pop();
                        }
                    }
                }
                let same = expected.last().is_some_and(
                    |e| matches!(&e.kind, AnnotationKind::Table { rows: r, .. } if r == rows),
                );
                if !same {
                    stale.push("The weld plan is stale".into());
                }
            }
        }
    }
    Ok(stale)
}

fn build_drawings(
    session: &mut Session,
    frame: &Frame,
    worktop: EntityId,
    vise: &[EntityId],
) -> Result<(EntityId, [EntityId; 4]), Error> {
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let mut everything = frame.members();
    everything.push(worktop);
    everything.extend(vise);
    let mut assembly = a3(PAGES[0], "Workbench assembly");
    let front = view(
        "Front",
        everything.clone(),
        [750., 0., 450.],
        [1., 0., 0.],
        [0., 0., 1.],
        [120., 95.],
    );
    let side = view(
        "Side",
        everything.clone(),
        [0., 500., 450.],
        [0., 1., 0.],
        [0., 0., 1.],
        [290., 95.],
    );
    let top = view(
        "Top",
        everything,
        [750., 500., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [120., 215.],
    );
    let surface = 860. + TOP_MM;
    let annotations = vec![
        dimension(
            front.id,
            face(&model, worktop, [-1., 0., 0.], 0.)?,
            face(&model, worktop, [1., 0., 0.], 1500.)?,
            LinearDirection::Horizontal,
            14.,
        ),
        dimension(
            front.id,
            face(&model, frame.feet[0], [0., 0., -1.], 0.)?,
            face(&model, worktop, [0., 0., 1.], surface)?,
            LinearDirection::Vertical,
            -30.,
        ),
        dimension(
            side.id,
            face(&model, worktop, [0., -1., 0.], 0.)?,
            face(&model, worktop, [0., 1., 0.], 1000.)?,
            LinearDirection::Horizontal,
            14.,
        ),
        dimension(
            side.id,
            face(&model, frame.feet[0], [0., 0., -1.], 0.)?,
            face(&model, frame.rails[0], [0., 0., 1.], 860.)?,
            LinearDirection::Vertical,
            -20.,
        ),
        note(
            [230., 200.],
            "RHS 60×40×3 S235 underframe, flat bar 40×5 tabs, 40 mm solid beech worktop",
        ),
        note(
            [230., 208.],
            "Worktop rests on the frame and is screwed up through the four tabs",
        ),
        note([230., 216.], DISCLAIMER),
    ];
    assembly.views = vec![front, side, top];
    assembly.annotations = annotations;
    let assembly_page = assembly.id;
    let (book, prepared) = prepare_create(
        session,
        session.revision(),
        actor(),
        DOCUMENT_NAME,
        Some(assembly),
    )?;
    checked(session, prepared)?;

    let mut weld = a3(PAGES[1], "Frame weld plan");
    // An isometric view from the front left, so no two joints coincide.
    let (r2, r6) = (2_f64.sqrt(), 6_f64.sqrt());
    weld.views = vec![view(
        "Frame isometric",
        frame.members(),
        [750., 500., 430.],
        [1. / r2, -1. / r2, 0.],
        [1. / r6, 1. / r6, 2. / r6],
        [110., 118.],
    )];
    let mut drilling = a3(PAGES[2], "Worktop drilling");
    let plan = view(
        "Worktop plan",
        vec![worktop],
        [750., 500., 900.],
        [1., 0., 0.],
        [0., 1., 0.],
        [105., 120.],
    );
    drilling.views = vec![plan];
    let schedule = a3(PAGES[3], "Members and BOM");
    let ids = [assembly_page, weld.id, drilling.id, schedule.id];
    for page in [weld, drilling, schedule] {
        let prepared = prepare(
            session,
            session.revision(),
            actor(),
            book,
            PageAction::Add(page),
        )?;
        checked(session, prepared)?;
    }
    Ok((book, ids))
}

fn build_export_sets(
    session: &mut Session,
    worktop: EntityId,
) -> Result<(EntityId, EntityId), Error> {
    let frame_set = EntityId::new();
    let prepared = prepare_set(
        session,
        session.revision(),
        actor(),
        frame_set,
        "Workbench frame and drawings",
        &SetDefinition {
            scope: Scope {
                root: ScopeRoot::Project,
                tags: Some(TagQuery {
                    all: [FRAME_TAG.into()].into(),
                    ..Default::default()
                }),
            },
            pages: Pages::Project,
            recipes: vec![
                Recipe {
                    id: EntityId::new(),
                    name: "PDF".into(),
                    output: RecipeOutput::Pdf,
                },
                Recipe::step("STEP"),
            ],
            ..Default::default()
        },
    )?;
    session.commit(prepared)?;
    let worktop_set = EntityId::new();
    let scope = Scope {
        root: ScopeRoot::Project,
        tags: Some(TagQuery {
            all: [TOP_TAG.into()].into(),
            ..Default::default()
        }),
    };
    let dxf = Recipe {
        id: EntityId::new(),
        name: "Worktop DXF".into(),
        output: RecipeOutput::PlanarDxf {
            units: LengthUnit::Millimeter,
            layers: Layers::default(),
            aids: AidPolicy::IncludeAll,
        },
    };
    let catalog =
        Exporter::default().planar_catalog(session.document(), session.revision(), &scope, None)?;
    let mut definition = SetDefinition {
        scope,
        recipes: vec![dxf.clone(), Recipe::step("STEP")],
        ..Default::default()
    };
    let mut links = vec![];
    for part in catalog
        .parts
        .into_iter()
        .filter(|p| p.subject.part == worktop)
    {
        // The upper face: the grid shows from above, and the blind pilot
        // holes from below do not cross it.
        let face = part
            .faces
            .into_iter()
            .filter(|f| f.plane.normal()[2] > 0.9)
            .max_by(|a, b| a.plane.origin_mm[2].total_cmp(&b.plane.origin_mm[2]))
            .ok_or("Worktop has no upper face")?;
        definition.planar_faces.push(PlanarFace {
            recipe: dxf.id,
            subject: part.subject,
            source: face.source,
            reference: face.intent.id,
            horizontal: PlanarAxis::WorldX,
        });
        links.push(Link {
            target: face.source,
            role: PLANAR_FACE_ROLE.into(),
            geometry: Some(face.intent),
        });
    }
    let prepared = prepare_set_with_geometry(
        session,
        session.revision(),
        actor(),
        worktop_set,
        "Worktop drilling",
        &definition,
        &links,
    )?;
    checked(session, prepared)?;
    Ok((frame_set, worktop_set))
}

/// Build the editable project; file information is set by [`finish`].
pub fn build() -> Result<Scenario, Error> {
    let top_length = parameter("top_length", "1500", "Worktop length along X");
    let top_depth = parameter("top_depth", "1000", "Worktop depth along Y");
    let top_overhang = parameter(
        "top_overhang",
        "50",
        "Worktop edge to the outside of the steel frame, all round",
    );
    let frame_height = parameter(
        "frame_height",
        "860",
        "Floor to the top of the steel frame (the worktop underside)",
    );
    let stretcher_height = parameter(
        "stretcher_height",
        "150",
        "Floor to the stretcher centerlines",
    );
    let hole_diameter = parameter("hole_diameter", "20", "Worktop grid hole diameter");
    let hole_pitch = parameter(
        "hole_pitch",
        "80",
        "Center distance of the worktop grid, both directions",
    );
    let hole_edge_x = parameter(
        "hole_edge_x",
        "=($top_length - 9 * $hole_pitch) / 2",
        "Worktop left edge to the first grid column; centered by default",
    );
    let hole_edge_y = parameter(
        "hole_edge_y",
        "=($top_depth - 9 * $hole_pitch) / 2",
        "Worktop front edge to the first grid row; centered by default",
    );
    let vise_opening = parameter("vise_opening", "80", "Clear distance between the vise jaws");
    let vise_center_x = parameter(
        "vise_center_x",
        "=$top_length / 5",
        "Vise center along the front edge",
    );
    let vise_jaw_width = parameter("vise_jaw_width", "240", "Width of the two beech vise jaws");
    let all = [
        &top_length,
        &top_depth,
        &top_overhang,
        &frame_height,
        &stretcher_height,
        &hole_diameter,
        &hole_pitch,
        &hole_edge_x,
        &hole_edge_y,
        &vise_opening,
        &vise_center_x,
        &vise_jaw_width,
    ];
    let mut edits: Vec<_> = all.iter().map(|p| Mutation::Insert((*p).clone())).collect();
    let rows: Vec<EntityId> = all.iter().map(|p| p.id).collect();
    let mut table = Entity::new(
        "Parameter table",
        EntityData::Extension {
            definition: multicad_core::document::Definition {
                type_id: "core.parameter_table".into(),
                schema_version: 1,
                data: serde_json::to_value(&rows)?,
            },
            inputs: rows.iter().map(|id| Link::new(*id, "row")).collect(),
        },
    );
    table.description = "Canonical row order shared with the Parameters table".into();
    edits.push(Mutation::Insert(table));
    let mut session = Session::new(Document::new(DOCUMENT_NAME)?.edited(&edits)?)?;
    let parameters = Parameters {
        top_length: top_length.id,
        top_depth: top_depth.id,
        top_overhang: top_overhang.id,
        frame_height: frame_height.id,
        stretcher_height: stretcher_height.id,
        hole_diameter: hole_diameter.id,
        hole_pitch: hole_pitch.id,
        hole_edge_x: hole_edge_x.id,
        hole_edge_y: hole_edge_y.id,
    };

    let steel = material("S235 steel", 7850., [120, 124, 130, 255]);
    let beech = material("Beech", 720., [214, 170, 120, 255]);
    let rhs = profile(steel.id, "RHS 60×40×3", RHS, Some(3.))?;
    let flat = profile(steel.id, "Flat bar 40×5", [40., 5.], None)?;
    let plate = profile(steel.id, "Flat bar 80×8", [80., 8.], None)?;
    let top_stock = LibraryDefinition::Stock(Stock {
        material: beech.id,
        specification: StockKind::Panel {
            thickness_mm: TOP_MM,
            length_mm: 3000.,
            width_mm: 1250.,
            grain: Grain::AlongLength,
        },
    })
    .entity(EntityId::new(), "Beech block 40")?;
    let ids = (steel.id, beech.id, rhs.id, flat.id, plate.id, top_stock.id);
    let prepared = prepare_library(&session, session.revision(), actor(), &[steel, beech])?;
    session.commit(prepared)?;
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[rhs, flat, plate, top_stock],
    )?;
    session.commit(prepared)?;
    let (steel, beech, rhs, flat, plate, top_stock) = ids;

    let frame = build_frame(&mut session, rhs, flat, plate)?;
    let vise = vise::build(&mut session, top_stock, steel, beech)?;

    // The worktop rests on the frame top.
    let request = PanelRequest::new(
        "Worktop",
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Rectangle {
                stock: top_stock,
                plane: PanelPlane::Xy,
                origin: point(&session, ["#0", "#0", FH])?,
                length: dim(&session, "=$top_length")?,
                width: dim(&session, "=$top_depth")?,
            },
            grain: GrainAxis::U,
            edges: PanelEdges::default(),
            oversize: None,
        },
    );
    let prepared = model::prepare_panel(&session, session.revision(), actor(), &request)?;
    checked(&mut session, prepared.prepared)?;
    let worktop = request.body;
    let grid = model::prepare_machining(
        &session,
        session.revision(),
        actor(),
        "Hole grid 10 × 10",
        &MachiningDefinition {
            generated_by: None,
            kind: MachiningKind::Drill {
                holes: grid_holes(&session)?,
                diameter: dim(&session, "=$hole_diameter")?,
                depth: None,
            },
            face: MachiningFace::Top,
            targets: vec![worktop],
            suppressed: false,
        },
    )?;
    let grid_id = grid.operation;
    checked(&mut session, grid.model.prepared)?;
    // Screw pilot holes from below, centered on the tabs.
    let mut pilots = vec![];
    for [x, y] in [
        [TAB_X0, "=$top_overhang + 65 mm"],
        [TAB_X1, "=$top_overhang + 65 mm"],
        [TAB_X1, "=$top_depth - $top_overhang - 65 mm"],
        [TAB_X0, "=$top_depth - $top_overhang - 65 mm"],
    ] {
        pilots.push([dim(&session, x)?, dim(&session, y)?]);
    }
    let pilot = model::prepare_machining(
        &session,
        session.revision(),
        actor(),
        "Tab screw pilots",
        &MachiningDefinition {
            generated_by: None,
            kind: MachiningKind::Drill {
                holes: pilots,
                diameter: length(PILOT_DIAMETER_MM),
                depth: Some(length(PILOT_DEPTH_MM)),
            },
            face: MachiningFace::Bottom,
            targets: vec![worktop],
            suppressed: false,
        },
    )?;
    let pilot_id = pilot.operation;
    checked(&mut session, pilot.model.prepared)?;
    tag(&mut session, &[worktop], TOP_TAG)?;

    let (book, pages) = build_drawings(&mut session, &frame, worktop, &vise)?;
    let (frame_set, worktop_set) = build_export_sets(&mut session, worktop)?;
    let mut scenario = Scenario {
        session,
        parameters,
        steel,
        beech,
        rhs,
        flat,
        plate,
        top_stock,
        frame,
        worktop,
        vise,
        grid: grid_id,
        pilots: pilot_id,
        book,
        assembly_page: pages[0],
        weld_page: pages[1],
        drilling_page: pages[2],
        schedule_page: pages[3],
        frame_set,
        worktop_set,
    };
    refresh_tables(&mut scenario.session, scenario.book)?;
    Ok(scenario)
}

/// Set a project parameter's expression as one checked transaction.
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

/// Every page evaluated and checked, with each page's layout conflicts.
pub fn page_report(session: &Session, book: EntityId) -> Result<Vec<(String, Vec<String>)>, Error> {
    let evaluation = Compiler::default()
        .compile(session.document(), session.revision())?
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )?;
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

#[path = "support/model_preview.rs"]
mod model_preview;

pub fn render_preview(session: &Session, parts: &[EntityId]) -> Result<ColorImage, Error> {
    let document = session.document();
    let model = Compiler::default().compile(document, session.revision())?;
    let solids = parts
        .iter()
        .map(|&part| {
            let solid = model
                .snapshot()
                .bodies
                .get(&part)
                .and_then(|feature| model.solid(*feature))
                .ok_or("Part has no solid")?;
            Ok::<_, Error>((part, solid))
        })
        .collect::<Result<Vec<_>, _>>()?;
    model_preview::render_preview(document, solids)
}

/// Set the file name, description and rendered preview as one undoable
/// transaction through the shared file-information edits.
pub fn finish(s: &mut Scenario) -> Result<(), Error> {
    let image = render_preview(&s.session, &s.parts())?;
    let preview = multicad_ui::preview_capture::preview_from(&image)?;
    let edits = project_file::info::edits(
        s.session.document(),
        &FileInfo {
            name: DOCUMENT_NAME.into(),
            description: DESCRIPTION.into(),
            preview: Some(preview),
            capture_on_save: true,
        },
    )?;
    let prepared = s.session.prepare(Request {
        expected_revision: s.session.revision(),
        actor: actor(),
        label: "Set file information".into(),
        edits,
    })?;
    checked(&mut s.session, prepared)
}

/// The native container bytes of the current document.
pub fn archive(session: &Session) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
}

#[cfg(not(test))]
#[path = "../src/files.rs"]
#[allow(dead_code)]
mod files;

#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if let [flag, input, output] = arguments.as_slice()
        && flag == "--upgrade-vise"
    {
        return upgrade::run(std::path::Path::new(input), std::path::Path::new(output));
    }
    if let [flag, input, output] = arguments.as_slice()
        && flag == "--refresh-drawings"
    {
        return upgrade::refresh(std::path::Path::new(input), std::path::Path::new(output));
    }
    if arguments.len() != 1 {
        return Err(
            "Usage: workbench_demo NEW_FILE.tcd | --upgrade-vise INPUT.tcd NEW_FILE.tcd".into(),
        );
    }

    use multicad_ui::{application::DocumentStorage, storage::SaveMode};
    let path = std::env::args_os().nth(1).ok_or(
        "Usage: cargo run -p multicad-app --example workbench_demo -- NEW_FILE.tcd (for example examples/welded-workbench.tcd)",
    )?;
    let path = std::path::PathBuf::from(path);
    if !multicad_core::project_path::is_current_project_path(&path) {
        return Err("The output must be a .tcd file".into());
    }
    let mut scenario = build()?;
    finish(&mut scenario)?;
    let session = &scenario.session;
    let pages = page_report(session, scenario.book)?;
    let stale = stale_tables(session, scenario.book)?;
    if !stale.is_empty() {
        return Err(format!("Placed tables are stale: {stale:?}").into());
    }
    let saved = files::ProjectStore::default()
        .save_project(
            &path,
            &ProjectFile::new(session.document().clone())?,
            SaveMode::Create,
        )
        .map_err(|e| e.to_string())?;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let members = model.member_cut_list(session.document())?;
    let info = project_file::info::read(session.document())?;
    let preview = info.preview.as_ref().ok_or("No preview")?;
    let manifest = serde_json::json!({
        "generator": "workbench_demo",
        "file": path,
        "name": info.name,
        "preview_px": [preview.width(), preview.height()],
        "warnings": saved.warnings,
        "members": members.iter().map(|m| serde_json::json!({ "name": m.name, "length_mm": m.length_mm })).collect::<Vec<_>>(),
        "pages": pages.iter().map(|(n, c)| serde_json::json!({ "page": n, "conflicts": c })).collect::<Vec<_>>(),
        "verification": {
            "native_geometry": true,
            "interactive_macos": false,
            "human_acceptance": false,
            "structural_or_welding_certification": false,
        },
    });
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
