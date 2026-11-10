//! Editable printed drill guide with metal bushings and an adjustable edge fence.
#[path = "workbench_demo.rs"]
#[allow(dead_code)]
mod workbench;
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    drawings::*,
    exchange::*,
    materials::{Assignment, TagEdit, TagQuery, prepare_assignment, prepare_library, prepare_tags},
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, length,
    },
    parameters::BoundExpression,
    project_file::{self, info::FileInfo},
    references::ReferenceFrame,
};
use multicad_model::Compiler;
use workbench::{Error, actor, checked};

pub const NAME: &str = "Adjustable drill guide";
pub const SHIPPED_PATH: &str = "examples/drill-guide.tcd";
pub struct Scenario {
    pub session: Session,
    pub body: EntityId,
    pub fence: EntityId,
    pub printed: Vec<EntityId>,
    pub bushings: Vec<EntityId>,
    pub parts: Vec<EntityId>,
    pub book: EntityId,
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
fn dim(s: &Session, value: &str) -> Result<Dimension, Error> {
    if let Some(n) = value.strip_prefix('#') {
        return Ok(length(n.parse()?));
    }
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(s.document(), value, "mm", None)?,
    })
}
fn point(s: &Session, values: [&str; 3]) -> Result<[Dimension; 3], Error> {
    Ok([dim(s, values[0])?, dim(s, values[1])?, dim(s, values[2])?])
}
fn primitive(
    s: &mut Session,
    name: &str,
    parameters: PrimitiveParameters,
) -> Result<BodyInput, Error> {
    let r = PrimitiveRequest::new(name, parameters);
    let p = model::prepare_primitive(s, s.revision(), actor(), &r)?;
    checked(s, p.prepared)?;
    Ok(BodyInput {
        body: r.body,
        producer: r.feature,
    })
}
fn block(
    s: &mut Session,
    name: &str,
    size: [&str; 3],
    origin: [&str; 3],
) -> Result<BodyInput, Error> {
    let p = PrimitiveParameters::Box {
        size: point(s, size)?,
        frame: PrimitiveFrame {
            origin: point(s, origin)?,
            ..Default::default()
        },
    };
    primitive(s, name, p)
}
fn cylinder(
    s: &mut Session,
    name: &str,
    radius: f64,
    height: &str,
    origin: [&str; 3],
) -> Result<BodyInput, Error> {
    let p = PrimitiveParameters::Cylinder {
        radius: length(radius),
        height: dim(s, height)?,
        frame: PrimitiveFrame {
            origin: point(s, origin)?,
            ..Default::default()
        },
    };
    primitive(s, name, p)
}
fn boolean(
    s: &mut Session,
    name: &str,
    operation: Operation,
    target: BodyInput,
    tools: Vec<BodyInput>,
) -> Result<BodyInput, Error> {
    let body = target.body;
    let mut r = BooleanRequest::new(operation, target, tools);
    r.name = name.into();
    let p = model::prepare_boolean(s, s.revision(), actor(), &r)?;
    checked(s, p.prepared)?;
    Ok(BodyInput {
        body,
        producer: r.feature,
    })
}
fn material(name: &str, color: [u8; 4], density: f64) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(density),
            color_rgba: color,
            section_hatch: None,
        },
    )
}
fn assign(s: &mut Session, parts: &[EntityId], material: EntityId) -> Result<(), Error> {
    let p = prepare_assignment(
        s,
        s.revision(),
        actor(),
        parts,
        &Assignment {
            material: Some(material),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )?;
    s.commit(p)?;
    Ok(())
}
fn tag(s: &mut Session, parts: &[EntityId], value: &str) -> Result<(), Error> {
    let p = prepare_tags(
        s,
        s.revision(),
        actor(),
        parts,
        &TagEdit::Add([value.into()].into()),
    )?;
    s.commit(p)?;
    Ok(())
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
fn view(name: &str, parts: Vec<EntityId>, frame: ReferenceFrame, at: [f64; 2]) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model { entities: parts },
        frame,
        kind: ViewKind::Projection,
        position_mm: at,
        scale: Scale {
            numerator: 1,
            denominator: 1,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.05,
        guide: None,
        equipment_categories: None,
    }
}
fn dimension(
    view: EntityId,
    a: multicad_core::inspection::GeometryReference,
    b: multicad_core::inspection::GeometryReference,
    direction: LinearDirection,
    offset: f64,
) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Dimension {
            view,
            measure: Box::new(DimensionMeasure::Linear { a, b, direction }),
            offset_mm: offset,
            precision: 1,
            reference: false,
        },
    }
}
pub fn build() -> Result<Scenario, Error> {
    let parameters = [
        parameter("guide_width", "120", "Body width and fence length"),
        parameter("guide_depth", "90", "Printed body depth"),
        parameter("body_thickness", "18", "Printed body thickness"),
        parameter(
            "guide_pitch",
            "30",
            "Center distance between drill bushings",
        ),
        parameter(
            "edge_distance",
            "30",
            "Drill axes to the fence contact face; adjustment range 5–38 mm",
        ),
        parameter(
            "bushing_fit",
            "0.2",
            "Extra diameter for the metal bushing seats; calibrate to the printer",
        ),
    ];
    let rows: Vec<_> = parameters.iter().map(|p| p.id).collect();
    let mut edits: Vec<_> = parameters.into_iter().map(Mutation::Insert).collect();
    edits.push(Mutation::Insert(Entity::new(
        "Parameter table",
        EntityData::Extension {
            definition: multicad_core::document::Definition {
                type_id: "core.parameter_table".into(),
                schema_version: 1,
                data: serde_json::to_value(&rows)?,
            },
            inputs: rows.iter().map(|id| Link::new(*id, "row")).collect(),
        },
    )));
    let mut session = Session::new(Document::new(NAME)?.edited(&edits)?)?;
    let mats = [
        material("PETG guide body", [33, 123, 145, 255], 1270.),
        material("PETG fence", [38, 58, 76, 255], 1270.),
        material("PETG thumb knobs", [225, 130, 42, 255], 1270.),
        material("Steel hardware", [177, 183, 190, 255], 7850.),
    ];
    let colors: Vec<_> = mats.iter().map(|m| m.id).collect();
    let p = prepare_library(&session, session.revision(), actor(), &mats)?;
    session.commit(p)?;
    let body = block(
        &mut session,
        "Printed drill-guide body",
        ["=$guide_width", "=$guide_depth", "=$body_thickness"],
        ["#0", "#0", "#0"],
    )?;
    let mut tools = vec![];
    let xs = [
        "=$guide_width / 2 - $guide_pitch",
        "=$guide_width / 2",
        "=$guide_width / 2 + $guide_pitch",
    ];
    for x in xs {
        let parameters = PrimitiveParameters::Cylinder {
            radius: dim(&session, "=6 mm + $bushing_fit / 2")?,
            height: dim(&session, "=$body_thickness + 2 mm")?,
            frame: PrimitiveFrame {
                origin: point(&session, [x, "=$guide_depth - 25 mm", "#-1"])?,
                ..Default::default()
            },
        };
        tools.push(primitive(&mut session, "Bushing seat cutter", parameters)?);
    }
    // Real round-ended adjustment slots, not a drawn decal.
    for x in ["#12", "=$guide_width - 12 mm"] {
        let left = if x == "#12" {
            "#8.5"
        } else {
            "=$guide_width - 15.5 mm"
        };
        tools.push(block(
            &mut session,
            "Fence slot middle",
            ["#7", "=$guide_depth - 50 mm", "=$body_thickness + 2 mm"],
            [left, "#20", "#-1"],
        )?);
        for y in ["#20", "=$guide_depth - 30 mm"] {
            tools.push(cylinder(
                &mut session,
                "Fence slot round end",
                3.5,
                "=$body_thickness + 2 mm",
                [x, y, "#-1"],
            )?);
        }
    }
    let body = boolean(
        &mut session,
        "Bushing seats and adjustment slots",
        Operation::Cut,
        body,
        tools,
    )?
    .body;
    let fence = block(
        &mut session,
        "Printed sliding fence",
        ["=$guide_width", "#14", "#22"],
        ["#0", "=$guide_depth - 39 mm - $edge_distance", "#-22"],
    )?;
    let mut holes = vec![];
    for x in ["#12", "=$guide_width - 12 mm"] {
        holes.push(cylinder(
            &mut session,
            "Fence screw clearance",
            3.3,
            "#24",
            [x, "=$guide_depth - 32 mm - $edge_distance", "#-23"],
        )?);
        holes.push(cylinder(
            &mut session,
            "Fence insert pocket",
            4.1,
            "#6",
            [x, "=$guide_depth - 32 mm - $edge_distance", "#-22.1"],
        )?);
    }
    let fence = boolean(
        &mut session,
        "Fence screw and insert pockets",
        Operation::Cut,
        fence,
        holes,
    )?
    .body;
    let mut hardware = vec![];
    let mut bushings = vec![];
    let mut knobs = vec![];
    for (index, (x, hole)) in xs.into_iter().zip([3.05, 5.05, 8.05]).enumerate() {
        let outer = cylinder(
            &mut session,
            &format!("Steel drill bushing {} mm", [3, 5, 8][index]),
            6.,
            "#22",
            [x, "=$guide_depth - 25 mm", "#0"],
        )?;
        let inner = cylinder(
            &mut session,
            "Bushing through bore",
            hole / 2.,
            "#24",
            [x, "=$guide_depth - 25 mm", "#-1"],
        )?;
        let b = boolean(
            &mut session,
            "Drill guide bore",
            Operation::Cut,
            outer,
            vec![inner],
        )?
        .body;
        hardware.push(b);
        bushings.push(b);
    }
    for (index, x) in ["#12", "=$guide_width - 12 mm"].into_iter().enumerate() {
        let side = if index == 0 { "Left" } else { "Right" };
        let y = "=$guide_depth - 32 mm - $edge_distance";
        let washer = cylinder(
            &mut session,
            &format!("{side} M6 washer"),
            8.,
            "#2",
            [x, y, "=$body_thickness"],
        )?;
        let bore = cylinder(
            &mut session,
            "Washer clearance",
            3.3,
            "#4",
            [x, y, "=$body_thickness - 1 mm"],
        )?;
        hardware.push(
            boolean(
                &mut session,
                "Washer bore",
                Operation::Cut,
                washer,
                vec![bore],
            )?
            .body,
        );
        let insert = cylinder(
            &mut session,
            &format!("{side} M6 insert (simplified)"),
            4.,
            "#5.8",
            [x, y, "#-22"],
        )?;
        let bore = cylinder(
            &mut session,
            "Insert bore envelope",
            3.05,
            "#8",
            [x, y, "#-23"],
        )?;
        hardware.push(
            boolean(
                &mut session,
                "Insert bore",
                Operation::Cut,
                insert,
                vec![bore],
            )?
            .body,
        );
        let shaft = cylinder(
            &mut session,
            &format!("{side} M6 screw (simplified)"),
            3.,
            "=$body_thickness + 32 mm",
            [x, y, "#-21"],
        )?;
        let hx = if index == 0 {
            "#8"
        } else {
            "=$guide_width - 16 mm"
        };
        let head = block(
            &mut session,
            "Captured square screw head",
            ["#8", "#8", "#3"],
            [
                hx,
                "=$guide_depth - 36 mm - $edge_distance",
                "=$body_thickness + 8 mm",
            ],
        )?;
        hardware.push(
            boolean(
                &mut session,
                "Screw and captured head",
                Operation::Join,
                shaft,
                vec![head],
            )?
            .body,
        );
        let knob = cylinder(
            &mut session,
            &format!("Printed thumb knob {}", index + 1),
            10.,
            "#9",
            [x, y, "=$body_thickness + 2 mm"],
        )?;
        let mut ribs = vec![];
        for n in 0..8 {
            let angle = n as f64 * std::f64::consts::TAU / 8.;
            let cx = if index == 0 {
                format!("={} mm", 12. + 9. * angle.cos())
            } else {
                format!("=$guide_width - {} mm", 12. - 9. * angle.cos())
            };
            let cy = format!(
                "=$guide_depth - {} mm - $edge_distance",
                32. - 9. * angle.sin()
            );
            ribs.push(cylinder(
                &mut session,
                "Thumb grip lobe",
                2.,
                "#9",
                [&cx, &cy, "=$body_thickness + 2 mm"],
            )?);
        }
        let knob = boolean(
            &mut session,
            "Thumb-grip lobes",
            Operation::Join,
            knob,
            ribs,
        )?;
        let bore = cylinder(
            &mut session,
            "Knob shaft clearance",
            3.3,
            "#12",
            [x, y, "=$body_thickness + 1 mm"],
        )?;
        let px = if index == 0 {
            "#7.8"
        } else {
            "=$guide_width - 16.2 mm"
        };
        let pocket = block(
            &mut session,
            "Knob head pocket",
            ["#8.4", "#8.4", "#4"],
            [
                px,
                "=$guide_depth - 36.2 mm - $edge_distance",
                "=$body_thickness + 7.8 mm",
            ],
        )?;
        knobs.push(
            boolean(
                &mut session,
                "Knob bore and captured-head pocket",
                Operation::Cut,
                knob,
                vec![bore, pocket],
            )?
            .body,
        );
    }
    assign(&mut session, &[body], colors[0])?;
    assign(&mut session, &[fence], colors[1])?;
    assign(&mut session, &knobs, colors[2])?;
    assign(&mut session, &hardware, colors[3])?;
    let mut printed = vec![body, fence];
    printed.extend(&knobs);
    tag(&mut session, &printed, "printed")?;
    tag(&mut session, &hardware, "purchased")?;
    let mut parts = printed.clone();
    parts.extend(&hardware);
    let mut page = Page::new("DG-101");
    page.settings.paper = PaperSize::A3;
    page.settings.scale = Scale {
        numerator: 1,
        denominator: 1,
    };
    page.info = DrawingInfo {
        title: NAME.into(),
        author: "ToskCAD".into(),
        project: NAME.into(),
        revision: "A".into(),
        checker: "Example geometry".into(),
    };
    let top = view(
        "Top",
        parts.clone(),
        ReferenceFrame {
            origin_mm: [60., 45., 0.],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        [100., 100.],
    );
    let front = view(
        "Front",
        parts.clone(),
        ReferenceFrame {
            origin_mm: [60., 0., 0.],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        [285., 100.],
    );
    let model = Compiler::default().compile(session.document(), session.revision())?;
    page.annotations = vec![
        dimension(
            top.id,
            workbench::face(&model, body, [-1., 0., 0.], 0.)?,
            workbench::face(&model, body, [1., 0., 0.], 120.)?,
            LinearDirection::Horizontal,
            12.,
        ),
        dimension(
            top.id,
            workbench::face(&model, body, [0., -1., 0.], 0.)?,
            workbench::face(&model, body, [0., 1., 0.], 90.)?,
            LinearDirection::Vertical,
            -12.,
        ),
        note(
            [35., 190.],
            "Steel guide bushings: 3, 5 and 8 mm drills; bore clearance 0.05 mm.",
        ),
        note(
            [35., 200.],
            "Change edge_distance to slide the fence and both clamps together.",
        ),
        note(
            [35., 210.],
            "Printed parts: body, fence and two knobs. Metal hardware is excluded from the print set.",
        ),
        note(
            [35., 220.],
            "M6 screw and insert envelopes are simplified; match real hardware before making the jig.",
        ),
    ];
    page.views = vec![top, front];
    let (book, p) = prepare_create(&session, session.revision(), actor(), NAME, Some(page))?;
    checked(&mut session, p)?;
    let definition = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Project,
            tags: Some(TagQuery {
                all: ["printed".into()].into(),
                ..Default::default()
            }),
        },
        recipes: vec![
            Recipe::step("STEP"),
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
            },
        ],
        ..Default::default()
    };
    let p = prepare_set(
        &session,
        session.revision(),
        actor(),
        EntityId::new(),
        "Printed drill-guide parts",
        &definition,
    )?;
    session.commit(p)?;
    let image = workbench::render_preview(&session, &parts)?;
    let info=FileInfo {name:NAME.into(),description:"An adjustable drill guide with a printed PETG body, edge fence and thumb knobs, plus three steel bushings for 3, 5 and 8 mm drills. Change edge_distance to move the fence and clamps together, or edit the bushing pitch and seat allowance. Common creates the parts; Parameters controls their positions; the print export set includes the four printed parts and excludes purchased hardware. Screw threads and inserts are shown as simplified envelopes.".into(),preview:Some(multicad_ui::preview_capture::preview_from(&image)?),capture_on_save:true};
    let edits = project_file::info::edits(session.document(), &info)?;
    let p = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Set drill guide information".into(),
        edits,
    })?;
    checked(&mut session, p)?;
    Ok(Scenario {
        session,
        body,
        fence,
        printed,
        bushings,
        parts,
        book,
    })
}
#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Usage: drill_guide_demo NEW_FILE.tcd")?;
    let path = std::path::PathBuf::from(path);
    if path.exists() {
        return Err("Output already exists".into());
    }
    let scenario = build()?;
    let report = workbench::page_report(&scenario.session, scenario.book)?;
    if let Some((page, conflicts)) = report.iter().find(|(_, conflicts)| !conflicts.is_empty()) {
        return Err(format!("{page} drawing conflicts: {conflicts:?}").into());
    }
    let mut bytes = vec![];
    project_file::write(
        &project_file::ProjectFile::new(scenario.session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    std::fs::write(&path, bytes)?;
    println!(
        "Saved {} native parts to {}",
        scenario.parts.len(),
        path.display()
    );
    Ok(())
}
