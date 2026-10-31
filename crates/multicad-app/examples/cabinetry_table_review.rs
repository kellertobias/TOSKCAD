//! The mixed-material fabrication slice in one native editable project: a
//! welded steel workshop table carrying wood panels. Project parameters
//! `table_width`, `table_depth`, `table_height` and `cabinet_width` drive
//! everything:
//!
//! - a base frame of SHS tube members from a constrained sketch rectangle
//!   with four mitered corners, a point-pair top frame with four mitered
//!   corners, four legs butted between both frames and a cross rail butted
//!   between the top rails, with 18 welds in an authored sequence;
//! - a laminated worktop stack (an MDF core with a trim oversize and a
//!   beech multiplex face layer with edge bands and allowances);
//! - a birch plywood cabinet carcass hung under the top, with shelf pin
//!   drilling, and two birch plywood upstands joined by a finger joint;
//! - purchased leveling feet, one Part placed four times;
//! - a drawing book with an assembly page (two views, associative
//!   dimensions), a weld plan with the authored sequence, and cut-list and
//!   schedule pages whose tables come from `multicad_model::schedules`;
//! - export sets for frame drawings and STEP, and panel 1:1 DXF and STEP,
//!   plus CSV schedules written by the schedule builders.
//!
//! The app test `tests/cabinetry_table.rs` builds the same project and
//! checks it.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{
        ConstraintKind, Dimension,
        binding::{self, DimensionInput},
        source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    drawings::*,
    exchange::*,
    inspection::GeometryReference,
    materials::*,
    model::{
        self, AxisAnchor, Back, Band, BodyOutput, CarcassDefinition, CarcassRequest,
        ComponentPlacementRequest, ConnectionDefinition, ConnectionKind, Construction, GrainAxis,
        MachiningDefinition, MachiningFace, MachiningKind, MemberDefinition, MemberPath,
        MemberRequest, NumericTransform, PanelDefinition, PanelEdges, PanelJointDefinition,
        PanelJointKind, PanelJointPair, PanelPlane, PanelRequest, PanelRole, PanelSource,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, SequenceDefinition, SequenceStep,
        ShelfLayout, Shelves, WeldDefinition, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
    sketch::Datum,
};
use multicad_model::{
    Compiler, DrawingProviders, Model,
    export::Exporter,
    group_cuts,
    schedules::{self, BomOptions, Schedule, Supply},
    weld_plan_annotations,
};
use std::{io::Write, path::Path};

pub type Error = Box<dyn std::error::Error>;
pub const FRAME_TAG: &str = "frame";
pub const PANEL_TAG: &str = "panel";
/// Worktop core plus face layer thickness; neither stock is edited.
pub const WORKTOP_MM: f64 = 19. + 12.;

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

/// Member Parts by role.
#[derive(Clone, Debug)]
pub struct Frame {
    /// Base rails from the sketch: front, right, back, left.
    pub base: Vec<EntityId>,
    /// Top rails: front, right, back, left.
    pub top: Vec<EntityId>,
    /// Legs: front left, front right, back right, back left.
    pub legs: Vec<EntityId>,
    pub cross: EntityId,
    pub sketch: EntityId,
    /// Connections in weld order, with their welds.
    pub connections: Vec<EntityId>,
    pub welds: Vec<EntityId>,
    pub sequence: EntityId,
}
impl Frame {
    pub fn members(&self) -> Vec<EntityId> {
        let mut all = self.base.clone();
        all.extend(&self.top);
        all.extend(&self.legs);
        all.push(self.cross);
        all
    }
}
pub struct Scenario {
    pub session: Session,
    pub table_width: EntityId,
    pub table_depth: EntityId,
    pub table_height: EntityId,
    pub cabinet_width: EntityId,
    pub steel: EntityId,
    pub shs: EntityId,
    pub ply: EntityId,
    pub mdf: EntityId,
    pub beech: EntityId,
    pub frame: Frame,
    pub top_core: EntityId,
    pub top_face: EntityId,
    pub worktop: EntityId,
    pub carcass: EntityId,
    pub cabinet: Vec<(PanelRole, EntityId)>,
    pub back_upstand: EntityId,
    pub left_upstand: EntityId,
    pub finger_joint: EntityId,
    pub shelf_pins: EntityId,
    pub foot: EntityId,
    pub foot_copies: Vec<EntityId>,
    pub book: EntityId,
    pub assembly_page: EntityId,
    pub weld_page: EntityId,
    pub cut_page: EntityId,
    pub schedule_page: EntityId,
    pub width_dimension: EntityId,
    pub height_dimension: EntityId,
    pub depth_dimension: EntityId,
    pub frame_set: EntityId,
    pub panel_set: EntityId,
}
impl Scenario {
    pub fn panels(&self) -> Vec<EntityId> {
        let mut all: Vec<_> = self.cabinet.iter().map(|(_, p)| *p).collect();
        all.extend([
            self.back_upstand,
            self.left_upstand,
            self.top_core,
            self.top_face,
        ]);
        all
    }
    pub fn cabinet_panel(&self, role: PanelRole) -> EntityId {
        self.cabinet
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, p)| *p)
            .expect("known cabinet role")
    }
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
        author: "MultiCAD".into(),
        project: "Workshop table".into(),
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

