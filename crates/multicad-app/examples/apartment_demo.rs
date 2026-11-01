//! Generator for the shipped apartment demo (CAD-185):
//! `examples/apartment.tcd` at the repository root.
//!
//! A small single-level apartment of about 10.3 × 8.3 m, built through the
//! typed transaction API that the window and the MCP server share, every
//! transaction compiled by the native kernel before it commits:
//!
//! - a 300 mm exterior wall loop and three 120 mm partitions forming four
//!   rooms: living room, kitchen, bedroom and workshop, with an entrance
//!   door, three interior doors and six windows;
//! - one floor slab on the "Apartment floor" level;
//! - simple placeholder furniture boxes in the living room, kitchen and
//!   bedroom;
//! - in the workshop, pinned project references to the shipped welded
//!   workbench (`examples/welded-workbench.tcd`) and the shipped cabinet
//!   (`examples/cabinet.tcd`);
//! - a drawing book: a dimensioned 1:50 floor plan with room names, hatched
//!   wall cuts and door swings, a fitted isometric 3D view and a workshop
//!   sheet naming the referenced sources and revisions, all with the
//!   standard title block;
//! - file name, description and a preview rendered headlessly from the
//!   native solids.
//!
//! Usage: `cargo run -p multicad-app --example apartment_demo -- OUTPUT.tcd`
//! (the output must not exist; it should sit next to the referenced
//! examples so the stored reference path is a bare file name). The app test
//! `tests/apartment_demo.rs` opens the shipped file and compares it with a
//! fresh build.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Placement},
    drawings::*,
    inspection::GeometryReference,
    materials::*,
    model::{
        self, BodyOutput, Hinge, Justification, LevelDefinition, Opening, OpeningKind,
        OpeningTypeDefinition, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        SlabDefinition, SlabKind, Swing, WallDefinition, WallRequest, WallTypeDefinition, length,
    },
    project_file::{self, ProjectFile, info::FileInfo},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
    symbols,
};
use multicad_kernel::MeshOptions;
use multicad_model::{Compiler, Model, project_references};
use multicad_ui::egui::{Color32, ColorImage};
use std::path::{Path, PathBuf};

pub type Error = Box<dyn std::error::Error>;
pub const DOCUMENT_NAME: &str = "Apartment with workshop";
pub const DESCRIPTION: &str = "Apartment demo: a single-level apartment of about 10.3 × 8.3 m \
with a living room, kitchen, bedroom and workshop, built from editable walls, doors, windows and \
a floor slab. The workshop holds pinned references to the shipped cabinet and welded workbench examples; \
each reference keeps its source file, revision and scale and reports a changed or missing source. \
The drawing book has a dimensioned 1:50 floor plan with room names, hatched wall cuts and door \
swings, a furnished 3D view and a workshop sheet. Layout demo only: no building-code, egress or \
structural compliance is claimed.";
/// Where the shipped file lives, relative to the repository root.
pub const SHIPPED_PATH: &str = "examples/apartment.tcd";
/// The referenced workbench example, relative to the repository root.
pub const WORKBENCH_SOURCE: &str = "examples/welded-workbench.tcd";
/// The referenced cabinet example (CAD-182), relative to the repository root.
pub const CABINET_SOURCE: &str = "examples/cabinet.tcd";
pub const DISCLAIMER: &str =
    "Layout demo only: no building-code, egress, accessibility or structural compliance is claimed";

/// Wall centerline rectangle of the exterior loop, in millimeters.
pub const OUTER: [f64; 2] = [10_000., 8_000.];
pub const EXTERIOR_MM: f64 = 300.;
pub const PARTITION_MM: f64 = 120.;
pub const WALL_HEIGHT_MM: f64 = 2_600.;
/// Partition centerlines: the long partition between front and back rooms,
/// the living/kitchen partition and the bedroom/workshop partition.
pub const LONG_Y: f64 = 4_400.;
pub const KITCHEN_X: f64 = 6_000.;
pub const WORKSHOP_X: f64 = 4_500.;

