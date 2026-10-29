//! The makerspace vertical slice in one native editable project: a room of
//! reusable wall, door and window types driven by `room_width` and
//! `room_depth`, a ground level with a floor slab, a mezzanine reached by a
//! stair, three placements of one dynamic equipment symbol with service
//! envelopes, a floor plan and a section generated from drawing guides with
//! associative dimensions, and a tag-selected export set with PDF, drawing
//! DXF and STEP recipes. The app test `tests/makerspace.rs` builds the same
//! project and checks it.
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{
        ConstraintKind, Dimension,
        binding::{self, DimensionInput},
        source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    drawings::*,
    exchange::*,
    inspection::GeometryReference,
    materials::*,
    model::{
        self, BodyOutput, EnvelopeDefinition, Extent, ExtrudeParameters, ExtrudeRequest, Hinge,
        Justification, LevelDefinition, Opening, OpeningKind, OpeningTypeDefinition,
        SlabDefinition, SlabKind, StairDefinition, StairForm, Swing, Termination, WallDefinition,
        WallRequest, WallTypeDefinition, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, TopologyKind},
    sketch::Datum,
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest, SymbolPlan, SymbolValue},
};
use multicad_model::{Compiler, Model, export::Exporter};
use std::{collections::BTreeMap, io::Write, path::Path};

pub type Error = Box<dyn std::error::Error>;
pub const EQUIPMENT_TAG: &str = "equipment";

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
fn bound(session: &Session, text: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(session.document(), text, "mm", None)?,
    })
}

/// Exposed parameter IDs of the equipment symbol definition.
#[derive(Clone, Copy, Debug)]
pub struct EquipmentParameters {
    pub width: EntityId,
    pub depth: EntityId,
    pub height: EntityId,
}
/// A dynamic equipment footprint: a constrained `width` × `depth` rectangle
/// on XY, extruded by `height`. Its local origin is the back-left corner of
/// the footprint at floor level; the plan sketch is the drawing symbol.
fn equipment_library() -> Result<(Session, EntityId, EntityId, EquipmentParameters), Error> {
    let width = parameter("width", "1000", "Equipment width along local X");
    let depth = parameter("depth", "800", "Equipment depth along local Y");
    let height = parameter("height", "900", "Equipment height");
    let ids = EquipmentParameters {
        width: width.id,
        depth: depth.id,
        height: height.id,
    };
    let (sketch, mut edits) = Source::create(Datum::Xy, "Equipment footprint")?;
    edits.extend([width, depth, height].into_iter().map(Mutation::Insert));
    let mut session = Session::new(Document::new("Equipment library")?.edited(&edits)?)?;
    let mut source = Source::read(session.document(), sketch)?.0;
    let bottom = source.add_rectangle([0., 0.], [1000., 800.])?;
    source.add_constraint(ConstraintKind::Fix {
        point: source.sketch.points[0].id,
    });
    let along = source.add_constraint(ConstraintKind::Length {
        line: bottom,
        dimension: length(1000.),
    });
    let across = source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: length(800.),
    });
    for (id, name) in [(along, "width"), (across, "depth")] {
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
    let edit =
        multicad_solver::prepare_edit(&session, session.revision(), actor(), sketch, source)?;
    session.commit(edit.preview.ok_or("Equipment footprint did not solve")?)?;
    let evaluated = multicad_solver::evaluate_document_sketch(
        session.document(),
        session.revision(),
        sketch,
        None,
    )?;
    let regions = evaluated.regions()?;
    if regions.regions.len() != 1 {
        return Err("Expected one equipment footprint region".into());
    }
    let mut extrusion = ExtrudeRequest::new(
        sketch,
        regions.regions.iter().map(|r| r.intent()).collect(),
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: bound(&session, "=$height")?,
                },
            },
            ..Default::default()
        },
    );
    extrusion.outputs[0].name = "Equipment body".into();
    let prepared = model::prepare_extrude(&session, session.revision(), actor(), &extrusion)?;
    let body = prepared.outputs[0].body;
    checked(&mut session, prepared.prepared)?;
    // The tag is part of the definition, so every placement carries it.
    let prepared = prepare_tags(
        &session,
        session.revision(),
        actor(),
        &[body],
        &TagEdit::Add([EQUIPMENT_TAG.into()].into()),
    )?;
    session.commit(prepared)?;
    let author = SymbolAuthorRequest::new(
        "Equipment footprint",
        body,
        SymbolKind::Dynamic,
        vec![ids.width, ids.depth, ids.height],
        Some(SymbolPlan { sketch }),
    );
    let prepared = symbols::prepare_author(&session, session.revision(), actor(), &author)?;
    checked(&mut session, prepared.prepared)?;
    Ok((session, author.definition, body, ids))
}