/// The SHS frame: base rectangle from a parameter-bound sketch, point-pair
/// top frame, legs and a cross rail, all connected and welded in order.
fn build_frame(session: &mut Session, shs: EntityId) -> Result<Frame, Error> {
    let template = MemberDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        stock: shs,
        path: MemberPath::Points {
            start: [length(0.), length(0.), length(0.)],
            end: [length(1.), length(0.), length(0.)],
        },
        anchor: [AxisAnchor::Center; 2],
        rotation: None,
        extend_start: None,
        extend_end: None,
    };

    // The base frame path: a W × D rectangle on the floor plane whose two
    // sides are bound to the table parameters.
    let (sketch, edits) = Source::create(Datum::Xy, "Base frame path")?;
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Create base frame path".into(),
        edits,
    })?;
    session.commit(prepared)?;
    let mut source = Source::read(session.document(), sketch)?.0;
    let bottom = source.add_rectangle([0., 0.], [1600., 800.])?;
    source.add_constraint(ConstraintKind::Fix {
        point: source.sketch.points[0].id,
    });
    let along = source.add_constraint(ConstraintKind::Length {
        line: bottom,
        dimension: length(1600.),
    });
    let across = source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: length(800.),
    });
    for (id, name) in [(along, "table_width"), (across, "table_depth")] {
        source = binding::draft_dimension(
            session.document(),
            &source,
            id,
            &DimensionInput::Expression {
                text: format!("=${name}"),
                unit: "mm".into(),
            },
            false,
        )?;
    }
    let edit = multicad_solver::prepare_edit(session, session.revision(), actor(), sketch, source)?;
    session.commit(edit.preview.ok_or("Base frame path did not solve")?)?;
    // The rails stand on the floor: the section's bottom face is on the path.
    let base_template = MemberDefinition {
        anchor: [AxisAnchor::Center, AxisAnchor::Min],
        ..template.clone()
    };
    let base =
        model::frame_member_requests(session.document(), sketch, "Base rail", &base_template)?;
    let prepared = model::prepare_members(session, session.revision(), actor(), &base, None)?;
    checked(session, prepared.prepared)?;

    // Top frame, legs and cross rail on parameter-driven point pairs.
    let z = "=$table_height - 20 mm";
    let corners = [
        ["#0", "#0", z],
        ["=$table_width", "#0", z],
        ["=$table_width", "=$table_depth", z],
        ["#0", "=$table_depth", z],
    ];
    let mut requests = vec![];
    for i in 0..4 {
        requests.push(MemberRequest::new(
            format!("Top rail {}", i + 1),
            MemberDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                path: MemberPath::Points {
                    start: point(session, corners[i])?,
                    end: point(session, corners[(i + 1) % 4])?,
                },
                ..template.clone()
            },
        ));
    }
    let legs = [
        ["=60 mm", "#0"],
        ["=$table_width - 60 mm", "#0"],
        ["=$table_width - 60 mm", "=$table_depth"],
        ["=60 mm", "=$table_depth"],
    ];
    for (i, [x, y]) in legs.into_iter().enumerate() {
        requests.push(MemberRequest::new(
            format!("Leg {}", i + 1),
            MemberDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                path: MemberPath::Points {
                    start: point(session, [x, y, "#0"])?,
                    end: point(session, [x, y, z])?,
                },
                ..template.clone()
            },
        ));
    }
    requests.push(MemberRequest::new(
        "Cross rail",
        MemberDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            path: MemberPath::Points {
                start: point(session, ["=$table_width / 2", "#0", z])?,
                end: point(session, ["=$table_width / 2", "=$table_depth", z])?,
            },
            ..template.clone()
        },
    ));
    let prepared = model::prepare_members(session, session.revision(), actor(), &requests, None)?;
    checked(session, prepared.prepared)?;
    let base: Vec<_> = base.iter().map(|r| r.body).collect();
    let top: Vec<_> = requests[0..4].iter().map(|r| r.body).collect();
    let legs: Vec<_> = requests[4..8].iter().map(|r| r.body).collect();
    let cross = requests[8].body;

    // Connections in the order they are welded.
    let mut joints: Vec<(String, ConnectionKind, [EntityId; 2], &str)> = vec![];
    for i in 0..4 {
        joints.push((
            format!("Top corner {}", i + 1),
            ConnectionKind::Miter,
            [top[i], top[(i + 1) % 4]],
            if i == 0 {
                "Tack corners, check diagonals"
            } else {
                ""
            },
        ));
    }
    joints.push((
        "Cross rail front".into(),
        ConnectionKind::Butt,
        [cross, top[0]],
        "",
    ));
    joints.push((
        "Cross rail back".into(),
        ConnectionKind::Butt,
        [cross, top[2]],
        "",
    ));
    for (i, leg) in legs.iter().enumerate() {
        // Legs stand under the front (0) and back (2) rails.
        let rail = if i < 2 { top[0] } else { top[2] };
        joints.push((
            format!("Leg {} top", i + 1),
            ConnectionKind::Butt,
            [*leg, rail],
            if i == 0 {
                "Frame inverted; alternate legs"
            } else {
                ""
            },
        ));
    }
    for i in 0..4 {
        joints.push((
            format!("Base corner {}", i + 1),
            ConnectionKind::Miter,
            [base[i], base[(i + 1) % 4]],
            if i == 0 {
                "Base flat; check for twist"
            } else {
                ""
            },
        ));
    }
    for (i, leg) in legs.iter().enumerate() {
        let rail = if i < 2 { base[0] } else { base[2] };
        joints.push((
            format!("Leg {} foot", i + 1),
            ConnectionKind::Butt,
            [*leg, rail],
            if i == 0 {
                "Plumb legs, then weld feet"
            } else {
                ""
            },
        ));
    }
    let mut connections = vec![];
    let mut welds = vec![];
    let mut steps = vec![];
    for (i, (name, kind, members, step_note)) in joints.into_iter().enumerate() {
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
                length: None,
                all_around: true,
                field: false,
                process: "135 MAG".into(),
                // Step notes carry the shop guidance; the weld plan table
                // column is narrow.
                notes: String::new(),
            },
        )?;
        checked(session, prepared)?;
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
        "Table frame sequence",
        &SequenceDefinition { steps },
    )?;
    checked(session, prepared)?;
    let all: Vec<_> = base
        .iter()
        .chain(&top)
        .chain(&legs)
        .copied()
        .chain([cross])
        .collect();
    tag(session, &all, FRAME_TAG)?;
    Ok(Frame {
        base,
        top,
        legs,
        cross,
        sketch,
        connections,
        welds,
        sequence,
    })
}