/// Clear interior rectangles `[x0, y0, x1, y1]` of the four rooms.
pub const ROOMS: [(&str, [f64; 4]); 4] = [
    ("Living room", [150., 150., KITCHEN_X - 60., LONG_Y - 60.]),
    (
        "Kitchen",
        [KITCHEN_X + 60., 150., OUTER[0] - 150., LONG_Y - 60.],
    ),
    (
        "Bedroom",
        [150., LONG_Y + 60., WORKSHOP_X - 60., OUTER[1] - 150.],
    ),
    (
        "Workshop",
        [
            WORKSHOP_X + 60.,
            LONG_Y + 60.,
            OUTER[0] - 150.,
            OUTER[1] - 150.,
        ],
    ),
];
/// The workbench stands against the workshop's back wall under its window:
/// its 1500 × 1000 top spans x 5150–6650, y 6800–7800.
pub const WORKBENCH_PLACEMENT: Placement = Placement {
    translation_mm: [5_150., 6_800., 0.],
    rotation_xyzw: [0., 0., 0., 1.],
};
/// The cabinet's origin is its front-left-bottom carcass corner, width
/// along +X, depth along +Y (front facing −Y; doors stand 18 mm proud).
/// Turned −90° about Z its front faces the room (−X) and its back
/// (local y = 500) stands against the workshop's right wall (inner face
/// x = 9850); its 800 mm width spans y 5400–6200.
pub const CABINET_PLACEMENT: Placement = Placement {
    translation_mm: [9_350., 6_200., 0.],
    rotation_xyzw: [
        0.,
        0.,
        -std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ],
};
pub const PAGES: [&str; 3] = ["AP-101", "AP-102", "AP-103"];

pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
/// The repository root, where `examples/` lives.
pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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
fn point(x: f64, y: f64) -> [multicad_core::constraint_sketch::Dimension; 2] {
    [length(x), length(y)]
}
fn opening(opening_type: EntityId, segment: usize, offset: f64) -> Opening {
    Opening {
        id: EntityId::new(),
        opening_type,
        segment,
        offset: length(offset),
        width: None,
        height: None,
        sill: None,
    }
}

/// Opening types by role.
#[derive(Clone, Copy, Debug)]
pub struct Openings {
    pub entrance: EntityId,
    /// Interior door swinging to the left of its wall path.
    pub door_left: EntityId,
    /// Interior door swinging to the right of its wall path.
    pub door_right: EntityId,
    pub window: EntityId,
}
/// One placed project reference.
#[derive(Clone, Debug)]
pub struct Reference {
    pub label: &'static str,
    pub instance: EntityId,
    pub source: &'static str,
}
pub struct Scenario {
    pub session: Session,
    pub level: EntityId,
    pub exterior_type: EntityId,
    pub partition_type: EntityId,
    pub openings: Openings,
    /// Exterior loop, long partition, kitchen partition, workshop partition.
    pub walls: [EntityId; 4],
    pub slab: EntityId,
    pub furniture: Vec<EntityId>,
    pub references: Vec<Reference>,
    pub book: EntityId,
    pub pages: [EntityId; 3],
}
impl Scenario {
    /// Parts drawn in the plan and 3D views, plus the reference Instances.
    pub fn plan_sources(&self) -> Vec<EntityId> {
        let mut all = self.walls.to_vec();
        all.extend(&self.furniture);
        all.extend(self.references.iter().map(|r| r.instance));
        all
    }
}

fn wall_type(
    session: &mut Session,
    name: &str,
    thickness: f64,
    material: EntityId,
) -> Result<EntityId, Error> {
    let (id, prepared) = model::prepare_wall_type(
        session,
        session.revision(),
        actor(),
        name,
        &WallTypeDefinition {
            thickness: length(thickness),
            height: length(WALL_HEIGHT_MM),
            justification: Justification::Center,
            material: Some(material),
        },
    )?;
    session.commit(prepared)?;
    Ok(id)
}
fn opening_type(
    session: &mut Session,
    name: &str,
    kind: OpeningKind,
    size: [f64; 3],
) -> Result<EntityId, Error> {
    let (id, prepared) = model::prepare_opening_type(
        session,
        session.revision(),
        actor(),
        name,
        &OpeningTypeDefinition {
            kind,
            width: length(size[0]),
            height: length(size[1]),
            sill: length(size[2]),
        },
    )?;
    session.commit(prepared)?;
    Ok(id)
}
#[allow(clippy::too_many_arguments)]
fn wall(
    session: &mut Session,
    name: &str,
    wall_type: EntityId,
    level: EntityId,
    path: Vec<[f64; 2]>,
    closed: bool,
    openings: Vec<Opening>,
) -> Result<EntityId, Error> {
    let request = WallRequest::new(
        name,
        WallDefinition {
            layer_placement: None,
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type,
            path: path.into_iter().map(|[x, y]| point(x, y)).collect(),
            closed,
            level: Some(level),
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings,
            cutouts: vec![],
        },
    );
    let body = request.wall.output.body;
    let prepared = model::prepare_wall(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    Ok(body)
}
fn furniture(
    session: &mut Session,
    name: &str,
    origin: [f64; 2],
    size: [f64; 3],
    material: EntityId,
) -> Result<EntityId, Error> {
    let request = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: size.map(length),
            frame: PrimitiveFrame {
                origin: [length(origin[0]), length(origin[1]), length(0.)],
                ..Default::default()
            },
        },
    );
    let body = request.body;
    let prepared = model::prepare_primitive(session, session.revision(), actor(), &request)?;
    checked(session, prepared.prepared)?;
    let prepared = prepare_assignment(
        session,
        session.revision(),
        actor(),
        &[body],
        &Assignment {
            material: Some(material),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )?;
    session.commit(prepared)?;
    Ok(body)
}

