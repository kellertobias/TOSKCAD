//! Parts placed by an Assembly stay usable as sources: a drawing view of a
//! member Instance projects its world-placed leaf when one placed copy holds
//! it (and refuses when several do), a view of the definition Part shows the
//! component in its own coordinates, and a sheet Part placed by an Assembly
//! keeps its sheet record, hardware record and developed flat pattern.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    drawings::*,
    materials::*,
    model::{
        self, AssemblyRequest, BodyOutput, ComponentPlacementRequest, CutoutFace, HardwarePreset,
        HardwareSide, MountingPattern, NumericTransform, PanelPlane, PrimitiveParameters,
        PrimitiveRequest, SheetBase, SheetDefinition, SheetRequest, Thread, length,
    },
    references::ReferenceFrame,
};
use multicad_model::*;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn place(s: &mut Session, component: EntityId, x: f64) -> EntityId {
    let request = ComponentPlacementRequest::new(
        component,
        NumericTransform {
            translation: [length(x), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let prepared = model::prepare_place_component(s, s.revision(), actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    request.instance
}
fn assembly(s: &mut Session, members: Vec<EntityId>, parts: Vec<EntityId>) -> EntityId {
    let request = AssemblyRequest::new("Unit", members, parts);
    let prepared = model::prepare_assembly(s, s.revision(), actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    request.assembly
}
fn view(entities: Vec<EntityId>) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: "Top".into(),
        source: ViewSource::Model { entities },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [80., 70.],
        scale: Scale {
            numerator: 1,
            denominator: 1,
        },
        hidden_lines: true,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
    }
}
/// The x range of a ready view's vectors in view millimetres.
fn x_range(model: &Model, s: &Session, view: &DrawingView) -> Result<[f64; 2], ModelError> {
    let ViewEvaluation::Ready { vectors, .. } = model.drawing_view(
        s.document(),
        s.revision(),
        view,
        &DrawingProviders::default(),
    )?
    else {
        panic!("view is not ready");
    };
    let xs = vectors.iter().flat_map(|v| &v.preview_mm).map(|p| p[0]);
    Ok([
        xs.clone().fold(f64::INFINITY, f64::min),
        xs.fold(f64::NEG_INFINITY, f64::max),
    ])
}

#[test]
fn member_instances_and_definition_parts_are_drawing_sources() {
    let mut s = Session::new(Document::new("Members").unwrap()).unwrap();
    let block = PrimitiveRequest::new(
        "Block",
        PrimitiveParameters::Box {
            size: [length(40.), length(30.), length(20.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, s.revision(), actor(), &block).unwrap();
    s.commit(prepared.prepared).unwrap();
    let member = place(&mut s, block.body, 100.);
    let unit = assembly(&mut s, vec![member], vec![block.body]);
    let first = place(&mut s, unit, 0.);
    let model = compile(&s);
    // The definition Part is no physical body, but a view can still show it.
    assert!(!model.snapshot().bodies.contains_key(&block.body));
    let [lo, hi] = x_range(&model, &s, &view(vec![member])).unwrap();
    assert!(
        (lo - 100.).abs() < 1e-6 && (hi - 140.).abs() < 1e-6,
        "{lo} {hi}"
    );
    let [lo, hi] = x_range(&model, &s, &view(vec![block.body])).unwrap();
    assert!(lo.abs() < 1e-6 && (hi - 40.).abs() < 1e-6, "{lo} {hi}");
    let [_, hi] = x_range(&model, &s, &view(vec![first])).unwrap();
    assert!((hi - 140.).abs() < 1e-6);

    // A second placed copy makes the member ambiguous; the placed copies
    // and the definition stay selectable.
    let second = place(&mut s, unit, 300.);
    let model = compile(&s);
    let error = x_range(&model, &s, &view(vec![member])).unwrap_err();
    assert!(error.to_string().contains("2 placed copies"), "{error}");
    let [_, hi] = x_range(&model, &s, &view(vec![second])).unwrap();
    assert!((hi - 440.).abs() < 1e-6);
    let [_, hi] = x_range(&model, &s, &view(vec![block.body])).unwrap();
    assert!((hi - 40.).abs() < 1e-6);
}

#[test]
fn a_sheet_part_placed_by_an_assembly_keeps_its_records_and_flat_pattern() {
    let mut s = Session::new(Document::new("Placed sheet").unwrap()).unwrap();
    let alu = Entity::new(
        "AlMg3",
        EntityData::Material {
            density_kg_m3: Some(2660.),
            color_rgba: [180, 184, 190, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: alu.id,
        specification: StockKind::Sheet {
            thickness_mm: 1.5,
            length_mm: 1000.,
            width_mm: 500.,
            inside_bend_radius_mm: 1.5,
            k_factor: 0.44,
        },
    })
    .entity(EntityId::new(), "AlMg3 1.5")
    .unwrap();
    let stock_id = stock.id;
    s.commit(prepare_library(&s, 0, actor(), &[alu, stock]).unwrap())
        .unwrap();
    let sheet = SheetRequest::new(
        "Plate",
        SheetDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            stock: stock_id,
            base: SheetBase::Rectangle {
                plane: PanelPlane::Xy,
                origin: [length(0.), length(0.), length(0.)],
                length: length(100.),
                width: length(60.),
            },
            reverse: false,
            inside_radius: None,
            k_factor: None,
            flanges: vec![],
            cutouts: vec![],
            fasteners: vec![MountingPattern {
                preset: HardwarePreset::Screw {
                    thread: Thread::M3,
                    length: length(8.),
                    washers: false,
                    nut: false,
                },
                face: CutoutFace::Base,
                side: HardwareSide::Material,
                points: vec![
                    [length(10.), length(10.)],
                    [length(90.), length(10.)] as [Dimension; 2],
                ],
            }],
        },
    );
    let prepared = model::prepare_sheet(&s, s.revision(), actor(), &sheet).unwrap();
    s.commit(prepared.prepared).unwrap();
    let prepared = model::prepare_sheet_fasteners(
        &s,
        s.revision(),
        actor(),
        sheet.feature,
        &model::read_sheet(s.document(), sheet.feature)
            .unwrap()
            .fasteners,
    )
    .unwrap();
    let screws = prepared.hardware[0].2.body;
    s.commit(prepared.model.prepared).unwrap();
    let before = compile(&s);
    let flat = before.sheet_metal(s.document()).unwrap()[0].flat_area_mm2;

    let members = vec![place(&mut s, sheet.body, 0.), place(&mut s, screws, 0.)];
    let unit = assembly(&mut s, members, vec![sheet.body, screws]);
    place(&mut s, unit, 0.);
    let model = compile(&s);
    assert!(!model.snapshot().bodies.contains_key(&sheet.body));
    let records = model.sheet_metal(s.document()).unwrap();
    assert_eq!(records.len(), 1);
    assert!((records[0].flat_area_mm2 - flat).abs() < 1e-9);
    let hardware = model.sheet_hardware(s.document()).unwrap();
    assert_eq!(hardware.len(), 1);
    assert_eq!(hardware[0].quantity, 2);
    let pattern = model.unfold(s.document(), sheet.body).unwrap();
    assert_eq!(pattern.drills.len(), 2);
}