#[allow(clippy::too_many_arguments)]
fn rectangle_panel(
    session: &mut Session,
    name: &str,
    stock: EntityId,
    plane: PanelPlane,
    origin: [&str; 3],
    size: [&str; 2],
    grain: GrainAxis,
    edges: PanelEdges,
    oversize: Option<f64>,
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
            edges,
            oversize: oversize.map(length),
        },
    );
    let prepared = model::prepare_panel(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(request.body)
}
fn band(thickness: f64, allowance: f64) -> Band {
    Band {
        thickness: length(thickness),
        allowance: (allowance != 0.).then(|| length(allowance)),
    }
}

/// Every schedule placed on the drawing pages or written as CSV.
pub struct Tables {
    pub fabrication: Schedule,
    pub machining: Schedule,
    pub members: Schedule,
    pub bom: Schedule,
    pub welds: Schedule,
}
/// Every schedule of the current model, from the shared builders.
pub fn tables(session: &Session) -> Result<Tables, Error> {
    let (document, revision) = (session.document(), session.revision());
    let model = Compiler::default().compile(document, revision)?;
    let cuts = model.cut_list(document)?;
    let groups = group_cuts(&model.member_cut_list(document)?);
    let bom = model.bom(document, &BomOptions::default())?;
    Ok(Tables {
        fabrication: fabrication_cut_list(document, revision, &cuts, &groups, &bom)?,
        machining: schedules::machining_schedule(document, revision, &model.machining(document)?)?,
        members: schedules::member_cut_schedule(document, revision, &groups)?,
        bom: schedules::bom_schedule(&bom),
        welds: schedules::weld_schedule(document, revision, &model.weld_schedule(document)?)?,
    })
}
fn mm(v: f64) -> String {
    let r = (v * 10.).round() / 10.;
    if r == r.trunc() {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}
/// One cut list in three sections built from the authoritative records:
/// stock panels and stack layers, member cut groups and purchased Parts.
pub fn fabrication_cut_list(
    document: &Document,
    revision: u64,
    cuts: &[multicad_model::CutRecord],
    groups: &[multicad_model::CutGroup],
    bom: &schedules::Bom,
) -> Result<Schedule, Error> {
    let mut rows = vec![];
    for r in cuts {
        let stack = r
            .stack
            .and_then(|s| document.get(s))
            .map(|s| s.name.clone());
        rows.push(vec![
            stack
                .as_ref()
                .map_or("panel".to_string(), |s| format!("layer of {s}")),
            r.name.clone(),
            "1".into(),
            r.stock_name.clone(),
            mm(r.cut.thickness_mm),
            format!(
                "{} × {}",
                mm(r.cut.finished_mm[0]),
                mm(r.cut.finished_mm[1])
            ),
            format!("{} × {}", mm(r.cut.core_mm[0]), mm(r.cut.core_mm[1])),
            if r.cut.grain_fixed {
                "grain along L"
            } else {
                "grain free"
            }
            .into(),
            r.bands
                .iter()
                .map(|(edge, b)| match b.allowance_mm {
                    a if a > 0. => format!("{edge} {}+{}", mm(b.thickness_mm), mm(a)),
                    _ => format!("{edge} {}", mm(b.thickness_mm)),
                })
                .collect::<Vec<_>>()
                .join("; "),
        ]);
    }
    let end = |(kind, angle): &(String, f64), face: Option<&str>| match face {
        _ if kind == "square" => "square".to_string(),
        Some(face) => format!("{kind} {}° {face}", mm(*angle)),
        None => format!("{kind} {}°", mm(*angle)),
    };
    for (i, g) in groups.iter().enumerate() {
        rows.push(vec![
            "member".into(),
            format!("M{}", i + 1),
            g.quantity.to_string(),
            g.designation.clone(),
            String::new(),
            mm(g.length_mm),
            String::new(),
            format!(
                "{} / {}{}",
                end(&g.ends[0], g.long_faces[0]),
                end(&g.ends[1], g.long_faces[1]),
                g.long_points.map(|p| format!(", {p}")).unwrap_or_default()
            ),
            g.names.join(", "),
        ]);
    }
    for r in bom.rows.iter().filter(|r| r.supply == Supply::Purchased) {
        rows.push(vec![
            "purchased".into(),
            r.name.clone(),
            r.quantity.to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "buy, do not make".into(),
        ]);
    }
    Ok(Schedule {
        kind: "fabrication_cut_list",
        title: "Fabrication cut list".into(),
        revision,
        source: multicad_core::project_file::ContentHash::document(document)?,
        notes: vec![
            "Millimeters; panels: finished and core L × W, L along the grain; bands: edge thickness+allowance".into(),
            "Members (marks as in the member cut list): overall length long point to long point; ±x/±y is the long-point section face, then whether both long points share a face".into(),
            "Purchased Parts from the BOM; geometry records, not a fabrication approval".into(),
        ],
        columns: [
            "Section", "Item", "Qty", "Stock", "T", "Finished", "Core", "Cut / grain", "Notes",
        ]
        .map(String::from)
        .to_vec(),
        rows,
    })
}
/// Annotations for the cut-list and schedule pages and the weld plan.
fn schedule_annotations(
    session: &Session,
    weld_view: &DrawingView,
) -> Result<[Vec<Annotation>; 3], Error> {
    let t = tables(session)?;
    let table = |s: &Schedule, at: [f64; 2]| Annotation {
        id: EntityId::new(),
        kind: s.table(at),
    };
    let rows = |s: &Schedule| (s.rows.len() + 2) as f64 * 5.;
    let cut = vec![
        table(&t.fabrication, [15., 20.]),
        table(&t.machining, [15., 30. + rows(&t.fabrication)]),
        note(
            [15., 272.],
            "Geometry and output records only: not a structural, welding or fabrication certification",
        ),
    ];
    let schedule = vec![
        table(&t.members, [15., 20.]),
        table(&t.bom, [15., 30. + rows(&t.members)]),
    ];
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let mut welds = weld_plan_annotations(
        &model.weld_schedule(session.document())?,
        weld_view,
        [215., 20.],
    );
    welds.push(note(
        [15., 272.],
        "Weld order is authored documentation, not a welding procedure or distortion analysis",
    ));
    Ok([cut, schedule, welds])
}
/// Replace the weld plan and schedule tables with ones from the current
/// model; views and associative dimensions are left alone.
pub fn refresh_tables(s: &mut Scenario) -> Result<(), Error> {
    let book = multicad_core::drawings::read(s.session.document(), s.book)?;
    let weld_page = book
        .pages
        .iter()
        .find(|p| p.id == s.weld_page)
        .ok_or("Weld plan page is missing")?;
    let [cut, schedule, welds] = schedule_annotations(&s.session, &weld_page.views[0])?;
    for (id, annotations) in [
        (s.cut_page, cut),
        (s.schedule_page, schedule),
        (s.weld_page, welds),
    ] {
        let mut page = multicad_core::drawings::read(s.session.document(), s.book)?
            .pages
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("Page is missing")?;
        page.annotations = annotations;
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
/// Placed schedule tables that no longer match the current schedules.
pub fn stale_tables(s: &Scenario) -> Result<Vec<String>, Error> {
    let t = tables(&s.session)?;
    let book = multicad_core::drawings::read(s.session.document(), s.book)?;
    let mut stale = vec![];
    for page in &book.pages {
        for a in &page.annotations {
            let AnnotationKind::Table { title, .. } = &a.kind else {
                continue;
            };
            let fresh = [&t.fabrication, &t.machining, &t.members, &t.bom]
                .into_iter()
                .find(|s| title.starts_with(&s.title));
            if let Some(fresh) = fresh {
                if let Err(e) = fresh.check_table(&a.kind) {
                    stale.push(e);
                }
            } else if title == "Weld sequence" {
                let view = &page.views[0];
                let model =
                    Compiler::default().compile(s.session.document(), s.session.revision())?;
                let expected = weld_plan_annotations(
                    &model.weld_schedule(s.session.document())?,
                    view,
                    [0., 0.],
                );
                let rows_now = |k: &AnnotationKind| match k {
                    AnnotationKind::Table { rows, .. } => Some(rows.clone()),
                    _ => None,
                };
                let symbols = |list: &[Annotation]| {
                    list.iter()
                        .filter_map(|a| match &a.kind {
                            AnnotationKind::Weld { points_mm, .. } => Some(points_mm[0]),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                };
                if rows_now(&a.kind) != expected.last().and_then(|e| rows_now(&e.kind))
                    || symbols(&page.annotations) != symbols(&expected)
                {
                    stale.push("The weld plan is stale".into());
                }
            }
        }
    }
    Ok(stale)
}

pub fn build() -> Result<Scenario, Error> {
    let table_width = parameter("table_width", "1600", "Frame centerline span along X");
    let table_depth = parameter("table_depth", "800", "Frame centerline span along Y");
    let table_height = parameter("table_height", "900", "Floor to the top of the steel frame");
    let cabinet_width = parameter("cabinet_width", "500", "Under-top cabinet width");
    let mut session = Session::new(Document::new("Workshop table")?.edited(&[
        Mutation::Insert(table_width.clone()),
        Mutation::Insert(table_depth.clone()),
        Mutation::Insert(table_height.clone()),
        Mutation::Insert(cabinet_width.clone()),
    ])?)?;

    // Materials and stock: steel tube, birch plywood, MDF and beech
    // multiplex.
    let steel = material("S235 steel", 7850., [120, 124, 130, 255]);
    let birch = material("Birch plywood", 680., [214, 186, 140, 255]);
    let mdf = material("MDF", 750., [170, 140, 100, 255]);
    let beech = material("Beech multiplex", 720., [222, 184, 135, 255]);
    let shs = LibraryDefinition::Stock(Stock {
        material: steel.id,
        specification: StockKind::Profile {
            designation: "SHS 40×40×2".into(),
            length_mm: 6000.,
            width_mm: 40.,
            height_mm: 40.,
            wall_mm: Some(2.),
            shape: ProfileShape::Rectangular,
        },
    })
    .entity(EntityId::new(), "SHS 40×40×2")?;
    let ply = panel_stock(
        birch.id,
        "Birch ply 18",
        18.,
        [2500., 1250.],
        Grain::AlongLength,
    )?;
    let mdf_stock = panel_stock(mdf.id, "MDF 19", 19., [2800., 2070.], Grain::None)?;
    let beech_stock = panel_stock(
        beech.id,
        "Beech multiplex 12",
        12.,
        [2500., 1250.],
        Grain::AlongLength,
    )?;
    let ids = (steel.id, shs.id, ply.id, mdf_stock.id, beech_stock.id);
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[steel, birch, mdf, beech],
    )?;
    session.commit(prepared)?;
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[shs, ply, mdf_stock, beech_stock],
    )?;
    session.commit(prepared)?;
    let (steel, shs, ply, mdf, beech) = ids;

    let frame = build_frame(&mut session, shs)?;

    // Laminated worktop on the frame: an MDF core with a 3 mm trim
    // oversize and a beech face layer with bands on its front and ends.
    let top_core = rectangle_panel(
        &mut session,
        "Worktop core",
        mdf,
        PanelPlane::Xy,
        ["#-20", "#-20", "=$table_height"],
        ["=$table_width + 40 mm", "=$table_depth + 40 mm"],
        GrainAxis::Free,
        PanelEdges::default(),
        Some(3.),
    )?;
    let top_face = rectangle_panel(
        &mut session,
        "Worktop face",
        beech,
        PanelPlane::Xy,
        ["#-20", "#-20", "=$table_height + 19 mm"],
        ["=$table_width + 40 mm", "=$table_depth + 40 mm"],
        GrainAxis::U,
        PanelEdges {
            u_start: Some(band(2., 0.)),
            u_end: Some(band(2., 0.)),
            v_start: Some(band(2., 1.)),
            v_end: None,
        },
        None,
    )?;
    let (worktop, prepared) = model::prepare_stack(
        &session,
        session.revision(),
        actor(),
        "Worktop",
        &[top_core, top_face],
        Some("PVAc D3".into()),
        false,
    )?;
    checked(&mut session, prepared)?;

    // Birch plywood cabinet hung under the top frame, between the legs and
    // the cross rail, with a banded front and one shelf.
    let carcass = model::prepare_carcass(
        &session,
        session.revision(),
        actor(),
        &CarcassRequest::new(
            "Cabinet",
            CarcassDefinition {
                cabinet: None,
                origin: point(&session, ["#100", "#20", "=$table_height - 440 mm"])?,
                rotation: angle(0.),
                width: dim(&session, "=$cabinet_width")?,
                height: length(400.),
                depth: dim(&session, "=$table_depth - 40 mm")?,
                stock: ply,
                construction: Construction::SidesFullHeight,
                back: Back::None,
                shelves: Shelves {
                    layout: ShelfLayout::Even { count: 1 },
                    setback: length(20.),
                    clearance: length(0.),
                },
                front_band: Some(band(1., 0.5)),
                group: None,
            },
        ),
    )?;
    let carcass_id = carcass.carcass;
    checked(&mut session, carcass.model.prepared)?;
    let cabinet: Vec<(PanelRole, EntityId)> =
        model::carcass_panels(session.document(), carcass_id)?
            .into_iter()
            .map(|(role, _, part)| (role, part))
            .collect();
    let left_side = cabinet
        .iter()
        .find(|(r, _)| *r == PanelRole::LeftSide)
        .ok_or("Cabinet has no left side")?
        .1;
    let pins = model::prepare_machining(
        &session,
        session.revision(),
        actor(),
        "Shelf pins",
        &MachiningDefinition {
            generated_by: None,
            suppressed: false,
            kind: MachiningKind::Drill {
                holes: [[100., 100.], [100., 300.], [600., 100.], [600., 300.]]
                    .into_iter()
                    .map(|[u, v]| [length(u), length(v)])
                    .collect(),
                diameter: length(5.),
                depth: Some(length(10.)),
            },
            face: MachiningFace::Top,
            targets: vec![left_side],
        },
    )?;
    let shelf_pins = pins.operation;
    checked(&mut session, pins.model.prepared)?;

    // Two birch plywood upstands on the worktop, finger-jointed at the back
    // left corner. Their outer faces are fixed, so a thickness edit keeps
    // the corner block one full thickness of each panel.
    let z = format!("=$table_height + {WORKTOP_MM} mm");
    let back_upstand = rectangle_panel(
        &mut session,
        "Back upstand",
        ply,
        PanelPlane::Xz,
        ["#-20", "=$table_depth + 20 mm", &z],
        ["=$table_width + 40 mm", "#150"],
        GrainAxis::U,
        PanelEdges::default(),
        None,
    )?;
    let left_upstand = rectangle_panel(
        &mut session,
        "Left upstand",
        ply,
        PanelPlane::Yz,
        ["#-20", "=$table_depth - 280 mm", &z],
        ["#300", "#150"],
        GrainAxis::U,
        PanelEdges::default(),
        None,
    )?;
    let joint = model::prepare_panel_joint(
        &session,
        session.revision(),
        actor(),
        "Upstand corner",
        &PanelJointDefinition {
            generated_by: None,
            kind: PanelJointKind::Finger {
                count: 5,
                fit: length(0.2),
            },
            pairs: vec![PanelJointPair {
                a: back_upstand,
                b: left_upstand,
            }],
        },
    )?;
    let finger_joint = joint.joint;
    checked(&mut session, joint.model.prepared)?;
    let mut panels: Vec<_> = cabinet.iter().map(|(_, p)| *p).collect();
    panels.extend([back_upstand, left_upstand, top_core, top_face]);
    tag(&mut session, &panels, PANEL_TAG)?;

    // Purchased leveling feet: one Part under the front left leg, placed
    // at the other three legs.
    let foot = PrimitiveRequest::new(
        "Leveling foot M10",
        PrimitiveParameters::Cylinder {
            radius: length(16.),
            height: length(25.),
            frame: PrimitiveFrame {
                origin: [length(60.), length(0.), length(-25.)],
                ..Default::default()
            },
        },
    );
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &foot)?;
    checked(&mut session, prepared.prepared)?;
    tag(&mut session, &[foot.body], "purchased")?;
    let mut foot_copies = vec![];
    for [x, y] in [
        ["=$table_width - 120 mm", "#0"],
        ["=$table_width - 120 mm", "=$table_depth"],
        ["#0", "=$table_depth"],
    ] {
        let place = ComponentPlacementRequest::new(
            foot.body,
            NumericTransform {
                translation: [dim(&session, x)?, dim(&session, y)?, length(0.)],
                ..Default::default()
            },
        );
        let prepared =
            model::prepare_place_component(&session, session.revision(), actor(), &place)?;
        foot_copies.push(place.instance);
        checked(&mut session, prepared.prepared)?;
    }

    // Drawings: the assembly with associative dimensions, the weld plan
    // and two pages of schedule tables.
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let mut everything = frame.members();
    everything.extend(&panels);
    everything.push(foot.body);
    everything.extend(&foot_copies);
    let mut assembly = a3("T-101", "Workshop table assembly");
    let front = view(
        "Front",
        everything.clone(),
        [800., 0., 530.],
        [1., 0., 0.],
        [0., 0., 1.],
        [135., 95.],
    );
    let top = view(
        "Top",
        everything,
        [800., 400., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [135., 215.],
    );
    let width_dimension = dimension(
        front.id,
        face(&model, top_face, [-1., 0., 0.], 20.)?,
        face(&model, top_face, [1., 0., 0.], 1620.)?,
        LinearDirection::Horizontal,
        14.,
    );
    let height_dimension = dimension(
        front.id,
        face(&model, frame.base[0], [0., 0., -1.], 0.)?,
        face(&model, top_face, [0., 0., 1.], 900. + WORKTOP_MM)?,
        LinearDirection::Vertical,
        -30.,
    );
    let depth_dimension = dimension(
        top.id,
        face(&model, top_face, [0., -1., 0.], 20.)?,
        face(&model, top_face, [0., 1., 0.], 820.)?,
        LinearDirection::Vertical,
        -30.,
    );
    let dimension_ids = (width_dimension.id, height_dimension.id, depth_dimension.id);
    assembly.views = vec![front, top];
    assembly.annotations = vec![
        width_dimension,
        height_dimension,
        depth_dimension,
        note(
            [230., 30.],
            "Steel SHS 40×40×2 frame, MDF/beech worktop, birch ply cabinet and upstands",
        ),
        note(
            [230., 38.],
            "Geometry check only: not a structural or load certification",
        ),
    ];
    let assembly_page = assembly.id;
    let (book, prepared) = prepare_create(
        &session,
        session.revision(),
        actor(),
        "Workshop table",
        Some(assembly),
    )?;
    checked(&mut session, prepared)?;
    let mut weld = a3("T-102", "Frame weld plan");
    // An isometric view from the front left, so no two joints coincide.
    let (r2, r6) = (2_f64.sqrt(), 6_f64.sqrt());
    weld.views = vec![view(
        "Frame isometric",
        frame.members(),
        [800., 400., 450.],
        [1. / r2, -1. / r2, 0.],
        [1. / r6, 1. / r6, 2. / r6],
        [110., 118.],
    )];
    let cut = a3("T-103", "Cut lists");
    let schedule = a3("T-104", "Members and BOM");
    let ids = (weld.id, cut.id, schedule.id);
    for page in [weld, cut, schedule] {
        let prepared = prepare(
            &session,
            session.revision(),
            actor(),
            book,
            PageAction::Add(page),
        )?;
        checked(&mut session, prepared)?;
    }

    // Export sets: frame drawings and STEP; panel 1:1 DXF and STEP.
    let frame_set = EntityId::new();
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        frame_set,
        "Table frame and drawings",
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
                Recipe {
                    id: EntityId::new(),
                    name: "DXF".into(),
                    output: RecipeOutput::DrawingDxf {
                        units: LengthUnit::Millimeter,
                        layers: Layers::default(),
                    },
                },
                Recipe::step("STEP"),
            ],
            ..Default::default()
        },
    )?;
    session.commit(prepared)?;
    let panel_set = EntityId::new();
    let scope = Scope {
        root: ScopeRoot::Project,
        tags: Some(TagQuery {
            all: [PANEL_TAG.into()].into(),
            ..Default::default()
        }),
    };
    let dxf = Recipe {
        id: EntityId::new(),
        name: "Panel DXF".into(),
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
    for part in catalog.parts {
        // A broad face of the panel; of two broad faces the machined one,
        // whose holes make its area slightly smaller. The horizontal axis
        // lies in its plane.
        let broadest = part.faces.iter().map(|f| f.area_mm2).fold(0., f64::max);
        let face = part
            .faces
            .into_iter()
            .filter(|f| f.area_mm2 >= 0.98 * broadest)
            .min_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2))
            .ok_or("Panel has no planar face")?;
        let n = face.plane.normal();
        definition.planar_faces.push(PlanarFace {
            recipe: dxf.id,
            subject: part.subject,
            source: face.source,
            reference: face.intent.id,
            horizontal: if n[0].abs() > 0.9 {
                PlanarAxis::WorldY
            } else {
                PlanarAxis::WorldX
            },
        });
        links.push(Link {
            target: face.source,
            role: PLANAR_FACE_ROLE.into(),
            geometry: Some(face.intent),
        });
    }
    let prepared = prepare_set_with_geometry(
        &session,
        session.revision(),
        actor(),
        panel_set,
        "Panel cutting",
        &definition,
        &links,
    )?;
    checked(&mut session, prepared)?;

    let mut scenario = Scenario {
        session,
        table_width: table_width.id,
        table_depth: table_depth.id,
        table_height: table_height.id,
        cabinet_width: cabinet_width.id,
        steel,
        shs,
        ply,
        mdf,
        beech,
        frame,
        top_core,
        top_face,
        worktop,
        carcass: carcass_id,
        cabinet,
        back_upstand,
        left_upstand,
        finger_joint,
        shelf_pins,
        foot: foot.body,
        foot_copies,
        book,
        assembly_page,
        weld_page: ids.0,
        cut_page: ids.1,
        schedule_page: ids.2,
        width_dimension: dimension_ids.0,
        height_dimension: dimension_ids.1,
        depth_dimension: dimension_ids.2,
        frame_set,
        panel_set,
    };
    refresh_tables(&mut scenario)?;
    Ok(scenario)
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