/// Place a pinned reference to the example at `source` (relative to the
/// repository root) as seen from the apartment file at `output`.
pub fn place_reference(
    session: &mut Session,
    output: &Path,
    source: &str,
    label: &str,
    placement: Placement,
) -> Result<EntityId, Error> {
    let prepared = project_references::prepare_reference_file(
        session,
        session.revision(),
        actor(),
        Some(output),
        &repository().join(source),
        label,
        placement,
    )?;
    let instance = prepared.instances[0];
    checked(session, prepared.prepared)?;
    Ok(instance)
}

/// Build the apartment as if saved at `output` (which decides the stored
/// relative reference paths).
pub fn build(output: &Path) -> Result<Scenario, Error> {
    let mut session = Session::new(Document::new(DOCUMENT_NAME)?)?;

    // Materials: masonry with a section hatch for the exterior, plaster
    // board (default wall hatch) for partitions, screed for the floor, and
    // plain finishes for the furniture placeholders.
    let masonry = material("Masonry", 1800., [205, 196, 182, 255]);
    let plaster = material("Plaster board", 900., [236, 233, 226, 255]);
    let screed = material("Screed", 2000., [176, 172, 165, 255]);
    let fabric = material("Upholstery", 300., [96, 118, 140, 255]);
    let oak = material("Oak", 700., [178, 138, 92, 255]);
    let laminate = material("White laminate", 800., [240, 240, 236, 255]);
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        pattern: HatchPattern::Lines,
        angle_deg: 45.,
        spacing_mm: 1.,
        line_width_mm: 0.13,
    })
    .entity(EntityId::new(), "Masonry section")?;
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[
            masonry.clone(),
            plaster.clone(),
            screed.clone(),
            fabric.clone(),
            oak.clone(),
            laminate.clone(),
            hatch.clone(),
        ],
    )?;
    session.commit(prepared)?;
    let style = LibraryDefinition::MaterialStyle(MaterialStyle {
        material: masonry.id,
        appearance: None,
        hatch: Some(LibraryRef::capture(session.document(), hatch.id)?),
    })
    .entity(EntityId::new(), "Masonry defaults")?;
    let prepared = prepare_library(&session, session.revision(), actor(), &[style])?;
    session.commit(prepared)?;

    let (level, prepared) = model::prepare_level(
        &session,
        session.revision(),
        actor(),
        "Apartment floor",
        &LevelDefinition {
            elevation: length(0.),
            height: length(WALL_HEIGHT_MM),
            slab_thickness: length(200.),
        },
    )?;
    session.commit(prepared)?;

    let exterior_type = wall_type(
        &mut session,
        "Exterior masonry 300",
        EXTERIOR_MM,
        masonry.id,
    )?;
    let partition_type = wall_type(&mut session, "Partition 120", PARTITION_MM, plaster.id)?;
    let openings = Openings {
        entrance: opening_type(
            &mut session,
            "Entrance door 1000",
            OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Left,
            },
            [1000., 2100., 0.],
        )?,
        door_left: opening_type(
            &mut session,
            "Interior door 900 (left)",
            OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Left,
            },
            [900., 2100., 0.],
        )?,
        door_right: opening_type(
            &mut session,
            "Interior door 900 (right)",
            OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Right,
            },
            [900., 2100., 0.],
        )?,
        window: opening_type(
            &mut session,
            "Window 1200 high",
            OpeningKind::Window,
            [1600., 1200., 900.],
        )?,
    };
    let window = |segment, offset, width: f64| Opening {
        width: Some(length(width)),
        ..opening(openings.window, segment, offset)
    };

    // The exterior loop runs counter-clockwise from the front left corner:
    // segment 0 front (+X), 1 right (+Y), 2 back (−X), 3 left (−Y). A door
    // swinging "left" of a segment swings into the apartment.
    let [w, d] = OUTER;
    let exterior = wall(
        &mut session,
        "Exterior walls",
        exterior_type,
        level,
        vec![[0., 0.], [w, 0.], [w, d], [0., d]],
        true,
        vec![
            opening(openings.entrance, 0, 800.),
            window(0, 2600., 2000.),
            window(0, 7000., 1600.),
            window(1, 1400., 1400.),
            window(2, 3300., 1600.),
            window(2, 6800., 1600.),
            window(3, 5000., 1600.),
        ],
    )?;
    // The long partition runs +X between the exterior faces; its doors
    // swing left, into the bedroom and the workshop.
    let long = wall(
        &mut session,
        "Partition living / bedroom / workshop",
        partition_type,
        level,
        vec![[150., LONG_Y], [w - 150., LONG_Y]],
        false,
        vec![
            opening(openings.door_left, 0, 1850.),
            opening(openings.door_left, 0, 4750.),
        ],
    )?;
    // The kitchen partition runs +Y from the front wall to the long
    // partition; its door swings right, into the kitchen.
    let kitchen = wall(
        &mut session,
        "Partition living / kitchen",
        partition_type,
        level,
        vec![[KITCHEN_X, 150.], [KITCHEN_X, LONG_Y - 60.]],
        false,
        vec![opening(openings.door_right, 0, 1450.)],
    )?;
    let workshop = wall(
        &mut session,
        "Partition bedroom / workshop",
        partition_type,
        level,
        vec![[WORKSHOP_X, LONG_Y + 60.], [WORKSHOP_X, d - 150.]],
        false,
        vec![],
    )?;

    let floor = SlabDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        level,
        kind: SlabKind::Slab,
        outline: vec![
            point(-150., -150.),
            point(w + 150., -150.),
            point(w + 150., d + 150.),
            point(-150., d + 150.),
        ],
        thickness: None,
        holes: vec![],
    };
    let prepared = model::prepare_slab(
        &session,
        session.revision(),
        actor(),
        "Floor slab",
        &floor,
        Some(screed.id),
    )?;
    let slab = floor.output.body;
    checked(&mut session, prepared.prepared)?;

    // Placeholder furniture: plain boxes, not manufacturing data.
    let furniture = vec![
        furniture(
            &mut session,
            "Sofa",
            [350., 1800.],
            [900., 2200., 800.],
            fabric.id,
        )?,
        furniture(
            &mut session,
            "Coffee table",
            [1700., 2400.],
            [1100., 600., 400.],
            oak.id,
        )?,
        furniture(
            &mut session,
            "Bed",
            [1800., 5800.],
            [1600., 2000., 500.],
            fabric.id,
        )?,
        furniture(
            &mut session,
            "Wardrobe",
            [3780., 5000.],
            [600., 1800., 2200.],
            oak.id,
        )?,
        furniture(
            &mut session,
            "Kitchen counter",
            [9250., 600.],
            [600., 3400., 900.],
            laminate.id,
        )?,
        furniture(
            &mut session,
            "Dining table",
            [6800., 2600.],
            [1400., 800., 750.],
            oak.id,
        )?,
    ];

    // The workshop's references to the shipped examples.
    let mut references = vec![Reference {
        label: "Workbench",
        instance: place_reference(
            &mut session,
            output,
            WORKBENCH_SOURCE,
            "Workbench",
            WORKBENCH_PLACEMENT,
        )?,
        source: WORKBENCH_SOURCE,
    }];
    references.push(Reference {
        label: "Cabinet",
        instance: place_reference(
            &mut session,
            output,
            CABINET_SOURCE,
            "Cabinet",
            CABINET_PLACEMENT,
        )?,
        source: CABINET_SOURCE,
    });

    let mut scenario = Scenario {
        session,
        level,
        exterior_type,
        partition_type,
        openings,
        walls: [exterior, long, kitchen, workshop],
        slab,
        furniture,
        references,
        book: EntityId::nil(),
        pages: [EntityId::nil(); 3],
    };
    build_drawings(&mut scenario)?;
    Ok(scenario)
}

