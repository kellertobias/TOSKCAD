//! Vertical cutouts choose what they cut: a wall, floor slabs over a range
//! of levels (a shaft), or both. Native volumes prove each slab loses exactly
//! the cutout rectangle; refusals leave the source unchanged.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{
        self, BodyOutput, CutoutChange, CutoutPlacement, Justification, LevelDefinition, NewType,
        NewTypeKind, PlanRectangle, SlabDefinition, SlabKind, VerticalCutout, WallChange,
        WallDefinition, WallFace, WallRequest, WallTypeDefinition, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, Model};

const SLAB: f64 = 6000. * 4000. * 200.;
const WALL: f64 = 4000. * 200. * 2500.;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-3, "{a} != {b}");
}
fn pt(x: f64, y: f64) -> [Dimension; 2] {
    [length(x), length(y)]
}
fn compile(document: &Document) -> Model {
    Compiler::default().compile(document, 0).unwrap()
}
fn volume(m: &Model, feature: EntityId) -> f64 {
    m.snapshot().shapes[&feature].volume_mm3
}

struct Building {
    s: Session,
    ground: EntityId,
    upper: EntityId,
    ground_slab: EntityId,
    upper_slab: EntityId,
    wall: EntityId,
}
/// Two levels 3 m apart, a 6 x 4 m floor slab on each, and an open 4 m wall
/// (200 thick, 2.5 m high) on the ground level along y = 1000.
fn building() -> Building {
    let mut s = Session::new(Document::new("Shafts").unwrap()).unwrap();
    let level = |s: &mut Session, name: &str, z: f64| {
        let (id, p) = model::prepare_level(
            s,
            s.revision(),
            actor(),
            name,
            &LevelDefinition {
                elevation: length(z),
                height: length(3000.),
                slab_thickness: length(200.),
            },
        )
        .unwrap();
        s.commit(p).unwrap();
        id
    };
    let ground = level(&mut s, "Ground", 0.);
    let upper = level(&mut s, "Upper", 3000.);
    let slab = |s: &mut Session, name: &str, level: EntityId| {
        let p = model::prepare_slab(
            s,
            s.revision(),
            actor(),
            name,
            &SlabDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                level,
                kind: SlabKind::Slab,
                outline: vec![pt(0., 0.), pt(6000., 0.), pt(6000., 4000.), pt(0., 4000.)],
                thickness: None,
                holes: vec![],
            },
            None,
        )
        .unwrap();
        let f = p.outputs[0].feature;
        s.commit(p.prepared).unwrap();
        f
    };
    let ground_slab = slab(&mut s, "Ground slab", ground);
    let upper_slab = slab(&mut s, "Upper slab", upper);
    let ty = NewType::new(
        "Block 200",
        NewTypeKind::Wall(WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: None,
        }),
    );
    let request = WallRequest::new(
        "Wall",
        WallDefinition {
            layer_placement: None,
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty.id,
            path: vec![pt(1000., 1000.), pt(5000., 1000.)],
            closed: false,
            level: Some(ground),
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    );
    let wall = request.feature;
    let p = model::prepare_wall_change(
        &s,
        s.revision(),
        actor(),
        &[ty],
        &WallChange::Create(&request),
    )
    .unwrap();
    s.commit(p.prepared).unwrap();
    Building {
        s,
        ground,
        upper,
        ground_slab,
        upper_slab,
        wall,
    }
}
fn rect(cx: f64, cy: f64, w: f64, d: f64) -> PlanRectangle {
    PlanRectangle {
        center: pt(cx, cy),
        width: length(w),
        depth: length(d),
        rotation_deg: 0.,
    }
}
fn apply(s: &mut Session, change: CutoutChange) -> model::ModelPrepared {
    let p = model::prepare_vertical_cutout(s, s.revision(), actor(), &change).unwrap();
    s.commit(p.prepared.clone()).unwrap();
    p
}
fn refused(b: &Building, change: CutoutChange) -> String {
    let before = b.s.document().clone();
    let Err(error) = model::prepare_vertical_cutout(&b.s, b.s.revision(), actor(), &change) else {
        panic!("change should be refused");
    };
    let message = error.to_string();
    assert_eq!(b.s.document(), &before);
    message
}

