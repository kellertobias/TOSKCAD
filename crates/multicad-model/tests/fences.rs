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