/// A planar face of a Part or Instance whose outward normal matches
/// `normal` and whose plane passes through coordinate `at` along it.
pub fn face(
    model: &Model,
    entity: EntityId,
    normal: [f64; 3],
    at: f64,
) -> Result<GeometryReference, Error> {
    let feature = model
        .snapshot()
        .bodies
        .get(&entity)
        .ok_or_else(|| format!("{entity} has no final body"))?;
    let mut topology = model.snapshot().shapes[feature].topology.clone();
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
fn note(position: [f64; 2], text: &str) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: position,
            text: text.into(),
        },
    }
}
/// A note centered on a paper point (2.5 mm monospaced text).
fn centered(paper: [f64; 2], text: &str) -> Annotation {
    let width = text.chars().count() as f64 * ANNOTATION_TEXT_MM * 0.6;
    note([paper[0] - width / 2., paper[1]], text)
}
fn view(
    name: &str,
    entities: Vec<EntityId>,
    origin: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
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
        position_mm: [150., 120.],
        scale: Scale {
            numerator: 1,
            denominator: 50,
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
        denominator: 50,
    };
    page.info = DrawingInfo {
        title: title.into(),
        author: "MultiCAD".into(),
        project: DOCUMENT_NAME.into(),
        revision: "A".into(),
        checker: "Pending human review".into(),
    };
    page
}
/// Fit one view the way the native Fit command does, around the model
/// extent `min..max` in view coordinates (relative to the frame origin).
fn fit(
    page: &mut Page,
    index: usize,
    min: [f64; 2],
    max: [f64; 2],
    mode: FitMode,
) -> Result<(), Error> {
    let layout = layout_page(page, 1, 1)?;
    let view = &mut page.views[index];
    let placed = fit_view(view, &layout, min, max, mode)?;
    for i in 0..3 {
        view.frame.origin_mm[i] +=
            view.frame.u[i] * placed.center_mm[0] + view.frame.v[i] * placed.center_mm[1];
    }
    if let ViewKind::Detail { center_mm, .. } = &mut view.kind {
        center_mm[0] -= placed.center_mm[0];
        center_mm[1] -= placed.center_mm[1];
    }
    view.scale = placed.scale;
    view.position_mm = placed.position_mm;
    Ok(())
}
/// The view's evaluated extent in view coordinates.
fn extent(evaluation: &DrawingEvaluation, view: EntityId) -> Result<([f64; 2], [f64; 2]), Error> {
    let Some(ViewEvaluation::Ready { vectors, .. }) = evaluation.views.get(&view) else {
        return Err(format!(
            "View {view} did not evaluate: {:?}",
            evaluation.views.get(&view)
        )
        .into());
    };
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in vectors.iter().flat_map(|v| &v.preview_mm) {
        for i in 0..2 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    Ok((min, max))
}
/// A plan point on paper for a view looking down the world Z axis.
fn on_plan(view: &DrawingView, x: f64, y: f64) -> [f64; 2] {
    let o = view.frame.origin_mm;
    view.paper_point([x - o[0], y - o[1]])
}
fn area_m2(r: [f64; 4]) -> f64 {
    (r[2] - r[0]) * (r[3] - r[1]) / 1e6
}

fn build_drawings(s: &mut Scenario) -> Result<(), Error> {
    let sources = s.plan_sources();
    let [w, d] = OUTER;
    let center = [w / 2., d / 2., 0.];
    let mut plan = a3(PAGES[0], "Floor plan");
    plan.views = vec![view(
        "Floor plan",
        sources.clone(),
        center,
        [1., 0., 0.],
        [0., 1., 0.],
    )];
    let (r2, r6) = (2_f64.sqrt(), 6_f64.sqrt());
    let mut iso = a3(PAGES[1], "Furnished apartment, 3D view");
    let mut all = sources.clone();
    all.push(s.slab);
    iso.views = vec![view(
        "Isometric",
        all,
        [center[0], center[1], 1000.],
        [1. / r2, -1. / r2, 0.],
        [1. / r6, 1. / r6, 2. / r6],
    )];
    let mut workshop = a3(PAGES[2], "Workshop and referenced examples");
    // The referenced objects at 1:25, from above and from the front: the
    // workbench (x 5150–6650) and the cabinet (x 9332–9850), centered on
    // their common extent.
    let refs: Vec<_> = s.references.iter().map(|r| r.instance).collect();
    let center = [7_500., 6_600., 1_000.];
    let mut top = view(
        "Workshop references, top",
        refs.clone(),
        [center[0], center[1], 0.],
        [1., 0., 0.],
        [0., 1., 0.],
    );
    let mut front = view(
        "Workshop references, front",
        refs,
        [center[0], 0., center[2]],
        [1., 0., 0.],
        [0., 0., 1.],
    );
    let quarter = Scale {
        numerator: 1,
        denominator: 25,
    };
    for (v, y) in [(&mut top, 100.), (&mut front, 200.)] {
        v.scale = quarter;
        v.position_mm = [130., y];
    }
    workshop.settings.scale = quarter;
    workshop.views = vec![top, front];
    let ids = [plan.id, iso.id, workshop.id];
    let (book, prepared) = prepare_create(
        &s.session,
        s.session.revision(),
        actor(),
        DOCUMENT_NAME,
        Some(plan),
    )?;
    checked(&mut s.session, prepared)?;
    for page in [iso, workshop] {
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
    s.pages = ids;

    // Fit every view to its sheet with the native Fit steps, leaving room
    // for the plan's dimension chains.
    let model = Compiler::default().compile(s.session.document(), s.session.revision())?;
    let evaluation = model.drawings(
        s.session.document(),
        s.session.revision(),
        &Default::default(),
    )?;
    let current = multicad_core::drawings::read(s.session.document(), book)?;
    let mut pages = current.pages.clone();
    for (index, page) in pages.iter_mut().enumerate().take(2) {
        let (mut min, mut max) = extent(&evaluation, page.views[0].id)?;
        let mode = if index == 0 {
            // Dimension chains need about 1.1 m of model space (22 mm of
            // paper) on each side; the plan keeps its 1:50 scale.
            for i in 0..2 {
                min[i] -= 1100.;
                max[i] += 1100.;
            }
            FitMode::KeepScale
        } else {
            for i in 0..2 {
                min[i] -= 300.;
                max[i] += 300.;
            }
            FitMode::Standard
        };
        fit(page, 0, min, max, mode)?;
    }

    // Plan annotations: room names and areas, overall and room dimensions.
    let plan = &mut pages[0];
    let v = plan.views[0].clone();
    let walls = s.walls;
    let exterior = walls[0];
    let mut notes = vec![];
    for (name, r) in ROOMS {
        let c = on_plan(&v, (r[0] + r[2]) / 2., (r[1] + r[3]) / 2.);
        notes.push(centered([c[0], c[1] - 2.], &name.to_uppercase()));
        notes.push(centered(
            [c[0], c[1] + 3.5],
            &format!("{:.1} m²", area_m2(r)),
        ));
    }
    let h = LinearDirection::Horizontal;
    let vert = LinearDirection::Vertical;
    let mut annotations = vec![
        // Overall outside dimensions.
        dimension(
            v.id,
            face(&model, exterior, [-1., 0., 0.], 150.)?,
            face(&model, exterior, [1., 0., 0.], w + 150.)?,
            h,
            20.,
        ),
        dimension(
            v.id,
            face(&model, exterior, [0., -1., 0.], 150.)?,
            face(&model, exterior, [0., 1., 0.], d + 150.)?,
            vert,
            -22.,
        ),
        // Clear widths of the front rooms (below) and back rooms (above).
        dimension(
            v.id,
            face(&model, exterior, [1., 0., 0.], 150.)?,
            face(&model, walls[2], [-1., 0., 0.], -(KITCHEN_X - 60.))?,
            h,
            -16.,
        ),
        dimension(
            v.id,
            face(&model, walls[2], [1., 0., 0.], KITCHEN_X + 60.)?,
            face(&model, exterior, [-1., 0., 0.], -(w - 150.))?,
            h,
            -16.,
        ),
        dimension(
            v.id,
            face(&model, exterior, [1., 0., 0.], 150.)?,
            face(&model, walls[3], [-1., 0., 0.], -(WORKSHOP_X - 60.))?,
            h,
            10.,
        ),
        dimension(
            v.id,
            face(&model, walls[3], [1., 0., 0.], WORKSHOP_X + 60.)?,
            face(&model, exterior, [-1., 0., 0.], -(w - 150.))?,
            h,
            10.,
        ),
        // Clear depths of the front and back rooms, on the right.
        dimension(
            v.id,
            face(&model, exterior, [0., 1., 0.], 150.)?,
            face(&model, walls[1], [0., -1., 0.], -(LONG_Y - 60.))?,
            vert,
            -14.,
        ),
        dimension(
            v.id,
            face(&model, walls[1], [0., 1., 0.], LONG_Y + 60.)?,
            face(&model, exterior, [0., -1., 0.], -(d - 150.))?,
            vert,
            -14.,
        ),
    ];
    annotations.extend(notes);
    // Below the plan, left of the title block, clear of the dimensions.
    annotations.push(note(
        [20., 262.],
        "Walls cut 1000 mm above the floor. Exterior 300 masonry, partitions 120. Furniture outlines are placeholders",
    ));
    annotations.push(note([20., 268.], DISCLAIMER));
    plan.annotations = annotations;

    let iso_page = &mut pages[1];
    iso_page.annotations = vec![
        note(
            [20., 20.],
            "Furnished apartment: living room, kitchen, bedroom and workshop with the referenced cabinet and workbench",
        ),
        note([20., 26.], DISCLAIMER),
    ];

    let shop_page = &mut pages[2];
    let mut lines = vec![note(
        [20., 20.],
        "Workshop: the objects below are pinned references to shipped example projects, not copies",
    )];
    let document = s.session.document();
    for (i, r) in s.references.iter().enumerate() {
        let record = symbols::read_instance(document, r.instance)?;
        let definition = symbols::read_definition(document, record.definition)?;
        let source = symbols::project_source_of(document, record.definition)?
            .ok_or("Reference has no source record")?
            .1;
        lines.push(note(
            [20., 28. + 6. * i as f64],
            &format!(
                "{}: {} · {} · revision {}",
                r.label, definition.name, source.path, definition.revision
            ),
        ));
    }
    shop_page.annotations = lines;

    for page in pages {
        let prepared = prepare(
            &s.session,
            s.session.revision(),
            actor(),
            book,
            PageAction::Replace(page),
        )?;
        checked(&mut s.session, prepared)?;
    }
    Ok(())
}

const PREVIEW: [usize; 2] = [1280, 960];
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    if l > 0. { a.map(|c| c / l) } else { a }
}
fn part_color(document: &Document, part: EntityId) -> [u8; 4] {
    match document.get(part).map(|e| &e.data) {
        Some(EntityData::Part {
            material: Some(m), ..
        }) => match document.get(*m).map(|e| &e.data) {
            Some(EntityData::Material { color_rgba, .. }) => *color_rgba,
            _ => [160, 160, 160, 255],
        },
        _ => [160, 160, 160, 255],
    }
}

/// Render the apartment in an orthographic bird's-eye view from the front
/// left, with walls cut down to 1.2 m so the rooms and the furnished
/// workshop are visible: every Part and every reference leaf in its
/// material color, flat-shaded.
pub fn render_preview(s: &Scenario) -> Result<ColorImage, Error> {
    let document = s.session.document();
    let model = Compiler::default().compile(document, s.session.revision())?;
    let toward = normalize([-0.8, -1.3, 1.9]);
    let right = normalize(cross([0., 0., 1.], toward));
    let up = cross(toward, right);
    let light = normalize([-0.4, -0.8, 1.]);
    let options = MeshOptions {
        linear_deflection_mm: 2.,
        angular_deflection_rad: 0.4,
        maximum_triangles: 4_000_000,
    };
    let mut triangles: Vec<([[f64; 3]; 3], [u8; 4])> = vec![];
    let mut add =
        |solid: &multicad_kernel::Solid, color: [u8; 4], clip: Option<f64>| -> Result<(), Error> {
            let mesh = solid.mesh(options)?;
            let v = |i: u32| {
                let i = i as usize * 3;
                [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]]
            };
            for t in mesh.indices.as_chunks::<3>().0 {
                let mut tri = [v(t[0]), v(t[1]), v(t[2])];
                if let Some(z) = clip {
                    for p in &mut tri {
                        p[2] = p[2].min(z);
                    }
                }
                triangles.push((tri, color));
            }
            Ok(())
        };
    let mut parts = s.walls.to_vec();
    parts.push(s.slab);
    parts.extend(&s.furniture);
    for part in &parts {
        let feature = model.snapshot().bodies[part];
        let solid = model.solid(feature).ok_or("Part has no solid")?;
        let clip = s.walls.contains(part).then_some(1200.);
        add(solid, part_color(document, *part), clip)?;
    }
    let roots: Vec<_> = s.references.iter().map(|r| r.instance).collect();
    for (path, leaf) in &model.snapshot().leaf_occurrences {
        if !roots.contains(&path.root) {
            continue;
        }
        let solid = model
            .leaf_solid(path)
            .ok_or("Reference leaf has no solid")?;
        add(solid, part_color(document, leaf.component), None)?;
    }
    let project = |p: [f64; 3]| [dot(p, right), dot(p, up), dot(p, toward)];
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (t, _) in &triangles {
        for p in t {
            let q = project(*p);
            for i in 0..2 {
                lo[i] = lo[i].min(q[i]);
                hi[i] = hi[i].max(q[i]);
            }
        }
    }
    let [w, h] = PREVIEW;
    let margin = 0.04;
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
    let mut pixels: Vec<Color32> = (0..w * h)
        .map(|i| {
            let y = (i / w) as f64 / h as f64;
            let c = (246. - 22. * y) as u8;
            Color32::from_rgb(c, c, c.saturating_add(4))
        })
        .collect();
    let mut depth = vec![f64::MIN; w * h];
    for (t, color) in &triangles {
        let n = normalize(cross(sub(t[1], t[0]), sub(t[2], t[0])));
        let shade = 0.35 + 0.65 * dot(n, light).abs();
        let rgb = [0, 1, 2].map(|i| (f64::from(color[i]) * shade).clamp(0., 255.) as u8);
        let s = t.map(screen);
        let area =
            (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        let x0 = s
            .iter()
            .map(|p| p[0])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let x1 = (s.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil() as usize).min(w - 1);
        let y0 = s
            .iter()
            .map(|p| p[1])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.) as usize;
        let y1 = (s.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil() as usize).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let edge = |a: [f64; 3], b: [f64; 3]| {
                    ((b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])) / area
                };
                let (l0, l1, l2) = (edge(s[1], s[2]), edge(s[2], s[0]), edge(s[0], s[1]));
                if l0 < 0. || l1 < 0. || l2 < 0. {
                    continue;
                }
                let z = l0 * s[0][2] + l1 * s[1][2] + l2 * s[2][2];
                let i = y * w + x;
                if z > depth[i] {
                    depth[i] = z;
                    pixels[i] = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                }
            }
        }
    }
    Ok(ColorImage::new(PREVIEW, pixels))
}