#[test]
fn a_slab_only_shaft_cuts_every_slab_in_its_level_range_and_survives_reopen() {
    let mut b = building();
    let parameters = multicad_core::parameters::evaluate(b.s.document()).unwrap();
    let shaft = rect(4000., 2500., 600., 800.);
    let id = EntityId::new();
    let placement = CutoutPlacement {
        id,
        wall: None,
        plan: Some(shaft.clone()),
        slabs: vec![],
    };
    let footprint = model::cutout_footprint(b.s.document(), &parameters, &placement).unwrap();
    let slabs = model::slabs_in_levels(
        b.s.document(),
        &parameters,
        b.ground,
        Some(b.upper),
        &footprint.corners(),
    )
    .unwrap();
    assert_eq!(slabs, vec![b.ground_slab, b.upper_slab]);
    // Only its own level, by default.
    assert_eq!(
        model::slabs_in_levels(
            b.s.document(),
            &parameters,
            b.upper,
            None,
            &footprint.corners()
        )
        .unwrap(),
        vec![b.upper_slab]
    );

    let before = b.s.document().clone();
    let revision = b.s.revision();
    let p = apply(
        &mut b.s,
        CutoutChange::place(CutoutPlacement { slabs, ..placement }),
    );
    assert_eq!(b.s.revision(), revision + 1);
    assert_eq!(
        b.s.activity().last().map(|a| a.label.as_str()),
        Some("Place vertical cutout")
    );
    assert_eq!(p.outputs.len(), 2);
    let m = compile(b.s.document());
    let hole = 600. * 800. * 200.;
    near(volume(&m, b.ground_slab), SLAB - hole);
    near(volume(&m, b.upper_slab), SLAB - hole);
    near(volume(&m, b.wall), WALL);
    let roles: Vec<String> = m.snapshot().shapes[&b.upper_slab]
        .topology
        .elements
        .iter()
        .flat_map(|e| e.origins.iter().map(|o| o.role.clone()))
        .collect();
    assert!(
        roles.iter().any(|r| r == &format!("cutout.{id}.side.0")),
        "{roles:?}"
    );
    let members = model::cutout_members(b.s.document(), id);
    assert!(members.wall.is_none());
    assert_eq!(members.slabs.len(), 2);
    assert!(model::required_capabilities(b.s.document()).contains(model::SLAB_HOLE_CAPABILITY));

    // Save/reopen keeps the holes; a build without hole support refuses.
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(b.s.document().clone()).unwrap(),
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
    assert!(opened.capabilities().contains(model::SLAB_HOLE_CAPABILITY));
    assert_eq!(&opened.document, b.s.document());
    near(
        volume(&compile(&opened.document), b.upper_slab),
        SLAB - hole,
    );
    let mut older = project_file::supported_capabilities();
    older.remove(model::SLAB_HOLE_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &older).is_err());

    // Narrowing the shaft to the upper level restores the ground slab.
    apply(
        &mut b.s,
        CutoutChange::place(CutoutPlacement {
            id,
            wall: None,
            plan: Some(shaft),
            slabs: vec![b.upper_slab],
        }),
    );
    assert_eq!(
        b.s.activity().last().map(|a| a.label.as_str()),
        Some("Edit vertical cutout")
    );
    let m = compile(b.s.document());
    near(volume(&m, b.ground_slab), SLAB);
    near(volume(&m, b.upper_slab), SLAB - hole);

    // Removing it is one transaction; two Undos restore the original document.
    apply(&mut b.s, CutoutChange::Remove(id));
    near(volume(&compile(b.s.document()), b.upper_slab), SLAB);
    assert!(!model::required_capabilities(b.s.document()).contains(model::SLAB_HOLE_CAPABILITY));
    for _ in 0..3 {
        b.s.undo(b.s.revision(), actor()).unwrap();
    }
    assert_eq!(b.s.document(), &before);
}

