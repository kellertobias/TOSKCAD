use super::common::*;
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, source::Source},
    document::{EntityData, EntityId, Mutation},
    model::{self, *},
    project_file::{self, ProjectFile},
    sketch::Datum,
    sketch_regions,
};
use std::f64::consts::PI;

fn add_sketch(
    session: &mut Session,
    datum: Datum,
    name: &str,
    z: f64,
    build: impl FnOnce(&mut Source, &Session),
) -> (EntityId, Source) {
    let (id, mut edits) = Source::create(datum, name).unwrap();
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit
            && let EntityData::Plane { origin_mm, .. } = &mut entity.data
        {
            origin_mm[2] = z;
        }
    }
    let doc = session.document().edited(&edits).unwrap();
    let mut source = Source::read(&doc, id).unwrap().0;
    build(&mut source, session);
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: name.into(),
            edits,
        })
        .unwrap();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: format!("Define {name}"),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    (id, source)
}
fn circle(
    session: &mut Session,
    name: &str,
    z: f64,
    r: f64,
    driven: bool,
) -> (ProfileSource, EntityId) {
    let mut region = None;
    let mut seam = EntityId::new();
    let (id, _) = add_sketch(session, Datum::Xy, name, z, |source, session| {
        let center = source.add_point([0., 0.]);
        seam = EntityId::new();
        source.sketch.curves.push(Curve {
            id: seam,
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: r,
            },
        });
        region = Some(sketch_regions::extract(source).regions[0].intent());
        source.add_constraint(ConstraintKind::Fix { point: center });
        if driven {
            source.add_constraint(ConstraintKind::Radius {
                curve: seam,
                dimension: bound(session, "wall", "mm"),
            });
        }
    });
    (
        ProfileSource {
            sketch: id,
            region: region.unwrap(),
        },
        seam,
    )
}
fn path(session: &mut Session) -> SketchPathSource {
    let mut line = EntityId::new();
    let (id, _) = add_sketch(session, Datum::Xz, "Ordered path", 0., |source, session| {
        let a = source.add_point([0., 0.]);
        let b = source.add_point([0., 40.]);
        line = source.add_line(a, b);
        source.add_constraint(ConstraintKind::Fix { point: a });
        source.add_constraint(ConstraintKind::Vertical { line });
        source.add_constraint(ConstraintKind::Length {
            line,
            dimension: bound(session, "pitch", "mm"),
        });
    });
    SketchPathSource {
        sketch: id,
        curves: vec![line],
    }
}
fn check_saved_rust_sweep(session: &mut Session, request: &AdvancedRequest, volume: f64) {
    use multicad_model::{Compiler, GeometryBackend};
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let evaluate = |document, revision| {
        compiler
            .compile_sources_with_backend(
                document,
                revision,
                &[request.body],
                GeometryBackend::Rust,
                None,
            )
            .unwrap()
    };
    let revision = session.revision();
    let before = session.document().clone();
    let model = evaluate(&before, revision);
    let solid = model.solid(request.body).unwrap();
    assert!(solid.uses_rust_geometry());
    near(solid.volume_mm3().unwrap(), volume);
    assert_eq!(solid.solid_count().unwrap(), 1);
    assert!(
        !solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    let folder = tempfile::tempdir().unwrap();
    let step = folder.path().join("swept.step");
    solid.write_step(&step).unwrap();
    let imported = multicad_kernel::Solid::import_rust_step(
        &std::fs::read_to_string(step).unwrap(),
        request.feature,
        Default::default(),
    )
    .unwrap();
    near(imported.volume_mm3().unwrap(), volume);

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(before.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, before);
    let restored = evaluate(&opened.project.document, revision);
    near(
        restored.solid(request.body).unwrap().volume_mm3().unwrap(),
        volume,
    );
    assert!(restored.snapshot().references.all_resolved());
    session.undo(revision, actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn sweep_rebuilds_solved_profile_and_ordered_path_without_display_geometry() {
    let (mut session, _) = fixture();
    let (profile, _) = circle(&mut session, "Sweep section", 0., 2., true);
    let path = path(&mut session);
    let request = AdvancedRequest::new_body(
        "Swept stock",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path),
    );
    commit(&mut session, &request);
    check_saved_rust_sweep(&mut session, &request, PI * 4. * 40.);
    apply_parameter(&mut session, "pitch", "50");
    apply_parameter(&mut session, "wall", "3");
    check_saved_rust_sweep(&mut session, &request, PI * 9. * 50.);
    let source = model::read_advanced(session.document(), request.feature)
        .unwrap()
        .0;
    assert_eq!(source.output.body, request.body);
    assert_eq!(source.profiles, request.profiles);
    assert_eq!(source.path, request.path);
}
#[test]
fn an_existing_sweep_rejects_a_path_changed_to_construction_without_source_mutation() {
    let (mut session, _) = fixture();
    let (profile, _) = circle(&mut session, "Sweep profile", 0., 2., false);
    let path = path(&mut session);
    let request = AdvancedRequest::new_body(
        "Swept stock",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path.clone()),
    );
    commit(&mut session, &request);
    let before = session.document().clone();
    let revision = session.revision();
    let EntityData::Sketch { definition, .. } = &before.get(path.sketch).unwrap().data else {
        panic!()
    };
    let mut source = Source::from_definition(definition).unwrap();
    source
        .sketch
        .curves
        .iter_mut()
        .find(|curve| curve.id == path.curves[0])
        .unwrap()
        .construction = true;
    let mut entity = before.get(path.sketch).unwrap().clone();
    let EntityData::Sketch { definition, .. } = &mut entity.data else {
        panic!()
    };
    *definition = source.definition().unwrap();
    let candidate = before.edited(&[Mutation::Replace(entity)]).unwrap();
    assert_eq!(
        model::read_advanced(&candidate, request.feature)
            .unwrap()
            .0
            .path,
        Some(path)
    );
    let failure = multicad_model::Compiler::default()
        .compile_sources_with_backend(
            &candidate,
            revision,
            &[request.body],
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(failure.entity, Some(request.feature));
    assert!(failure.message.contains("construction curve"));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    near(
        compile(&session)
            .solid(request.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        PI * 4. * 40.,
    );
}
#[test]
fn loft_uses_explicit_section_seams_and_current_bound_section_geometry() {
    let (mut session, _) = fixture();
    let (first, a) = circle(&mut session, "Loft start", 0., 2., true);
    let (second, b) = circle(&mut session, "Loft end", 20., 4., false);
    let request = AdvancedRequest::new_body(
        "Lofted stock",
        AdvancedParameters::Loft(LoftParameters {
            ruled: true,
            seams: vec![a, b],
        }),
        vec![first, second],
        None,
    );
    commit(&mut session, &request);
    persist_and_check(
        &mut session,
        request.body,
        PI * 20. / 3. * (4. + 8. + 16.),
        1,
    );
    apply_parameter(&mut session, "wall", "3");
    persist_and_check(
        &mut session,
        request.body,
        PI * 20. / 3. * (9. + 12. + 16.),
        1,
    );
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Loft(p) = &mut edit.parameters else {
        panic!()
    };
    p.ruled = false;
    let prepared =
        model::prepare_advanced_edit(&session, session.revision(), actor(), &edit).unwrap();
    session.commit(prepared.prepared).unwrap();
    persist_and_check(
        &mut session,
        request.body,
        PI * 20. / 3. * (9. + 12. + 16.),
        1,
    );
}
#[test]
fn missing_seam_and_unsupported_curved_path_fail_without_committing_source() {
    let (mut session, _) = fixture();
    let (first, _) = circle(&mut session, "First", 0., 2., false);
    let (second, b) = circle(&mut session, "Second", 20., 4., false);
    let request = AdvancedRequest::new_body(
        "Bad seam",
        AdvancedParameters::Loft(LoftParameters {
            ruled: true,
            seams: vec![EntityId::new(), b],
        }),
        vec![first.clone(), second],
        None,
    );
    let original = session.document().clone();
    let revision = session.revision();
    assert!(model::prepare_advanced(&session, revision, actor(), &request).is_err());
    assert_eq!(session.document(), &original);
    let path = SketchPathSource {
        sketch: first.sketch,
        curves: vec![
            Source::read(session.document(), first.sketch)
                .unwrap()
                .0
                .sketch
                .curves[0]
                .id,
        ],
    };
    let request = AdvancedRequest::new_body(
        "Unsupported path",
        AdvancedParameters::Sweep(Default::default()),
        vec![first],
        Some(path),
    );
    let candidate = model::prepare_advanced(&session, revision, actor(), &request).unwrap();
    let result =
        multicad_model::Compiler::default().compile(candidate.prepared.document(), revision);
    assert!(result.err().unwrap().message.contains("arcs and splines"));
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), revision);
}