/// One placed machine.
pub struct Equipment {
    pub label: &'static str,
    pub place: SymbolPlaceRequest,
    pub envelope: EntityId,
}
pub struct Scenario {
    pub session: Session,
    pub room_width: EntityId,
    pub room_depth: EntityId,
    pub concrete: EntityId,
    pub ground: EntityId,
    pub mezzanine: EntityId,
    pub wall_type: EntityId,
    pub door: EntityId,
    pub window: EntityId,
    pub walls: EntityId,
    pub floor: EntityId,
    pub deck: EntityId,
    pub stair: EntityId,
    pub stair_feature: EntityId,
    pub definition: EntityId,
    pub parameters: EquipmentParameters,
    pub equipment: Vec<Equipment>,
    pub plan_guide: EntityId,
    pub section_guide: EntityId,
    pub book: EntityId,
    pub plan_page: EntityId,
    pub room_dimension: EntityId,
    pub bench_dimension: EntityId,
    pub set: EntityId,
}
impl Scenario {
    pub fn equipment(&self, label: &str) -> &Equipment {
        self.equipment
            .iter()
            .find(|e| e.label == label)
            .expect("known equipment")
    }
    pub fn instances(&self) -> Vec<EntityId> {
        self.equipment.iter().map(|e| e.place.instance).collect()
    }
}

/// A planar face of a placed Part or Instance whose normal matches `normal`
/// and whose plane passes through coordinate `at` along that normal.
pub fn face(
    model: &Model,
    entity: EntityId,
    normal: [f64; 3],
    at: f64,
) -> Result<GeometryReference, Error> {
    let mut topology = if let Some(occurrence) = model.snapshot().occurrences.get(&entity) {
        occurrence.topology.clone()
    } else {
        let feature = model.snapshot().bodies[&entity];
        model.snapshot().shapes[&feature].topology.clone()
    };
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
    offset: f64,
) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Dimension {
            view,
            measure: Box::new(DimensionMeasure::Linear {
                a,
                b,
                direction: LinearDirection::Horizontal,
            }),
            offset_mm: offset,
            precision: 0,
            reference: false,
        },
    }
}
/// A plan point; "0" is a literal zero, anything else an expression.
fn corner(session: &Session, x: &str, y: &str) -> Result<[Dimension; 2], Error> {
    let axis = |text: &str| {
        if text == "0" {
            Ok(length(0.))
        } else {
            bound(session, text)
        }
    };
    Ok([axis(x)?, axis(y)?])
}
fn level(
    session: &mut Session,
    name: &str,
    elevation: f64,
    height: f64,
) -> Result<EntityId, Error> {
    let (id, prepared) = model::prepare_level(
        session,
        session.revision(),
        actor(),
        name,
        &LevelDefinition {
            elevation: length(elevation),
            height: length(height),
            slab_thickness: length(200.),
        },
    )?;
    session.commit(prepared)?;
    Ok(id)
}
/// The book page that holds a view generated by `guide`.
pub fn guided_page(document: &Document, book: EntityId, guide: EntityId) -> Result<Page, Error> {
    multicad_core::drawings::read(document, book)?
        .pages
        .into_iter()
        .find(|p| {
            p.views
                .iter()
                .any(|v| v.guide.as_ref().is_some_and(|g| g.guide == guide))
        })
        .ok_or_else(|| "Guide has no generated page".into())
}

