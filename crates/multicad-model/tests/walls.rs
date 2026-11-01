//! Native walls with reusable wall, door and window types: exact solids with
//! cut openings, type updates versus instance overrides, materials, closed
//! rooms, diagnostics, plan sections with door swings, and save/reopen.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId},
    drawings::*,
    materials::*,
    model::{
        self, Hinge, Justification, Opening, OpeningKind, OpeningTypeDefinition, Swing,
        WallDefinition, WallRequest, WallTypeDefinition, length,
    },
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
use multicad_model::{Compiler, DrawingProviders, Model};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64, t: f64) {
    assert!((a - b).abs() <= t, "{a} != {b} (±{t})");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn point(x: f64, y: f64) -> [multicad_core::constraint_sketch::Dimension; 2] {
    [length(x), length(y)]
}
fn opening(ty: EntityId, segment: usize, offset: f64) -> Opening {
    Opening {
        id: EntityId::new(),
        opening_type: ty,
        segment,
        offset: length(offset),
        width: None,
        height: None,
        sill: None,
    }
}

struct Fixture {
    s: Session,
    material: EntityId,
    wall_type: EntityId,
    door: EntityId,
    window: EntityId,
}
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Walls").unwrap()).unwrap();
    let material = Entity::new(
        "Concrete",
        EntityData::Material {
            density_kg_m3: Some(2400.),
            color_rgba: [180, 180, 175, 255],
            section_hatch: None,
        },
    );
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        pattern: HatchPattern::Lines,
        angle_deg: 45.,
        spacing_mm: 2.,
        line_width_mm: 0.18,
    })
    .entity(EntityId::new(), "Concrete section")
    .unwrap();
    s.commit(prepare_library(&s, 0, actor(), &[material.clone(), hatch.clone()]).unwrap())
        .unwrap();
    let style = LibraryDefinition::MaterialStyle(MaterialStyle {
        material: material.id,
        appearance: None,
        hatch: Some(LibraryRef::capture(s.document(), hatch.id).unwrap()),
    })
    .entity(EntityId::new(), "Defaults")
    .unwrap();
    s.commit(prepare_library(&s, s.revision(), actor(), &[style]).unwrap())
        .unwrap();
    let (wall_type, p) = model::prepare_wall_type(
        &s,
        s.revision(),
        actor(),
        "Concrete 200",
        &WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: Some(material.id),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let (door, p) = model::prepare_opening_type(
        &s,
        s.revision(),
        actor(),
        "Door 900",
        &OpeningTypeDefinition {
            kind: OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Left,
            },
            width: length(900.),
            height: length(2100.),
            sill: length(0.),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let (window, p) = model::prepare_opening_type(
        &s,
        s.revision(),
        actor(),
        "Window 1200",
        &OpeningTypeDefinition {
            kind: OpeningKind::Window,
            width: length(1200.),
            height: length(1200.),
            sill: length(900.),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    Fixture {
        s,
        material: material.id,
        wall_type,
        door,
        window,
    }
}
fn l_wall(f: &Fixture) -> WallDefinition {
    WallDefinition {
        layer_placement: None,
        output: model::BodyOutput {
            body: EntityId::new(),
        },
        wall_type: f.wall_type,
        path: vec![point(0., 0.), point(4000., 0.), point(4000., 3000.)],
        closed: false,
        level: None,
        base_z: length(0.),
        thickness: None,
        height: None,
        justification: None,
        openings: vec![opening(f.door, 0, 500.), opening(f.window, 1, 1000.)],
        cutouts: vec![],
    }
}
fn create(f: &mut Fixture, name: &str, wall: WallDefinition) -> (EntityId, EntityId) {
    let request = WallRequest::new(name, wall);
    let prepared = model::prepare_wall(&f.s, f.s.revision(), actor(), &request).unwrap();
    Compiler::default()
        .compile(prepared.prepared.document(), f.s.revision())
        .unwrap();
    f.s.commit(prepared.prepared).unwrap();
    (request.feature, request.body)
}
fn volume(m: &Model, feature: EntityId) -> f64 {
    m.snapshot().shapes[&feature].volume_mm3
}
const DOOR: f64 = 900. * 200. * 2100.;
const WINDOW: f64 = 1200. * 200. * 1200.;

#[test]
fn walls_cut_openings_follow_types_and_keep_overrides() {
    let mut f = fixture();
    let source = l_wall(&f);
    let (wall, body) = create(&mut f, "L wall", source);
    let m = compile(&f.s);
    let shape = &m.snapshot().shapes[&wall];
    assert_eq!(shape.solid_count, 1);
    // A centered miter keeps centerline length × thickness in plan.
    near(shape.volume_mm3, 7000. * 200. * 2500. - DOOR - WINDOW, 1e-3);
    near(shape.bounds.min[0], 0., 1e-6);
    near(shape.bounds.min[1], -100., 1e-6);
    near(shape.bounds.max[0], 4100., 1e-6);
    near(shape.bounds.max[1], 3000., 1e-6);
    near(shape.bounds.max[2], 2500., 1e-6);
    // Faces carry stable wall and opening roles for references.
    let roles: Vec<_> = shape
        .topology
        .elements
        .iter()
        .flat_map(|e| e.origins.iter().map(|o| o.role.clone()))
        .collect();
    let source = model::read_wall(f.s.document(), wall).unwrap();
    for role in [
        "wall.seg.0.left".to_string(),
        "wall.start".to_string(),
        format!("opening.{}.head", source.openings[1].id),
        format!("opening.{}.jamb.start", source.openings[0].id),
    ] {
        assert!(roles.iter().any(|r| r.contains(&role)), "missing {role}");
    }
    // The type's material is on the wall Part.
    assert!(
        matches!(&f.s.document().get(body).unwrap().data, EntityData::Part { material: Some(m), .. } if *m == f.material)
    );

    // A second wall overrides its height and keeps it through a type edit.
    let mut straight = l_wall(&f);
    straight.path = vec![point(0., 6000.), point(5000., 6000.)];
    straight.height = Some(length(3000.));
    straight.openings = vec![];
    let (tall, _) = create(&mut f, "Tall wall", straight);
    near(volume(&compile(&f.s), tall), 5000. * 200. * 3000., 1e-3);

    let before = f.s.document().clone();
    let p = model::prepare_wall_type_edit(
        &f.s,
        f.s.revision(),
        actor(),
        f.wall_type,
        &WallTypeDefinition {
            thickness: length(250.),
            height: length(2700.),
            justification: Justification::Center,
            material: Some(f.material),
        },
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let m = compile(&f.s);
    near(
        volume(&m, wall),
        7000. * 250. * 2700. - 900. * 250. * 2100. - 1200. * 250. * 1200.,
        1e-3,
    );
    // Height override kept, thickness follows the type.
    near(volume(&m, tall), 5000. * 250. * 3000., 1e-3);
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &before);

    // Opening overrides and edits rebuild the cut.
    let mut edited = model::read_wall(f.s.document(), wall).unwrap();
    edited.openings[0].width = Some(length(1000.));
    let p = model::prepare_wall_edit(&f.s, f.s.revision(), actor(), wall, &edited).unwrap();
    f.s.commit(p.prepared).unwrap();
    near(
        volume(&compile(&f.s), wall),
        7000. * 200. * 2500. - 1000. * 200. * 2100. - WINDOW,
        1e-3,
    );

    // Unsupported openings are refused with a reason; the source is unchanged.
    let unchanged = f.s.document().clone();
    for (edit, reason) in [
        (
            {
                let mut w = edited.clone();
                w.openings[0].offset = length(3500.);
                w
            },
            "does not fit",
        ),
        (
            {
                let mut w = edited.clone();
                w.openings[1].height = Some(length(1700.));
                w
            },
            "wall top",
        ),
        (
            {
                let mut w = edited.clone();
                w.openings[1].segment = 0;
                w.openings[1].offset = length(1000.);
                w
            },
            "overlaps",
        ),
    ] {
        let message = model::prepare_wall_edit(&f.s, f.s.revision(), actor(), wall, &edit)
            .err()
            .unwrap()
            .to_string();
        assert!(message.contains(reason), "{message}");
    }
    assert_eq!(f.s.document(), &unchanged);

    // A material change on the type follows walls that still use it.
    let other = Entity::new(
        "Brick",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 80, 60, 255],
            section_hatch: None,
        },
    );
    f.s.commit(
        prepare_library(&f.s, f.s.revision(), actor(), std::slice::from_ref(&other)).unwrap(),
    )
    .unwrap();
    let p = model::prepare_wall_type_edit(
        &f.s,
        f.s.revision(),
        actor(),
        f.wall_type,
        &WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: Some(other.id),
        },
    )
    .unwrap();
    f.s.commit(p).unwrap();
    assert!(
        matches!(&f.s.document().get(body).unwrap().data, EntityData::Part { material: Some(m), .. } if *m == other.id)
    );

    // Save/reopen keeps the editable walls and their regenerated solids.
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(f.s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert!(opened.capabilities().contains(model::WALL_CAPABILITY));
    assert_eq!(&opened.document, f.s.document());
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(volume(&reopened, wall), volume(&compile(&f.s), wall), 1e-6);
}

#[test]
fn closed_room_and_plan_section_show_door_swings_and_window_panes() {
    let mut f = fixture();
    let room = WallDefinition {
        layer_placement: None,
        path: vec![
            point(0., 0.),
            point(5000., 0.),
            point(5000., 4000.),
            point(0., 4000.),
        ],
        closed: true,
        openings: vec![opening(f.door, 0, 1000.), opening(f.window, 2, 1500.)],
        cutouts: vec![],
        ..l_wall(&f)
    };
    let (wall, body) = create(&mut f, "Room", room);
    let m = compile(&f.s);
    near(
        volume(&m, wall),
        (5200. * 4200. - 4800. * 3800.) * 2500. - DOOR - WINDOW,
        1e-3,
    );
    assert_eq!(m.snapshot().shapes[&wall].solid_count, 1);

    // A plan section cut at 1 m, looking down.
    let mut page = Page::new("A-201");
    page.settings.paper = PaperSize::A3;
    let plan = DrawingView {
        id: EntityId::new(),
        name: "Plan".into(),
        source: ViewSource::Model {
            entities: vec![body],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Section {
            offset_mm: 1000.,
            range: SectionRange::SectionOnly,
            window: SectionWindow::default(),
        },
        position_mm: [100., 250.],
        scale: Scale {
            numerator: 1,
            denominator: 50,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    };
    let view_id = plan.id;
    page.views.push(plan);
    let (_, p) = prepare_create(&f.s, f.s.revision(), actor(), "Plans", Some(page)).unwrap();
    f.s.commit(p).unwrap();
    let m = compile(&f.s);
    let evaluation = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&view_id] else {
        panic!("{:?}", evaluation.views[&view_id]);
    };
    assert!(vectors.iter().any(|v| v.role == VectorRole::Section));
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
    // The material's hatch style replaces the default wall hatch.
    assert!(
        vectors
            .iter()
            .filter(|v| v.role == VectorRole::Hatch)
            .all(|v| v.line_width_mm == 0.18)
    );
    // The door swing: a 900 mm quarter arc hinged at the start jamb on the
    // left (inner) face of segment 0, at (1000, 100).
    let arc = vectors
        .iter()
        .find_map(|v| match &v.curve {
            VectorCurve::Ellipse {
                center,
                major_mm,
                start_rad,
                end_rad,
                ..
            } if (*major_mm - 900.).abs() < 1e-9 => Some((*center, *end_rad - *start_rad)),
            _ => None,
        })
        .expect("door swing arc");
    near(arc.0[0], 1000., 1e-9);
    near(arc.0[1], 100., 1e-9);
    near(arc.1, std::f64::consts::FRAC_PI_2, 1e-9);
    // Window panes run along segment 2 (y = 4000) across the opening.
    let panes = vectors
        .iter()
        .filter(|v| matches!(v.curve, VectorCurve::Line { a, b } if (a[1] - b[1]).abs() < 1e-9 && (a[1] - 4000.).abs() < 40. && ((a[0] - b[0]).abs() - 1200.).abs() < 1e-9))
        .count();
    assert_eq!(panes, 2);
}

#[test]
fn room_tools_place_openings_and_vertical_cutouts_natively_and_survive_reopen() {
    use model::{VerticalCutout, WallFace, WallOpeningChange, prepare_wall_opening};
    let mut f = fixture();
    let straight = WallDefinition {
        layer_placement: None,
        path: vec![point(0., 0.), point(6000., 0.)],
        openings: vec![],
        ..l_wall(&f)
    };
    let (wall, _) = create(&mut f, "Straight", straight);
    let full = 6000. * 200. * 2500.;
    let apply = |f: &mut Fixture, change: WallOpeningChange| {
        let prepared =
            prepare_wall_opening(&f.s, f.s.revision(), actor(), wall, &[], &change).unwrap();
        Compiler::default()
            .compile(prepared.prepared.document(), f.s.revision())
            .unwrap();
        f.s.commit(prepared.prepared).unwrap();
    };
    let door = opening(f.door, 0, 600.);
    apply(&mut f, WallOpeningChange::AddOpening(door.clone()));
    let window = opening(f.window, 0, 2000.);
    apply(&mut f, WallOpeningChange::AddOpening(window));
    // A 300 mm wide, 80 mm deep chase from the left face, full height.
    let chase = VerticalCutout {
        id: EntityId::new(),
        segment: 0,
        offset: length(4000.),
        width: length(300.),
        depth: Some(length(80.)),
        face: WallFace::Left,
    };
    apply(&mut f, WallOpeningChange::AddCutout(chase.clone()));
    // A full-depth gap at the wall end shortens the wall by 500 mm.
    let gap = VerticalCutout {
        id: EntityId::new(),
        segment: 0,
        offset: length(5500.),
        width: length(500.),
        depth: None,
        face: WallFace::Left,
    };
    apply(&mut f, WallOpeningChange::AddCutout(gap.clone()));
    let m = compile(&f.s);
    let shape = &m.snapshot().shapes[&wall];
    assert_eq!(shape.solid_count, 1);
    let cut = DOOR + WINDOW + 300. * 80. * 2500. + 500. * 200. * 2500.;
    near(shape.volume_mm3, full - cut, 1e-3);
    near(shape.bounds.max[0], 5500., 1e-6);
    let roles: Vec<_> = shape
        .topology
        .elements
        .iter()
        .flat_map(|e| e.origins.iter().map(|o| o.role.clone()))
        .collect();
    for role in [
        format!("cutout.{}.back", chase.id),
        format!("cutout.{}.jamb.start", chase.id),
        format!("cutout.{}.jamb.start", gap.id),
    ] {
        assert!(roles.iter().any(|r| r.contains(&role)), "missing {role}");
    }
    assert!(model::required_capabilities(f.s.document()).contains(model::WALL_CUTOUT_CAPABILITY));

    // Moving the door is an edit of the same opening identity.
    let mut moved = door.clone();
    moved.offset = length(1000.);
    apply(&mut f, WallOpeningChange::ReplaceOpening(moved));
    near(volume(&compile(&f.s), wall), full - cut, 1e-3);

    // Save/reopen preserves the openings, cutouts and the rebuilt solid.
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(f.s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert!(
        opened
            .capabilities()
            .contains(model::WALL_CUTOUT_CAPABILITY)
    );
    assert_eq!(&opened.document, f.s.document());
    let reopened = model::read_wall(&opened.document, wall).unwrap();
    assert_eq!((reopened.openings.len(), reopened.cutouts.len()), (2, 2));
    let rebuilt = Compiler::default().compile(&opened.document, 0).unwrap();
    near(volume(&rebuilt, wall), full - cut, 1e-3);
    // A build without cutout support refuses the file instead of misreading it.
    let mut older = project_file::supported_capabilities();
    older.remove(model::WALL_CUTOUT_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &older).is_err());
}

/// A line vector whose ends both lie on `y` and which spans `x0..x1`.
fn spans_x(v: &DrawingVector, y: f64, x0: f64, x1: f64) -> bool {
    matches!(v.curve, VectorCurve::Line { a, b }
        if (a[1] - y).abs() < 1e-6 && (b[1] - y).abs() < 1e-6
            && a[0].min(b[0]) <= x0 && a[0].max(b[0]) >= x1)
}
/// A line vector from `y0` to `y1` at `x`.
fn jamb(v: &DrawingVector, x: f64, y0: f64, y1: f64) -> bool {
    matches!(v.curve, VectorCurve::Line { a, b }
        if (a[0] - x).abs() < 1e-6 && (b[0] - x).abs() < 1e-6
            && (a[1].min(b[1]) - y0).abs() < 1e-6 && (a[1].max(b[1]) - y1).abs() < 1e-6)
}
fn plan_view(body: EntityId, kind: ViewKind) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: "Plan".into(),
        source: ViewSource::Model {
            entities: vec![body],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind,
        position_mm: [200., 150.],
        scale: Scale {
            numerator: 1,
            denominator: 50,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    }
}

/// The review room: 4 x 3 m, a 900 x 2100 door and a 1200 x 1200 window at
/// a 900 mm sill, with a wall type that has no material (the UI default).
#[test]
fn default_room_plan_hatches_cut_walls_with_real_door_and_window_voids() {
    let mut f = fixture();
    let (plain, p) = model::prepare_wall_type(
        &f.s,
        f.s.revision(),
        actor(),
        "Plain 200",
        &WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: None,
        },
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let room = WallDefinition {
        layer_placement: None,
        wall_type: plain,
        path: vec![
            point(0., 0.),
            point(4000., 0.),
            point(4000., 3000.),
            point(0., 3000.),
        ],
        closed: true,
        // Door 500..1400 on the south wall; window 2600..1400 on the north.
        openings: vec![opening(f.door, 0, 500.), opening(f.window, 2, 1400.)],
        cutouts: vec![],
        ..l_wall(&f)
    };
    let (wall, body) = create(&mut f, "Room", room);
    let manufacturing = (4200. * 3200. - 3800. * 2800.) * 2500. - DOOR - WINDOW;

    // The default view a user places: a top projection. Sections at 800 mm
    // (below the window sill) and 2300 mm (above both heads) check the
    // response to the cut height.
    let mut page = Page::new("A-101");
    page.settings.paper = PaperSize::A3;
    let default_view = plan_view(body, ViewKind::Projection);
    let low = plan_view(
        body,
        ViewKind::Section {
            offset_mm: 800.,
            range: SectionRange::Depth { depth_mm: 900. },
            window: SectionWindow::default(),
        },
    );
    let high = plan_view(
        body,
        ViewKind::Section {
            offset_mm: 2300.,
            range: SectionRange::Depth { depth_mm: 2400. },
            window: SectionWindow::default(),
        },
    );
    // Side by side on the sheet: default, low cut, high cut.
    let (mut default_view, mut low, mut high) = (default_view, low, high);
    default_view.position_mm = [80., 190.];
    low.position_mm = [200., 190.];
    high.position_mm = [320., 190.];
    let ids = [default_view.id, low.id, high.id];
    page.views.extend([default_view, low, high]);
    let page_id = page.id;
    let (book, p) = prepare_create(&f.s, f.s.revision(), actor(), "Plans", Some(page)).unwrap();
    f.s.commit(p).unwrap();
    let m = compile(&f.s);
    near(volume(&m, wall), manufacturing, 1e-3);
    let evaluation = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    let vectors = |id: EntityId| match &evaluation.views[&id] {
        ViewEvaluation::Ready { vectors, .. } => vectors.clone(),
        other => panic!("{other:?}"),
    };

    // Default plan: cut walls hatched with the default pattern and weights.
    let plan = vectors(ids[0]);
    let role = |r: VectorRole| plan.iter().filter(move |v| v.role == r);
    assert!(role(VectorRole::Hatch).count() > 20);
    assert!(role(VectorRole::Hatch).all(|v| v.line_width_mm == 0.13));
    assert!(role(VectorRole::Section).all(|v| v.line_width_mm == 0.5));
    // The doorway is a gap: no cut line or hatch crosses it, jambs close it.
    for y in [-100., 100.] {
        assert!(
            !plan
                .iter()
                .any(|v| v.role != VectorRole::Hidden && spans_x(v, y, 600., 1300.)),
            "a line continues across the doorway at y = {y}"
        );
    }
    assert!(!role(VectorRole::Hatch).any(|v| {
        v.preview_mm
            .iter()
            .any(|p| p[0] > 510. && p[0] < 1390. && p[1].abs() < 95.)
    }));
    for x in [500., 1400.] {
        assert!(role(VectorRole::Section).any(|v| jamb(v, x, -100., 100.)));
    }
    // The door leaf and its 900 mm swing remain.
    let swing = |vs: &[DrawingVector]| {
        vs.iter().any(|v| {
            matches!(v.curve, VectorCurve::Ellipse { major_mm, .. } if (major_mm - 900.).abs() < 1e-9)
        })
    };
    assert!(swing(&plan));
    // The window is cut (sill 900 <= 1000 < head 2100): jambs, two panes,
    // and its sill seen below the cut as thinner projected lines.
    for x in [1400., 2600.] {
        assert!(role(VectorRole::Section).any(|v| jamb(v, x, 2900., 3100.)));
    }
    for y in [2900., 3100.] {
        assert!(!role(VectorRole::Section).any(|v| spans_x(v, y, 1500., 2500.)));
        assert!(
            role(VectorRole::Visible)
                .any(|v| spans_x(v, y, 1400., 2600.) && v.line_width_mm == 0.18),
            "sill edge at y = {y}"
        );
    }
    let panes = role(VectorRole::Visible)
        .filter(|v| {
            matches!(v.curve, VectorCurve::Line { a, b }
                if (a[1] - b[1]).abs() < 1e-9 && (a[1] - 3000.).abs() < 40.
                    && ((a[0] - b[0]).abs() - 1200.).abs() < 1e-9)
        })
        .count();
    assert_eq!(panes, 2);
    assert!(role(VectorRole::Hidden).next().is_none());

    // Cut below the window sill: the wall is cut through under the window
    // and the window is a dashed outline; the door is still cut.
    let low = vectors(ids[1]);
    assert!(
        low.iter()
            .any(|v| v.role == VectorRole::Section && spans_x(v, 3100., 1500., 2500.))
    );
    assert_eq!(
        low.iter().filter(|v| v.role == VectorRole::Hidden).count(),
        4
    );
    assert!(swing(&low));
    // Cut above both heads: the wall is continuous; both openings dashed.
    let high = vectors(ids[2]);
    assert!(
        high.iter()
            .any(|v| v.role == VectorRole::Section && spans_x(v, -100., 600., 1300.))
    );
    assert!(!swing(&high));
    assert_eq!(
        high.iter().filter(|v| v.role == VectorRole::Hidden).count(),
        8
    );

    // The native sheet and PDF carry the same hierarchy.
    let render =
        render::render_page(f.s.document(), f.s.revision(), book, page_id, &evaluation).unwrap();
    assert!(
        render
            .polylines
            .iter()
            .any(|p| p.layer == render::Layer::Hatch && p.width_mm == 0.13)
    );
    assert!(
        render
            .polylines
            .iter()
            .any(|p| p.layer == render::Layer::Hidden && p.dashed)
    );
    if let Ok(path) = std::env::var("MULTICAD_PLAN_SVG") {
        std::fs::write(path, svg::render_svg(&render)).unwrap();
    }
    let pdf = pdf::book_pdf(
        f.s.document(),
        f.s.revision(),
        &[(book, page_id)],
        &evaluation,
    )
    .unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("0.369 w"), "default hatch weight in the PDF");
    assert!(text.contains("1.417 w"), "cut wall weight in the PDF");

    // A thicker wall regenerates the cut: jambs now span ±150.
    let mut edited = model::read_wall(f.s.document(), wall).unwrap();
    edited.thickness = Some(length(300.));
    let p = model::prepare_wall_edit(&f.s, f.s.revision(), actor(), wall, &edited).unwrap();
    f.s.commit(p.prepared).unwrap();
    let m = compile(&f.s);
    let evaluation = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&ids[0]] else {
        panic!("{:?}", evaluation.views[&ids[0]]);
    };
    assert!(
        vectors
            .iter()
            .any(|v| v.role == VectorRole::Section && jamb(v, 500., -150., 150.))
    );
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
}