#[test]
fn a_wall_cutout_can_also_cut_the_floor_slab_under_its_footprint() {
    let mut b = building();
    let id = EntityId::new();
    // A 400 mm wide, 100 mm deep chase from the left (+Y) face, 1 m along.
    let chase = VerticalCutout {
        id,
        segment: 0,
        offset: length(1000.),
        width: length(400.),
        depth: Some(length(100.)),
        face: WallFace::Left,
    };
    let parameters = multicad_core::parameters::evaluate(b.s.document()).unwrap();
    let placement = CutoutPlacement {
        id,
        wall: Some((b.wall, chase.clone())),
        plan: None,
        slabs: vec![],
    };
    let footprint = model::cutout_footprint(b.s.document(), &parameters, &placement).unwrap();
    // Wall path y = 1000, left face at y = 1100; the chase spans y 1000–1100.
    near(footprint.center[0], 1000. + 1200.);
    near(footprint.center[1], 1050.);
    near(footprint.width, 400.);
    near(footprint.depth, 100.);
    let slabs = model::slabs_in_levels(
        b.s.document(),
        &parameters,
        b.ground,
        None,
        &footprint.corners(),
    )
    .unwrap();
    assert_eq!(slabs, vec![b.ground_slab]);
    let p = apply(
        &mut b.s,
        CutoutChange::place(CutoutPlacement { slabs, ..placement }),
    );
    assert_eq!(p.outputs[0].feature, b.wall);
    let m = compile(b.s.document());
    near(volume(&m, b.wall), WALL - 400. * 100. * 2500.);
    near(volume(&m, b.ground_slab), SLAB - 400. * 100. * 200.);
    near(volume(&m, b.upper_slab), SLAB);
    let members = model::cutout_members(b.s.document(), id);
    assert_eq!(members.wall.as_ref().map(|(w, _)| *w), Some(b.wall));
    assert_eq!(members.slabs.len(), 1);

    // Wall-only again: the slab hole goes, the wall chase stays.
    apply(
        &mut b.s,
        CutoutChange::place(CutoutPlacement {
            id,
            wall: Some((b.wall, chase)),
            plan: None,
            slabs: vec![],
        }),
    );
    let m = compile(b.s.document());
    near(volume(&m, b.wall), WALL - 400. * 100. * 2500.);
    near(volume(&m, b.ground_slab), SLAB);
}

#[test]
fn misplaced_cutouts_are_refused_without_changing_the_source() {
    let mut b = building();
    let first = EntityId::new();
    apply(
        &mut b.s,
        CutoutChange::place(CutoutPlacement {
            id: first,
            wall: None,
            plan: Some(rect(4000., 2500., 600., 800.)),
            slabs: vec![b.ground_slab],
        }),
    );
    let place = |plan: PlanRectangle, slabs: Vec<EntityId>| {
        CutoutChange::place(CutoutPlacement {
            id: EntityId::new(),
            wall: None,
            plan: Some(plan),
            slabs,
        })
    };
    // Crossing the slab edge, reaching it, or lying outside.
    for plan in [
        rect(5900., 2000., 600., 600.),
        rect(5700., 2000., 600., 600.),
        rect(8000., 2000., 600., 600.),
    ] {
        let message = refused(&b, place(plan, vec![b.ground_slab]));
        assert!(message.contains("not inside the slab outline"), "{message}");
    }
    // Overlapping or touching the existing hole.
    for plan in [
        rect(4200., 2500., 600., 800.),
        rect(4600., 2500., 600., 800.),
    ] {
        let message = refused(&b, place(plan, vec![b.ground_slab]));
        assert!(message.contains("overlaps"), "{message}");
    }
    // Cutting nothing, or the same slab twice.
    let message = refused(&b, place(rect(1000., 3000., 300., 300.), vec![]));
    assert!(message.contains("Choose what the cutout cuts"), "{message}");
    let message = refused(
        &b,
        place(
            rect(1000., 3000., 300., 300.),
            vec![b.ground_slab, b.ground_slab],
        ),
    );
    assert!(message.contains("only once"), "{message}");
    // A wall entity is not a slab.
    let message = refused(&b, place(rect(1000., 3000., 300., 300.), vec![b.wall]));
    assert!(message.contains("not a floor slab"), "{message}");
    // A through-cut inside the open wall would still split it, slab or not.
    let id = EntityId::new();
    let gap = CutoutChange::place(CutoutPlacement {
        id,
        wall: Some((
            b.wall,
            VerticalCutout {
                id,
                segment: 0,
                offset: length(1500.),
                width: length(400.),
                depth: None,
                face: WallFace::Left,
            },
        )),
        plan: None,
        slabs: vec![b.ground_slab],
    });
    let message = refused(&b, gap);
    assert!(message.contains("splits it"), "{message}");
    // Removing an unknown cutout.
    let message = refused(&b, CutoutChange::Remove(EntityId::new()));
    assert!(message.contains("No wall or slab"), "{message}");
    // A slab edit that would leave a hole outside its new outline is refused.
    let slab = model::read_slab(b.s.document(), b.ground_slab).unwrap();
    let smaller = SlabDefinition {
        outline: vec![pt(0., 0.), pt(3000., 0.), pt(3000., 4000.), pt(0., 4000.)],
        ..slab
    };
    let message = model::prepare_slab_edit(&b.s, b.s.revision(), actor(), b.ground_slab, &smaller)
        .err()
        .unwrap()
        .to_string();
    assert!(message.contains("not inside the slab outline"), "{message}");
}