pub fn build() -> Result<Scenario, Error> {
    let (library, definition_id, _, parameters) = equipment_library()?;
    let room_width = parameter("room_width", "9000", "Wall centerline span along X");
    let room_depth = parameter("room_depth", "6000", "Wall centerline span along Y");
    let mut session = Session::new(Document::new("Makerspace")?.edited(&[
        Mutation::Insert(room_width.clone()),
        Mutation::Insert(room_depth.clone()),
    ])?)?;

    // Concrete with a section hatch for walls, slabs and the stair.
    let concrete = Entity::new(
        "Concrete",
        EntityData::Material {
            density_kg_m3: Some(2400.),
            color_rgba: [190, 188, 182, 255],
            section_hatch: None,
        },
    );
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        pattern: HatchPattern::Lines,
        angle_deg: 45.,
        spacing_mm: 2.,
        line_width_mm: 0.18,
    })
    .entity(EntityId::new(), "Concrete section")?;
    let prepared = prepare_library(
        &session,
        session.revision(),
        actor(),
        &[concrete.clone(), hatch.clone()],
    )?;
    session.commit(prepared)?;
    let style = LibraryDefinition::MaterialStyle(MaterialStyle {
        material: concrete.id,
        appearance: None,
        hatch: Some(LibraryRef::capture(session.document(), hatch.id)?),
    })
    .entity(EntityId::new(), "Defaults")?;
    let prepared = prepare_library(&session, session.revision(), actor(), &[style])?;
    session.commit(prepared)?;

    // Levels: the ground floor and a mezzanine 2600 mm above it.
    let ground = level(&mut session, "Ground floor", 0., 2600.)?;
    let mezzanine = level(&mut session, "Mezzanine", 2600., 2400.)?;

    // Reusable wall, door and window types.
    let (wall_type, prepared) = model::prepare_wall_type(
        &session,
        session.revision(),
        actor(),
        "Block 200",
        &WallTypeDefinition {
            thickness: length(200.),
            height: length(3600.),
            justification: Justification::Center,
            material: Some(concrete.id),
        },
    )?;
    session.commit(prepared)?;
    let (door, prepared) = model::prepare_opening_type(
        &session,
        session.revision(),
        actor(),
        "Door 1000",
        &OpeningTypeDefinition {
            kind: OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Left,
            },
            width: length(1000.),
            height: length(2200.),
            sill: length(0.),
        },
    )?;
    session.commit(prepared)?;
    let (window, prepared) = model::prepare_opening_type(
        &session,
        session.revision(),
        actor(),
        "Window 1500",
        &OpeningTypeDefinition {
            kind: OpeningKind::Window,
            width: length(1500.),
            height: length(1200.),
            sill: length(1000.),
        },
    )?;
    session.commit(prepared)?;

    // The room: one closed wall loop on the parameter-driven centerline.
    let opening = |opening_type: EntityId, segment: usize, offset: f64| Opening {
        id: EntityId::new(),
        opening_type,
        segment,
        offset: length(offset),
        width: None,
        height: None,
        sill: None,
    };
    let request = WallRequest::new(
        "Makerspace walls",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type,
            path: vec![
                corner(&session, "0", "0")?,
                corner(&session, "=$room_width", "0")?,
                corner(&session, "=$room_width", "=$room_depth")?,
                corner(&session, "0", "=$room_depth")?,
            ],
            closed: true,
            level: Some(ground),
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![
                opening(door, 0, 4000.),
                opening(window, 1, 1000.),
                opening(window, 2, 3000.),
            ],
        },
    );
    let walls = request.wall.output.body;
    let prepared = model::prepare_wall(&session, session.revision(), actor(), &request)?;
    checked(&mut session, prepared.prepared)?;

    // Floor slab under the whole room and a mezzanine deck in the back-left
    // corner, both following the room parameters.
    let floor = SlabDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        level: ground,
        kind: SlabKind::Slab,
        outline: vec![
            corner(&session, "=-100 mm", "=-100 mm")?,
            corner(&session, "=$room_width + 100 mm", "=-100 mm")?,
            corner(&session, "=$room_width + 100 mm", "=$room_depth + 100 mm")?,
            corner(&session, "=-100 mm", "=$room_depth + 100 mm")?,
        ],
        thickness: None,
    };
    let prepared = model::prepare_slab(
        &session,
        session.revision(),
        actor(),
        "Ground slab",
        &floor,
        Some(concrete.id),
    )?;
    checked(&mut session, prepared.prepared)?;
    let deck = SlabDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        level: mezzanine,
        kind: SlabKind::Slab,
        outline: vec![
            corner(&session, "=100 mm", "=$room_depth - 2600 mm")?,
            corner(&session, "=3100 mm", "=$room_depth - 2600 mm")?,
            corner(&session, "=3100 mm", "=$room_depth - 100 mm")?,
            corner(&session, "=100 mm", "=$room_depth - 100 mm")?,
        ],
        thickness: None,
    };
    let prepared = model::prepare_slab(
        &session,
        session.revision(),
        actor(),
        "Mezzanine deck",
        &deck,
        Some(concrete.id),
    )?;
    checked(&mut session, prepared.prepared)?;

    // A straight stair walking toward −X onto the deck edge.
    let stair = StairDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        level: ground,
        to_level: Some(mezzanine),
        form: StairForm::Straight,
        start: corner(&session, "=6740 mm", "=$room_depth - 2150 mm")?,
        direction_deg: 180.,
        width: length(900.),
        risers: 14,
        going: length(260.),
    };
    let prepared = model::prepare_stair(
        &session,
        session.revision(),
        actor(),
        "Mezzanine stair",
        &stair,
        Some(concrete.id),
    )?;
    let stair_feature = prepared.outputs[0].feature;
    checked(&mut session, prepared.prepared)?;

    // Three placements of one dynamic equipment symbol, each with its own
    // dimensions, a service envelope and the `equipment` tag.
    let source = symbols::read_definition(library.document(), definition_id)?;
    let imported = symbols::prepare_import(&session, session.revision(), actor(), &source)?;
    let definition = imported.definition;
    checked(&mut session, imported.prepared)?;
    let size = |w: f64, d: f64, h: f64| {
        BTreeMap::from([
            (
                parameters.width,
                SymbolValue {
                    value: w,
                    unit: "mm".into(),
                },
            ),
            (
                parameters.depth,
                SymbolValue {
                    value: d,
                    unit: "mm".into(),
                },
            ),
            (
                parameters.height,
                SymbolValue {
                    value: h,
                    unit: "mm".into(),
                },
            ),
        ])
    };
    // (label, size, origin, clearances front/back/left/right/top)
    let machines = [
        (
            "Laser cutter",
            size(1400., 900., 1000.),
            [600., 1200.],
            [900., 300., 300., 300., 300.],
        ),
        (
            "CNC router",
            size(2500., 1500., 1100.),
            [6000., 1500.],
            [1000., 300., 300., 400., 500.],
        ),
        (
            "Workbench",
            size(1800., 800., 900.),
            [3000., 1200.],
            [900., 600., 0., 0., 0.],
        ),
    ];
    let mut equipment = vec![];
    for (label, values, [x, y], [front, back, left, right, top]) in machines {
        let place = SymbolPlaceRequest::new(
            session.document(),
            definition,
            label,
            values,
            Placement {
                translation_mm: [x, y, 0.],
                ..Default::default()
            },
        )?;
        let prepared = symbols::prepare_place(&session, session.revision(), actor(), &place)?;
        checked(&mut session, prepared.prepared)?;
        let (envelope, prepared) = model::prepare_envelope(
            &session,
            session.revision(),
            actor(),
            &format!("{label} clearance"),
            &EnvelopeDefinition {
                target: place.instance,
                front: length(front),
                back: length(back),
                left: length(left),
                right: length(right),
                top: length(top),
                label: format!("{label} service area"),
            },
        )?;
        session.commit(prepared)?;
        equipment.push(Equipment {
            label,
            place,
            envelope,
        });
    }
    let instances: Vec<_> = equipment.iter().map(|e| e.place.instance).collect();

    // Drawing guides generate a linked floor plan and a section.
    let mut sources = vec![
        walls,
        floor.output.body,
        deck.output.body,
        stair.output.body,
    ];
    sources.extend(&instances);
    let plan = GuideDefinition {
        kind: GuideKind::Plan {
            cut_height: length(1200.),
            view_depth: length(1400.),
        },
        levels: vec![ground],
        sources: sources.clone(),
        scale: Scale {
            numerator: 1,
            denominator: 50,
        },
    };
    let generated = prepare_guide(
        &session,
        session.revision(),
        actor(),
        None,
        "Floor plan",
        &plan,
        true,
        None,
    )?;
    let plan_guide = generated.guide;
    let book = generated
        .generation
        .as_ref()
        .ok_or("Plan guide generated no views")?
        .book;
    checked(&mut session, generated.prepared)?;
    let section = GuideDefinition {
        kind: GuideKind::Section {
            start: [length(-500.), bound(&session, "=$room_depth - 2150 mm")?],
            end: [
                bound(&session, "=$room_width + 500 mm")?,
                bound(&session, "=$room_depth - 2150 mm")?,
            ],
            depth: None,
        },
        levels: vec![ground, mezzanine],
        sources,
        scale: Scale {
            numerator: 1,
            denominator: 50,
        },
    };
    let generated = prepare_guide(
        &session,
        session.revision(),
        actor(),
        None,
        "Section A-A",
        &section,
        true,
        Some(book),
    )?;
    let section_guide = generated.guide;
    checked(&mut session, generated.prepared)?;

    // The plan page: title block, the room's inner width and the workbench
    // width dimensioned on the plan view, a note and an equipment table.
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let mut page = guided_page(session.document(), book, plan_guide)?;
    // Room spans 180 × 120 mm on paper at 1:50, left of the table.
    page.views[0].position_mm = [115., 205.];
    let view = page.views[0].id;
    page.number = "A-101".into();
    page.info = DrawingInfo {
        title: "Makerspace floor plan".into(),
        author: "MultiCAD".into(),
        project: "Makerspace".into(),
        revision: "A".into(),
        checker: "Pending human review".into(),
    };
    let room_dimension = dimension(
        view,
        face(&model, walls, [1., 0., 0.], 100.)?,
        face(&model, walls, [-1., 0., 0.], -(9000. - 100.))?,
        20.,
    );
    let bench = equipment[2].place.instance;
    let bench_dimension = dimension(
        view,
        face(&model, bench, [-1., 0., 0.], -3000.)?,
        face(&model, bench, [1., 0., 0.], 4800.)?,
        8.,
    );
    let ids = (room_dimension.id, bench_dimension.id);
    page.annotations.push(room_dimension);
    page.annotations.push(bench_dimension);
    page.annotations.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: [30., 270.],
            text: "Dashed service areas are clearances, not certified egress".into(),
        },
    });
    page.annotations.push(Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Table {
            position_mm: [315., 40.],
            title: "Equipment".into(),
            columns: vec!["Machine".into(), "W × D × H".into()],
            rows: vec![
                vec!["Laser cutter".into(), "1400 × 900 × 1000".into()],
                vec!["CNC router".into(), "2500 × 1500 × 1100".into()],
                vec!["Workbench".into(), "1800 × 800 × 900".into()],
            ],
            widths_mm: vec![30., 34.],
            row_height_mm: 5.,
        },
    });
    let info = page.info.clone();
    let plan_page = page.id;
    let prepared = prepare(
        &session,
        session.revision(),
        actor(),
        book,
        PageAction::Replace(page),
    )?;
    checked(&mut session, prepared)?;
    let mut page = guided_page(session.document(), book, section_guide)?;
    page.views[0].position_mm = [118., 190.];
    page.number = "A-201".into();
    page.info = DrawingInfo {
        title: "Makerspace section A-A".into(),
        ..info
    };
    let prepared = prepare(
        &session,
        session.revision(),
        actor(),
        book,
        PageAction::Replace(page),
    )?;
    checked(&mut session, prepared)?;

    // One export set selects the tagged equipment and every drawing page.
    let set = EntityId::new();
    let definition_set = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Project,
            tags: Some(TagQuery {
                all: [EQUIPMENT_TAG.into()].into(),
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
    };
    let prepared = prepare_set(
        &session,
        session.revision(),
        actor(),
        set,
        "Makerspace delivery",
        &definition_set,
    )?;
    session.commit(prepared)?;
    Ok(Scenario {
        session,
        room_width: room_width.id,
        room_depth: room_depth.id,
        concrete: concrete.id,
        ground,
        mezzanine,
        wall_type,
        door,
        window,
        walls,
        floor: floor.output.body,
        deck: deck.output.body,
        stair: stair.output.body,
        stair_feature,
        definition,
        parameters,
        equipment,
        plan_guide,
        section_guide,
        book,
        plan_page,
        room_dimension: ids.0,
        bench_dimension: ids.1,
        set,
    })
}

