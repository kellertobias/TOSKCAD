//! Drawing guides: floor plans repeated on levels, a section line and an
//! elevation generate linked views; rerunning updates them, level edits move
//! the cut, deleted guides are reported, and the cut walls are hatched.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    materials::*,
    model::{
        self, BodyOutput, Justification, LevelDefinition, WallDefinition, WallRequest,
        WallTypeDefinition, length,
    },
    project_file::{self, ProjectFile},
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
fn level(s: &mut Session, name: &str, elevation: f64) -> EntityId {
    let (id, p) = model::prepare_level(
        s,
        s.revision(),
        actor(),
        name,
        &LevelDefinition {
            elevation: length(elevation),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    id
}
fn views(s: &Session, book: EntityId) -> Vec<DrawingView> {
    multicad_core::drawings::read(s.document(), book)
        .unwrap()
        .pages
        .into_iter()
        .flat_map(|p| p.views)
        .collect()
}

struct Fixture {
    s: Session,
    ground: EntityId,
    upper: EntityId,
    walls: Vec<EntityId>,
}
/// Two concrete rooms, one per level, with a hatch style for concrete.
fn fixture() -> Fixture {
    let mut s = Session::new(Document::new("Guides").unwrap()).unwrap();
    let material = Entity::new(
        "Concrete",
        EntityData::Material {
            density_kg_m3: Some(2400.),
            color_rgba: [180, 180, 175, 255],
            section_hatch: None,
        },
    );
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        color_rgb: None,
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
    let ground = level(&mut s, "Ground floor", 0.);
    let upper = level(&mut s, "First floor", 3000.);
    let (wall_type, p) = model::prepare_wall_type(
        &s,
        s.revision(),
        actor(),
        "Concrete 200",
        &WallTypeDefinition {
            thickness: length(200.),
            height: length(2800.),
            justification: Justification::Center,
            material: Some(material.id),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let mut walls = vec![];
    for (name, l) in [("Ground room", ground), ("Upper room", upper)] {
        let request = WallRequest::new(
            name,
            WallDefinition {
                layer_placement: None,
                output: BodyOutput {
                    body: EntityId::new(),
                },
                wall_type,
                path: [(0., 0.), (5000., 0.), (5000., 4000.), (0., 4000.)]
                    .map(|(x, y)| [length(x), length(y)])
                    .to_vec(),
                closed: true,
                level: Some(l),
                base_z: length(0.),
                thickness: None,
                height: None,
                justification: None,
                openings: vec![],
                cutouts: vec![],
            },
        );
        walls.push(request.wall.output.body);
        let p = model::prepare_wall(&s, s.revision(), actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
    }
    Fixture {
        s,
        ground,
        upper,
        walls,
    }
}
fn plan(f: &Fixture, cut: f64) -> GuideDefinition {
    GuideDefinition {
        kind: GuideKind::Plan {
            cut_height: length(cut),
            view_depth: length(1200.),
        },
        levels: vec![f.ground, f.upper],
        sources: f.walls.clone(),
        scale: Scale {
            numerator: 1,
            denominator: 50,
        },
    }
}

#[test]
fn plan_guides_generate_linked_views_per_level_and_update_without_duplicates() {
    let mut f = fixture();
    let p = prepare_guide(
        &f.s,
        f.s.revision(),
        actor(),
        None,
        "Floor plans",
        &plan(&f, 1000.),
        true,
        None,
    )
    .unwrap();
    let guide = p.guide;
    let report = p.generation.clone().unwrap();
    assert_eq!(report.created.len(), 2);
    assert!(report.updated.is_empty() && report.stale.is_empty());
    f.s.commit(p.prepared).unwrap();
    let book = report.book;
    let generated = views(&f.s, book);
    assert_eq!(generated.len(), 2);
    let ground_view = generated
        .iter()
        .find(|v| v.guide.unwrap().level == Some(f.ground))
        .unwrap();
    assert_eq!(ground_view.name, "Floor plans – Ground floor");
    near(ground_view.frame.origin_mm[2], 1000., 1e-9);
    assert_eq!(
        ground_view.kind,
        ViewKind::Section {
            offset_mm: 0.,
            range: SectionRange::Depth { depth_mm: 1200. },
            window: SectionWindow::default(),
        }
    );
    let upper_view = generated
        .iter()
        .find(|v| v.guide.unwrap().level == Some(f.upper))
        .unwrap()
        .clone();
    near(upper_view.frame.origin_mm[2], 4000., 1e-9);

    // The cut walls are hatched with their material's section style, and
    // each plan shows only its own level's room.
    let m = compile(&f.s);
    let evaluation = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    for view in &generated {
        let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&view.id] else {
            panic!("{:?}", evaluation.views[&view.id])
        };
        assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
        assert!(vectors.iter().any(|v| v.role == VectorRole::Section));
        let level_wall = if view.guide.unwrap().level == Some(f.ground) {
            f.walls[0]
        } else {
            f.walls[1]
        };
        assert!(
            vectors
                .iter()
                .filter(|v| v.role == VectorRole::Section)
                .all(|v| v.source == level_wall)
        );
    }

    // The guide is visible in the viewport as one cut plane per level.
    let outlines = &m.snapshot().guide_outlines[&guide];
    assert_eq!(outlines.len(), 2);
    near(outlines[0][0][2], 1000., 1e-9);
    near(outlines[1][0][2], 4000., 1e-9);
    let rust = Compiler::default()
        .compile_with_backend(
            f.s.document(),
            f.s.revision(),
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert_eq!(&rust.snapshot().guide_outlines[&guide], outlines);

    // A level edit moves the linked cut without regenerating.
    let p = model::prepare_level_edit(
        &f.s,
        f.s.revision(),
        actor(),
        f.upper,
        &LevelDefinition {
            elevation: length(3300.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let resolved = resolve_guided(
        f.s.document(),
        f.s.parameters(),
        &views(&f.s, book)
            .into_iter()
            .find(|v| v.id == upper_view.id)
            .unwrap(),
    )
    .unwrap();
    near(resolved.frame.origin_mm[2], 4300., 1e-9);
    let m = compile(&f.s);
    let evaluation = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    assert!(matches!(
        evaluation.views[&upper_view.id],
        ViewEvaluation::Ready { .. }
    ));

    // Rerunning with a new cut height updates the same views; a level
    // removed from the guide leaves its view reported as stale.
    let mut edited = plan(&f, 1100.);
    edited.levels = vec![f.ground];
    let p = prepare_guide(
        &f.s,
        f.s.revision(),
        actor(),
        Some(guide),
        "",
        &edited,
        true,
        None,
    )
    .unwrap();
    let report = p.generation.clone().unwrap();
    assert_eq!(report.book, book);
    assert!(report.created.is_empty());
    assert_eq!(report.updated, vec![ground_view.id]);
    assert_eq!(report.stale, vec![upper_view.id]);
    f.s.commit(p.prepared).unwrap();
    let after = views(&f.s, book);
    assert_eq!(after.len(), 2, "no duplicate views");
    near(
        after
            .iter()
            .find(|v| v.id == ground_view.id)
            .unwrap()
            .frame
            .origin_mm[2],
        1100.,
        1e-9,
    );
    let evaluation = compile(&f.s)
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    let ViewEvaluation::Unresolved { message } = &evaluation.views[&upper_view.id] else {
        panic!("a view whose level left the guide must be reported")
    };
    assert!(message.contains("no longer part"), "{message}");

    // Save and reopen keep the guide, its links and the capabilities.
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
    assert!(opened.capabilities().contains(GUIDE_CAPABILITY));
    assert!(opened.capabilities().contains(GUIDED_VIEW_CAPABILITY));
    assert_eq!(&opened.document, f.s.document());
}

#[test]
fn section_and_elevation_guides_follow_levels_and_report_deleted_guides() {
    let mut f = fixture();
    let section = GuideDefinition {
        kind: GuideKind::Section {
            start: [length(-500.), length(2000.)],
            end: [length(5500.), length(2000.)],
            depth: Some(length(3000.)),
        },
        levels: vec![f.ground, f.upper],
        sources: f.walls.clone(),
        scale: Scale {
            numerator: 1,
            denominator: 100,
        },
    };
    let p = prepare_guide(
        &f.s,
        f.s.revision(),
        actor(),
        None,
        "Section A",
        &section,
        true,
        None,
    )
    .unwrap();
    let section_guide = p.guide;
    let book = p.generation.as_ref().unwrap().book;
    let section_view = p.generation.as_ref().unwrap().created[0];
    f.s.commit(p.prepared).unwrap();
    let view = views(&f.s, book)
        .into_iter()
        .find(|v| v.id == section_view)
        .unwrap();
    // Looking across the line toward +Y, limited to the line and the levels.
    assert_eq!(view.frame.u, [1., 0., 0.]);
    assert_eq!(view.frame.v, [0., 0., 1.]);
    assert_eq!(view.frame.normal(), [0., -1., 0.]);
    let ViewKind::Section { window, range, .. } = view.kind else {
        panic!()
    };
    assert_eq!(range, SectionRange::Depth { depth_mm: 3000. });
    assert_eq!(window.u_mm, Some([0., 6000.]));
    assert_eq!(window.v_mm, Some([-200., 6000.]));

    // An elevation into the same book: viewer looks toward +Y.
    let elevation = GuideDefinition {
        kind: GuideKind::Elevation { direction_deg: 90. },
        levels: vec![],
        sources: f.walls.clone(),
        scale: Scale {
            numerator: 1,
            denominator: 100,
        },
    };
    let p = prepare_guide(
        &f.s,
        f.s.revision(),
        actor(),
        None,
        "South elevation",
        &elevation,
        true,
        Some(book),
    )
    .unwrap();
    let elevation_view = p.generation.as_ref().unwrap().created[0];
    f.s.commit(p.prepared).unwrap();
    let ev = views(&f.s, book)
        .into_iter()
        .find(|v| v.id == elevation_view)
        .unwrap();
    assert_eq!(ev.kind, ViewKind::Projection);
    let n = ev.frame.normal();
    near(n[0], 0., 1e-12);
    near(n[1], -1., 1e-12);
    let m = compile(&f.s);
    let evaluation = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    for id in [section_view, elevation_view] {
        let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&id] else {
            panic!("{:?}", evaluation.views[&id])
        };
        assert!(!vectors.is_empty());
    }
    let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&section_view] else {
        panic!()
    };
    assert!(vectors.iter().any(|v| v.role == VectorRole::Hatch));
    // The section frame stands on the line from the first floor's slab
    // bottom to the top of the upper level.
    let outline = &m.snapshot().guide_outlines[&section_guide][0];
    assert_eq!(outline[0], [-500., 2000., -200.]);
    assert_eq!(outline[2], [5500., 2000., 6000.]);

    // Raising the upper level widens the section window.
    let p = model::prepare_level_edit(
        &f.s,
        f.s.revision(),
        actor(),
        f.upper,
        &LevelDefinition {
            elevation: length(3300.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    f.s.commit(p).unwrap();
    let ViewKind::Section { window, .. } = resolve_guided(f.s.document(), f.s.parameters(), &view)
        .unwrap()
        .kind
    else {
        panic!()
    };
    assert_eq!(window.v_mm, Some([-200., 6300.]));

    // Invalid guides are refused with a reason.
    let mut bad = section.clone();
    bad.kind = GuideKind::Section {
        start: [length(0.), length(0.)],
        end: [length(0.), length(0.)],
        depth: None,
    };
    let message = prepare_guide(&f.s, f.s.revision(), actor(), None, "Bad", &bad, true, None)
        .err()
        .unwrap()
        .to_string();
    assert!(message.contains("two distinct points"), "{message}");
    let mut no_levels = plan(&f, 1000.);
    no_levels.levels.clear();
    assert!(
        prepare_guide(
            &f.s,
            f.s.revision(),
            actor(),
            None,
            "Bad",
            &no_levels,
            false,
            None
        )
        .is_err()
    );

    // Deleting the elevation guide leaves its view reported, not silently
    // drawn from a stale frame.
    let p = f.s.prepare(multicad_core::commands::Request {
        expected_revision: f.s.revision(),
        actor: actor(),
        label: "Delete guide".into(),
        edits: vec![Mutation::Remove(
            multicad_core::drawings::guides(f.s.document())
                .find(|g| g.name == "South elevation")
                .unwrap()
                .id,
        )],
    });
    f.s.commit(p.unwrap()).unwrap();
    let evaluation = compile(&f.s)
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    let ViewEvaluation::Unresolved { message } = &evaluation.views[&elevation_view] else {
        panic!()
    };
    assert!(message.contains("deleted"), "{message}");
}