/// Set the file name, description and rendered preview as one undoable
/// transaction through the shared file-information edits.
pub fn finish(s: &mut Scenario) -> Result<(), Error> {
    let image = render_preview(s)?;
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
    use multicad_ui::{application::DocumentStorage, storage::SaveMode};
    let path = std::env::args_os().nth(1).ok_or(
        "Usage: cargo run -p multicad-app --example apartment_demo -- NEW_FILE.tcd (for example examples/apartment.tcd)",
    )?;
    let path = PathBuf::from(path);
    if !multicad_core::project_path::is_current_project_path(&path) {
        return Err("The output must be a .tcd file".into());
    }
    if path.exists() {
        return Err(format!("{} exists; delete it to regenerate", path.display()).into());
    }
    let mut scenario = build(&path)?;
    finish(&mut scenario)?;
    let session = &scenario.session;
    let saved = files::ProjectStore::default()
        .save_project(
            &path,
            &ProjectFile::new(session.document().clone())?,
            SaveMode::Create,
        )
        .map_err(|e| e.to_string())?;
    let checks = project_references::check_references(session.document(), Some(&path), &[])?;
    let info = project_file::info::read(session.document())?;
    let preview = info.preview.as_ref().ok_or("No preview")?;
    let manifest = serde_json::json!({
        "generator": "apartment_demo",
        "file": path,
        "name": info.name,
        "preview_px": [preview.width(), preview.height()],
        "warnings": saved.warnings,
        "references": checks.iter().map(|c| c.summary()).collect::<Vec<_>>(),
        "cabinet": CABINET_SOURCE,
        "verification": {
            "native_geometry": true,
            "interactive_macos": false,
            "human_acceptance": false,
            "building_code_or_egress": false,
        },
    });
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