/// Preview and run the export set into `root`; returns the receipt.
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

pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let scenario = build()?;
    let project = directory.join("makerspace.multicad");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&project)?;
    file.write_all(&archive(&scenario.session)?)?;
    file.sync_all()?;
    let model =
        Compiler::default().compile(scenario.session.document(), scenario.session.revision())?;
    let clearances = model
        .envelopes(scenario.session.document())?
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "envelope": scenario.session.document().get(e.envelope).map(|x| x.name.clone()),
                "volume_mm3": e.volume_mm3,
                "conflicts": e.conflicts.len(),
            })
        })
        .collect::<Vec<_>>();
    let exports = directory.join("exports");
    std::fs::create_dir(&exports)?;
    let exports = exports.canonicalize()?;
    let receipt = export(&scenario.session, scenario.set, &exports)?;
    let manifest = serde_json::json!({
        "generator": "makerspace_review",
        "project": project,
        "revision": scenario.session.revision(),
        "clearances": clearances,
        "exports": receipt.files.iter().map(|f| &f.artifact.output.filename).collect::<Vec<_>>(),
        "export_directory": receipt.directory,
        "verification": {
            "native_geometry": true,
            "interactive_macos": false,
            "windows_linux_runtime": false,
            "human_acceptance": false,
            "building_code_or_egress": false,
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
        .ok_or("Usage: cargo run -p multicad-app --example makerspace_review -- NEW_DIRECTORY")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&generate(Path::new(&path))?)?
    );
    Ok(())
}
