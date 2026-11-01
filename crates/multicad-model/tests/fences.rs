use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::Resolver,
    model::*,
    project_file,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, EvaluationCache, Model, ModelError};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64, t: f64) {
    assert!((a - b).abs() <= t, "{a} != {b}");
}
fn failure(result: Result<Model, ModelError>) -> ModelError {
    result.err().expect("Expected native refusal")
}
fn fixture(
    points: &[[f64; 2]],
    closed: bool,
) -> (Session, FenceTypeRequest, FenceRunRequest, EntityId) {
    let material = |name: &str, color| {
        Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: color,
                section_hatch: None,
            },
        )
    };
    let wood = material("Timber", [160, 110, 70, 255]);
    let metal = material("Metal", [100, 110, 120, 255]);
    let doc = Document::new("Native fence")
        .unwrap()
        .edited(&[
            Mutation::Insert(wood.clone()),
            Mutation::Insert(metal.clone()),
        ])
        .unwrap();
    let ty = FenceTypeRequest::new(
        "Construction",
        FenceConstruction {
            height: length(1800.),
            post: FencePostProfile::Rectangular {
                width: length(90.),
                depth: length(90.),
            },
            post_material: wood.id,
            preferred_post_spacing: length(2000.),
            layout: FenceBayLayout::ShortLast,
            infills: vec![FenceInfill {
                id: EntityId::new(),
                name: "Panel".into(),
                material: metal.id,
                bottom: length(100.),
                height: length(1600.),
                normal_offset: length(0.),
                construction: FenceInfillKind::Panel {
                    thickness: length(18.),
                    width: FencePanelWidth::FitBay,
                },
            }],
        },
    );
    let path = FencePath {
        vertices: points
            .iter()
            .map(|p| FenceVertex {
                id: EntityId::new(),
                point: p.map(length),
            })
            .collect(),
        closed,
        base_z: length(0.),
    };
    let run = FenceRunRequest::new("Boundary", FenceRunDefinition::new(ty.id, path));
    (Session::new(doc).unwrap(), ty, run, metal.id)
}
fn apply(s: &mut Session, ty: &FenceTypeRequest, run: &FenceRunRequest) {
    let p = prepare_fence_change(
        s,
        s.revision(),
        actor(),
        &FenceChange {
            new_type: Some(ty.clone()),
            run: Some(FenceRunChange::Create(run.clone())),
            ..Default::default()
        },
    )
    .unwrap();
    s.commit(p.model.prepared).unwrap();
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn volume(s: &Session, m: &Model, run: EntityId) -> f64 {
    fence_members(s.document(), run)
        .unwrap()
        .into_iter()
        .map(|(_, f, b)| {
            let solid = m.solid(b).unwrap();
            assert_eq!(solid.solid_count().unwrap(), 1);
            let snapshot = m.snapshot();
            let topology = &snapshot.shapes[&f].topology;
            assert!(
                topology
                    .elements
                    .iter()
                    .all(|e| e.origins.iter().any(|o| o.source == f))
            );
            assert!(
                topology
                    .elements
                    .iter()
                    .any(|e| e.origins.iter().any(|o| o.role.starts_with("fence.")))
            );
            solid.volume_mm3().unwrap()
        })
        .sum()
}

#[test]
fn panel_run_has_separate_material_parts_true_dimensions_and_independent_step_readback() {
    let (mut s, ty, run, metal) = fixture(&[[0., 0.], [4500., 0.]], false);
    apply(&mut s, &ty, &run);
    let m = compile(&s);
    assert_eq!(m.snapshot().bodies.len(), 7);
    near(volume(&s, &m, run.id), 180_144_000., 0.01);
    let resolver = Resolver::new(s.document()).unwrap();
    let mut solids = Vec::new();
    for (key, _, body) in fence_members(s.document(), run.id).unwrap() {
        let solid = m.solid(body).unwrap();
        let b = solid.bounds().unwrap();
        match key {
            FenceMemberKey::Post { .. } => {
                near(b.max[0] - b.min[0], 90., 1e-4);
                near(b.max[1] - b.min[1], 90., 1e-4);
                near(b.max[2] - b.min[2], 1800., 1e-4);
                assert_eq!(
                    resolver.resolve(body).unwrap().material,
                    Some(ty.construction.post_material)
                );
            }
            FenceMemberKey::Infill { .. } => {
                near(b.max[1] - b.min[1], 18., 1e-4);
                near(b.min[2], 100., 1e-4);
                near(b.max[2], 1700., 1e-4);
                assert_eq!(resolver.resolve(body).unwrap().material, Some(metal));
            }
        }
        solids.push(solid);
    }
    let compound =
        Solid::compound_named(&solids, EntityId::new(), GeometryPolicy::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fence.step");
    compound.write_step(&path).unwrap();
    let back = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert_eq!(back.solid_count().unwrap(), 7);
    near(back.volume_mm3().unwrap(), 180_144_000., 0.01);
    let project = project_file::ProjectFile::new(s.document().clone()).unwrap();
    let mut bytes = Vec::new();
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let opened = Session::new(opened.project.document).unwrap();
    near(
        volume(&opened, &compile(&opened), run.id),
        180_144_000.,
        0.01,
    );
}

#[test]
fn l_closed_and_rotated_runs_keep_native_member_counts_and_do_not_overlap() {
    for (points, closed, count) in [
        (vec![[0., 0.], [4000., 0.], [4000., 3000.]], false, 9),
        (
            vec![[0., 0.], [4000., 0.], [4000., 3000.], [0., 3000.]],
            true,
            16,
        ),
    ] {
        let (mut s, ty, mut run, _) = fixture(&points, closed);
        apply(&mut s, &ty, &run);
        let m = compile(&s);
        let v = volume(&s, &m, run.id);
        assert_eq!(m.snapshot().bodies.len(), count);
        let (sin, cos) = 30_f64.to_radians().sin_cos();
        for (vertex, p) in run.definition.path.vertices.iter_mut().zip(points) {
            vertex.point = [
                length(p[0] * cos - p[1] * sin + 300.),
                length(p[0] * sin + p[1] * cos - 600.),
            ];
        }
        let p = prepare_fence_change(
            &s,
            s.revision(),
            actor(),
            &FenceChange {
                run: Some(FenceRunChange::Edit(run.clone())),
                ..Default::default()
            },
        )
        .unwrap();
        s.commit(p.model.prepared).unwrap();
        let m = compile(&s);
        assert_eq!(m.snapshot().bodies.len(), count);
        near(volume(&s, &m, run.id), v, 0.1);
    }
}

#[test]
fn open_pickets_and_touching_rails_remain_individual_solids_with_real_clear_gaps() {
    let (mut s, mut ty, run, _) = fixture(&[[0., 0.], [2000., 0.]], false);
    ty.construction.infills[0].construction = FenceInfillKind::Pickets {
        width: length(70.),
        depth: length(20.),
        maximum_gap: length(100.),
    };
    for bottom in [200., 1400.] {
        let mut rail = ty.construction.infills[0].clone();
        rail.id = EntityId::new();
        rail.name = "Rail".into();
        rail.bottom = length(bottom);
        rail.height = length(60.);
        rail.normal_offset = length(20.);
        rail.construction = FenceInfillKind::Rail { depth: length(20.) };
        ty.construction.infills.push(rail);
    }
    apply(&mut s, &ty, &run);
    let m = compile(&s);
    assert_eq!(m.snapshot().bodies.len(), 15);
    near(volume(&s, &m, run.id), 58_384_000., 0.01);
    let recipe = ty.construction.infills[0].id;
    let mut pickets = fence_members(s.document(), run.id)
        .unwrap()
        .into_iter()
        .filter(
            |(key, ..)| matches!(key, FenceMemberKey::Infill { infill } if infill.recipe == recipe),
        )
        .map(|(_, _, body)| m.solid(body).unwrap().bounds().unwrap())
        .collect::<Vec<_>>();
    pickets.sort_by(|a, b| a.min[0].total_cmp(&b.min[0]));
    assert_eq!(pickets.len(), 11);
    for b in &pickets {
        near(b.max[0] - b.min[0], 70., 1e-4);
    }
    for pair in pickets.windows(2) {
        near(pair[1].min[0] - pair[0].max[0], 95., 1e-4);
    }
}

#[test]
fn round_posts_and_explicit_short_panel_cuts_have_measured_geometry() {
    let (mut s, mut ty, run, _) = fixture(&[[0., 0.], [2000., 0.], [3000., 1000.]], false);
    ty.construction.post = FencePostProfile::Round {
        diameter: length(90.),
    };
    apply(&mut s, &ty, &run);
    let m = compile(&s);
    assert_eq!(m.snapshot().bodies.len(), 5);
    for (k, _, b) in fence_members(s.document(), run.id).unwrap() {
        if matches!(k, FenceMemberKey::Post { .. }) {
            let solid = m.solid(b).unwrap();
            near(
                solid.volume_mm3().unwrap(),
                std::f64::consts::PI * 45_f64.powi(2) * 1800.,
                0.01,
            );
            let b = solid.bounds().unwrap();
            near(b.max[0] - b.min[0], 90., 1e-4);
            near(b.max[1] - b.min[1], 90., 1e-4);
        }
    }
    let (mut s, mut ty, run, _) = fixture(&[[0., 0.], [4500., 0.]], false);
    ty.construction.infills[0].construction = FenceInfillKind::Panel {
        thickness: length(18.),
        width: FencePanelWidth::Purchased {
            width: length(1910.),
            short_bay: FenceShortPanelPolicy::Cut,
        },
    };
    apply(&mut s, &ty, &run);
    let m = compile(&s);
    let mut widths = Vec::new();
    for (k, _, b) in fence_members(s.document(), run.id).unwrap() {
        if matches!(k, FenceMemberKey::Infill { .. }) {
            let b = m.solid(b).unwrap().bounds().unwrap();
            widths.push(b.max[0] - b.min[0]);
        }
    }
    widths.sort_by(f64::total_cmp);
    for (a, b) in widths.into_iter().zip([410., 1910., 1910.]) {
        near(a, b, 1e-4);
    }
}

#[test]
fn whole_run_overlap_refusal_also_guards_a_single_scoped_member_and_the_cache() {
    let (mut s, mut ty, run, _) = fixture(&[[0., 0.], [2000., 0.]], false);
    let mut duplicate = ty.construction.infills[0].clone();
    duplicate.id = EntityId::new();
    duplicate.name = "Overlapping panel".into();
    ty.construction.infills.push(duplicate);
    apply(&mut s, &ty, &run);
    let feature = fence_members(s.document(), run.id).unwrap()[0].1;
    let mut cache = EvaluationCache::default();
    let e = failure(Compiler::default().compile_sources_cached(
        &mut cache,
        s.document(),
        s.revision(),
        &[feature],
        None,
    ));
    assert!(e.message.contains("overlap"), "{e:?}");
    assert!(cache.is_empty());
    let (mut s, ty, run, _) = fixture(
        &[
            [0., 0.],
            [4000., 0.],
            [4000., 2000.],
            [40., 2000.],
            [40., 40.],
        ],
        false,
    );
    apply(&mut s, &ty, &run);
    let e = failure(Compiler::default().compile(s.document(), s.revision()));
    assert!(e.message.contains("overlap"), "{e:?}");
}

#[test]
fn warm_cache_checks_current_members_materials_and_changed_construction() {
    let (mut s, ty, run, metal) = fixture(&[[0., 0.], [4500., 0.]], false);
    apply(&mut s, &ty, &run);
    let compiler = Compiler::default();
    let mut cache = EvaluationCache::default();
    compiler
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    compiler
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert_eq!(cache.last_compile().reused.len(), 7);
    assert!(cache.last_compile().evaluated.is_empty());
    let (_, f, b) = fence_members(s.document(), run.id).unwrap()[0];
    let mut hierarchy = s.document().hierarchy().clone();
    for children in hierarchy.members.values_mut() {
        children.retain(|id| *id != f && *id != b);
    }
    let missing = s
        .document()
        .edited(&[
            Mutation::Remove(f),
            Mutation::Remove(b),
            Mutation::SetHierarchy(hierarchy),
        ])
        .unwrap();
    let e = failure(compiler.compile_cached(&mut cache, &missing, s.revision(), None));
    assert!(e.message.contains("incomplete"));
    assert!(cache.last_compile().reused.is_empty());
    let post = fence_members(s.document(), run.id)
        .unwrap()
        .into_iter()
        .find(|(k, ..)| matches!(k, FenceMemberKey::Post { .. }))
        .unwrap()
        .2;
    let mut wrong = s.document().get(post).unwrap().clone();
    if let EntityData::Part { material, .. } = &mut wrong.data {
        *material = Some(metal);
    }
    let wrong = s.document().edited(&[Mutation::Replace(wrong)]).unwrap();
    let e = failure(compiler.compile_cached(&mut cache, &wrong, s.revision(), None));
    assert!(e.message.contains("material"));
    assert!(cache.last_compile().reused.is_empty());
    let mut edit = ty;
    edit.construction.height = length(2000.);
    let p = prepare_fence_change(
        &s,
        s.revision(),
        actor(),
        &FenceChange {
            edit_type: Some(edit),
            ..Default::default()
        },
    )
    .unwrap();
    s.commit(p.model.prepared).unwrap();
    let m = compiler
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert_eq!(cache.last_compile().evaluated.len(), 7);
    assert!(cache.last_compile().reused.is_empty());
    near(volume(&s, &m, run.id), 186_624_000., 0.01);
}

#[test]
fn unsupported_thin_members_do_not_inflate_geometry_or_publish_partial_cache() {
    let (mut s, mut ty, run, _) = fixture(&[[0., 0.], [2000., 0.]], false);
    if let FenceInfillKind::Panel { thickness, .. } = &mut ty.construction.infills[0].construction {
        *thickness = length(1e-6);
    }
    apply(&mut s, &ty, &run);
    let before = s.document().clone();
    let mut cache = EvaluationCache::default();
    let e =
        failure(Compiler::default().compile_cached(&mut cache, s.document(), s.revision(), None));
    assert!(e.message.contains("below the native minimum"), "{e:?}");
    assert!(cache.is_empty());
    assert_eq!(s.document(), &before);
}

#[test]
fn cached_geometry_tracks_bound_parameters_and_floor_relative_level_context() {
    use multicad_core::{constraint_sketch::Dimension, parameters::BoundExpression};
    let (mut s, mut ty, mut run, _) = fixture(&[[0., 0.], [4500., 0.]], false);
    let parameter = Entity::new(
        "post_height",
        EntityData::Parameter {
            expression: "1.8".into(),
            unit: "m".into(),
            exposed: true,
            inputs: Vec::new(),
        },
    );
    let request = multicad_core::commands::Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fence height parameter".into(),
        edits: vec![Mutation::Insert(parameter.clone())],
    };
    s.commit(s.prepare(request).unwrap()).unwrap();
    ty.construction.height = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$post_height", "mm", None).unwrap(),
    };
    let mut level = LevelDefinition {
        elevation: length(3000.),
        height: length(3000.),
        slab_thickness: length(200.),
    };
    let (level_id, prepared) =
        prepare_level(&s, s.revision(), actor(), "Upper floor", &level).unwrap();
    s.commit(prepared).unwrap();
    run.definition.level = Some(level_id);
    run.definition.path.base_z = length(100.);
    apply(&mut s, &ty, &run);
    let compiler = Compiler::default();
    let mut cache = EvaluationCache::default();
    compiler
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    let mut renamed = parameter;
    renamed.name = "retained_height".into();
    if let EntityData::Parameter { expression, .. } = &mut renamed.data {
        *expression = "2".into();
    }
    let reused = Entity::new(
        "post_height",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "m".into(),
            exposed: true,
            inputs: Vec::new(),
        },
    );
    let request = multicad_core::commands::Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Rename and change the retained height".into(),
        edits: vec![Mutation::Replace(renamed), Mutation::Insert(reused)],
    };
    s.commit(s.prepare(request).unwrap()).unwrap();
    let model = compiler
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(cache.last_compile().reused.is_empty());
    assert_eq!(cache.last_compile().evaluated.len(), 7);
    near(volume(&s, &model, run.id), 186_624_000., 0.01);
    level.elevation = length(4000.);
    let prepared = prepare_level_edit(&s, s.revision(), actor(), level_id, &level).unwrap();
    s.commit(prepared).unwrap();
    let model = compiler
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(cache.last_compile().reused.is_empty());
    assert_eq!(cache.last_compile().evaluated.len(), 7);
    for (key, _, body) in fence_members(s.document(), run.id).unwrap() {
        if matches!(key, FenceMemberKey::Post { .. }) {
            let bounds = model.solid(body).unwrap().bounds().unwrap();
            near(bounds.min[2], 4100., 1e-4);
            near(bounds.max[2], 6100., 1e-4);
        }
    }
    assert_eq!(
        read_fence_run(s.document(), run.id).unwrap().path.base_z,
        length(100.)
    );
}
#[test]
fn linked_plan_elevation_pdf_and_dxf_measure_actual_members_at_three_scales() {
    use multicad_core::{drawings::*, references::ReferenceFrame};
    let (mut s, ty, run, _) = fixture(&[[0., 0.], [4500., 0.]], false);
    apply(&mut s, &ty, &run);
    let parts = fence_members(s.document(), run.id)
        .unwrap()
        .into_iter()
        .map(|(_, _, part)| part)
        .collect::<Vec<_>>();
    let native = compile(&s);
    let face = |vertex, normal: [f64; 3]| {
        use multicad_core::{
            inspection::GeometryReference,
            references::{AnalyticGeometry, GeometryIntent, TopologyKind},
        };
        let part=fence_members(s.document(),run.id).unwrap().into_iter().find(|(key,_,_)|matches!(key,FenceMemberKey::Post{post:FencePostKey::Vertex{vertex:id}} if *id==vertex)).unwrap().2;
        let feature = native.snapshot().bodies[&part];
        let mut topology = native.snapshot().shapes[&feature].topology.clone();
        topology.owner = part;
        let element=topology.elements.iter().find(|e|e.key.kind==TopologyKind::Face && matches!(e.geometry,Some(AnalyticGeometry::Plane{frame}) if frame.normal().iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-9))).unwrap();
        GeometryReference {
            source: part,
            intent: GeometryIntent::capture(&topology, element.key).unwrap(),
        }
    };
    let left = face(run.definition.path.vertices[0].id, [-1., 0., 0.]);
    let right = face(run.definition.path.vertices[1].id, [1., 0., 0.]);
    let mut dimensions = Vec::new();
    let mut books = Vec::new();
    for denominator in [100, 50, 10] {
        let mut page = Page::new(format!("Fence 1:{denominator}"));
        page.settings.paper = PaperSize::Custom {
            width_mm: 600.,
            height_mm: 500.,
        };
        for (name, v, kind, position) in [
            (
                "Plan at 1000 mm",
                [0., 1., 0.],
                ViewKind::Section {
                    offset_mm: 1000.,
                    range: SectionRange::SectionOnly,
                    window: SectionWindow::default(),
                },
                [60., 100.],
            ),
            ("Elevation", [0., 0., 1.], ViewKind::Projection, [60., 400.]),
        ] {
            page.views.push(DrawingView {
                id: EntityId::new(),
                name: name.into(),
                source: ViewSource::Model {
                    entities: parts.clone(),
                },
                frame: ReferenceFrame {
                    origin_mm: [0.; 3],
                    u: [1., 0., 0.],
                    v,
                },
                kind,
                position_mm: position,
                scale: Scale {
                    numerator: 1,
                    denominator,
                },
                hidden_lines: false,
                tangent_edges: false,
                paper_chord_tolerance_mm: 0.01,
                guide: None,
                equipment_categories: None,
            });
        }
        let dimension = Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Dimension {
                view: page.views[1].id,
                measure: Box::new(DimensionMeasure::Linear {
                    a: left.clone(),
                    b: right.clone(),
                    direction: LinearDirection::Horizontal,
                }),
                offset_mm: -15.,
                precision: 0,
                reference: false,
            },
        };
        dimensions.push(dimension.id);
        page.annotations.push(dimension);
        let (book, prepared) =
            prepare_create(&s, s.revision(), actor(), "Fence drawings", Some(page)).unwrap();
        s.commit(prepared).unwrap();
        books.push((book, denominator as f64));
    }
    let model = compile(&s);
    let bom = model
        .bom(
            s.document(),
            &multicad_model::schedules::BomOptions::default(),
        )
        .unwrap();
    assert_eq!(bom.rows.len(), 7);
    assert!(
        bom.rows
            .iter()
            .all(|r| r.quantity == 1 && r.material.is_some())
    );
    assert_eq!(
        bom.rows
            .iter()
            .filter(|r| r
                .material
                .as_ref()
                .is_some_and(|(_, name)| name == "Timber"))
            .count(),
        4
    );
    assert_eq!(
        bom.rows
            .iter()
            .filter(|r| r.material.as_ref().is_some_and(|(_, name)| name == "Metal"))
            .count(),
        3
    );
    let evaluation = model
        .drawings(s.document(), s.revision(), &Default::default())
        .unwrap();
    for dimension in &dimensions {
        let AnnotationEvaluation::Ready { graphic } = &evaluation.annotations[dimension] else {
            panic!("{:?}", evaluation.annotations[dimension])
        };
        near(graphic.value, 4590., 1e-6);
        assert_eq!(graphic.text, "4590");
    }
    for (book, denominator) in books {
        let value = multicad_core::drawings::read(s.document(), book).unwrap();
        let page = &value.pages[0];
        for view in &page.views {
            assert!(
                matches!(&evaluation.views[&view.id], ViewEvaluation::Ready { .. }),
                "{:?}",
                evaluation.views[&view.id]
            );
        }
        let render =
            render::render_page(s.document(), s.revision(), book, page.id, &evaluation).unwrap();
        let pdf =
            pdf::book_pdf(s.document(), s.revision(), &[(book, page.id)], &evaluation).unwrap();
        let dxf = dxf::render_dxf(&render, 1., 4);
        let drawing = ::dxf::Drawing::load(&mut std::io::Cursor::new(dxf.as_bytes())).unwrap();
        let lines = drawing
            .entities()
            .filter_map(|e| {
                if matches!(e.common.layer.as_str(), "VIEW_VISIBLE" | "VIEW_SECTION")
                    && let ::dxf::entities::EntityType::Line(line) = &e.specific
                {
                    Some(([line.p1.x, line.p1.y], [line.p2.x, line.p2.y]))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let text = String::from_utf8_lossy(&pdf);
        let begin = text.find("stream\n").unwrap() + 7;
        let stream = &text[begin..];
        let stream = &stream[..stream.find("endstream").unwrap()];
        let paths = stream
            .lines()
            .filter(|l| l.ends_with(" S"))
            .map(|line| {
                let tokens = line.split_whitespace().collect::<Vec<_>>();
                tokens
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| {
                        if *t == "m" || *t == "l" {
                            Some([
                                tokens[i - 2].parse::<f64>().unwrap() / pdf::PT_PER_MM,
                                tokens[i - 1].parse::<f64>().unwrap() / pdf::PT_PER_MM,
                            ])
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let has_line = |a: [f64; 2], b: [f64; 2]| {
            let same = |p: [f64; 2], q: [f64; 2]| {
                (p[0] - q[0]).abs() < 0.002 && (p[1] - q[1]).abs() < 0.002
            };
            assert!(
                lines
                    .iter()
                    .any(|(p, q)| (same(*p, a) && same(*q, b)) || (same(*p, b) && same(*q, a))),
                "DXF missing {a:?} to {b:?} at 1:{denominator}"
            );
            assert!(
                paths.iter().any(|path| path
                    .windows(2)
                    .any(|p| (same(p[0], a) && same(p[1], b)) || (same(p[0], b) && same(p[1], a)))),
                "PDF missing {a:?} to {b:?} at 1:{denominator}"
            );
        };
        // Four real 90 mm posts, not one opaque wall, in plan and elevation.
        for center in [0., 2000., 4000., 4500.] {
            has_line(
                [60. + (center - 45.) / denominator, 400. - 45. / denominator],
                [60. + (center + 45.) / denominator, 400. - 45. / denominator],
            );
            has_line(
                [60. + (center - 45.) / denominator, 100.],
                [
                    60. + (center - 45.) / denominator,
                    100. + 1800. / denominator,
                ],
            );
        }
        // Actual full and short-last clear panel widths, and physical elevation.
        for (left, right) in [(45., 1955.), (2045., 3955.), (4045., 4455.)] {
            has_line(
                [60. + left / denominator, 400. - 9. / denominator],
                [60. + right / denominator, 400. - 9. / denominator],
            );
            has_line(
                [60. + left / denominator, 100. + 1700. / denominator],
                [60. + right / denominator, 100. + 1700. / denominator],
            );
        }
        if let Ok(directory) = std::env::var("MULTICAD_FENCE_REVIEW") {
            let directory = std::path::Path::new(&directory);
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join(format!("fence-1-{denominator}.pdf")), &pdf).unwrap();
            std::fs::write(directory.join(format!("fence-1-{denominator}.dxf")), dxf).unwrap();
            std::fs::write(
                directory.join(format!("fence-1-{denominator}.svg")),
                svg::render_svg(&render),
            )
            .unwrap();
            let file = project_file::ProjectFile::new(s.document().clone()).unwrap();
            project_file::write(
                &file,
                &mut std::fs::File::create(directory.join("fence.tcd")).unwrap(),
                Default::default(),
            )
            .unwrap();
        }
    }
    // A retained path edit updates these linked views through the same Part IDs.
    let before = parts;
    let mut changed = run.clone();
    changed.definition.path.vertices[1].point[0] = length(4800.);
    let prepared = prepare_fence_change(
        &s,
        s.revision(),
        actor(),
        &FenceChange {
            run: Some(FenceRunChange::Edit(changed)),
            ..Default::default()
        },
    )
    .unwrap();
    s.commit(prepared.model.prepared).unwrap();
    assert_eq!(
        fence_members(s.document(), run.id)
            .unwrap()
            .into_iter()
            .map(|(_, _, part)| part)
            .collect::<Vec<_>>(),
        before
    );
    let model = compile(&s);
    let evaluation = model
        .drawings(s.document(), s.revision(), &Default::default())
        .unwrap();
    for dimension in dimensions {
        let AnnotationEvaluation::Ready { graphic } = &evaluation.annotations[&dimension] else {
            panic!("{:?}", evaluation.annotations[&dimension])
        };
        near(graphic.value, 4890., 1e-6);
        assert_eq!(graphic.text, "4890");
    }
    assert!(
        evaluation
            .views
            .values()
            .all(|v| matches!(v, ViewEvaluation::Ready { .. }))
    );
}
