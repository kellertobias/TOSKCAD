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
        output: model::BodyOutput {
            body: EntityId::new(),
        },
        wall_type: f.wall_type,
        path: vec![point(0., 0.), point(4000., 0.), point(4000., 3000.)],
        closed: false,
        base_z: length(0.),
        thickness: None,
        height: None,
        justification: None,
        openings: vec![opening(f.door, 0, 500.), opening(f.window, 1, 1000.)],
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
        path: vec![
            point(0., 0.),
            point(5000., 0.),
            point(5000., 4000.),
            point(0., 4000.),
        ],
        closed: true,
        openings: vec![opening(f.door, 0, 1000.), opening(f.window, 2, 1500.)],
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