pub fn archive(session: &Session) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
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
/// Change a panel stock's thickness as one checked library edit.
pub fn set_stock_thickness(
    session: &mut Session,
    stock: EntityId,
    thickness: f64,
) -> Result<(), Error> {
    let entity = session
        .document()
        .get(stock)
        .ok_or("Missing stock")?
        .clone();
    let Some(LibraryDefinition::Stock(mut spec)) = multicad_core::materials::read(&entity)? else {
        return Err("Not stock".into());
    };
    let StockKind::Panel { thickness_mm, .. } = &mut spec.specification else {
        return Err("Not panel stock".into());
    };
    *thickness_mm = thickness;
    let replaced = LibraryDefinition::Stock(spec).entity(stock, entity.name.clone())?;
    let prepared = prepare_library(session, session.revision(), actor(), &[replaced])?;
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

fn sha256(path: &Path) -> Result<String, Error> {
    let output = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()?;
    Ok(String::from_utf8(output.stdout)?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned())
}

/// Build the project and write it, its exports and its CSV schedules into
/// the new `directory`, with a manifest of file hashes.
pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let scenario = build()?;
    let project = directory.join("workshop-table.multicad");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&project)?;
    file.write_all(&archive(&scenario.session)?)?;
    file.sync_all()?;
    let session = &scenario.session;
    let pages = page_report(session, scenario.book)?;
    let csv = directory.join("schedules");
    std::fs::create_dir(&csv)?;
    let t = tables(session)?;
    let mut files = vec![project.clone()];
    for (name, schedule) in [
        ("fabrication-cut-list.csv", &t.fabrication),
        ("panel-cut-list.csv", &{
            let model = Compiler::default().compile(session.document(), session.revision())?;
            schedules::cut_list_schedule(
                session.document(),
                session.revision(),
                &model.cut_list(session.document())?,
            )?
        }),
        ("machining.csv", &t.machining),
        ("member-cut-list.csv", &t.members),
        ("weld-schedule.csv", &t.welds),
        ("bom.csv", &t.bom),
    ] {
        let path = csv.join(name);
        std::fs::write(&path, schedule.csv())?;
        files.push(path);
    }
    let exports = directory.join("exports");
    std::fs::create_dir(&exports)?;
    let exports = exports.canonicalize()?;
    for set in [scenario.frame_set, scenario.panel_set] {
        let receipt = export(session, set, &exports)?;
        for f in &receipt.files {
            files.push(Path::new(&receipt.directory).join(&f.artifact.output.filename));
        }
    }
    let hashes = files
        .iter()
        .map(|f| {
            Ok(serde_json::json!({
                "file": f.strip_prefix(directory).unwrap_or(f),
                "sha256": sha256(f)?,
            }))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let stacks = model.stacks(session.document())?;
    let manifest = serde_json::json!({
        "generator": "cabinetry_table_review",
        "project": project,
        "revision": session.revision(),
        "parameters": { "table_width": 1600, "table_depth": 800, "table_height": 900, "cabinet_width": 500 },
        "pages": pages.iter().map(|(n, c)| serde_json::json!({ "page": n, "conflicts": c })).collect::<Vec<_>>(),
        "stacks": stacks.iter().map(|s| serde_json::json!({ "stack": s.name, "total_thickness_mm": s.total_thickness_mm, "face_to_face": s.face_to_face, "issues": s.issues })).collect::<Vec<_>>(),
        "files": hashes,
        "verification": {
            "native_geometry": true,
            "outputs_read_back_by_tests": true,
            "interactive_macos": false,
            "windows_linux_runtime": false,
            "human_acceptance": false,
            "structural_or_welding_certification": false,
            "fabrication_approval": false,
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
    let path = std::env::args_os().nth(1).ok_or(
        "Usage: cargo run -p multicad-app --example cabinetry_table_review -- NEW_DIRECTORY",
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&generate(Path::new(&path))?)?
    );
    Ok(())
}
