use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{
        self, GearFamily, GearHand, GearParameters, GearRequest, PrimitiveFrame, angle, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::Compiler;
#[test]
fn unchanged_gear_reuses_native_result_and_recipe_edits_invalidate_it() {
    let mut s = Session::new(Document::new("Cached gear").unwrap()).unwrap();
    let request = GearRequest::new(
        "Gear",
        GearParameters {
            face_width: length(4.),
            ..Default::default()
        },
    );
    let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let mut cache = multicad_model::EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(cache.last_compile().evaluated.contains(&request.feature));
    let again = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(cache.last_compile().reused.contains(&request.feature));
    assert!(cache.last_compile().evaluated.is_empty());
    assert_eq!(
        first.solid(request.feature).unwrap().volume_mm3().unwrap(),
        again.solid(request.feature).unwrap().volume_mm3().unwrap()
    );
    let mut params = request.parameters.clone();
    params.face_width = length(5.);
    let p = model::prepare_gear_edit(&s, 1, actor(), request.feature, &params).unwrap();
    s.commit(p.prepared).unwrap();
    let edited = compiler()
        .compile_cached(&mut cache, s.document(), 2, None)
        .unwrap();
    assert!(cache.last_compile().evaluated.contains(&request.feature));
    assert!(!cache.last_compile().reused.contains(&request.feature));
    assert!(
        (edited.solid(request.feature).unwrap().volume_mm3().unwrap()
            - first.solid(request.feature).unwrap().volume_mm3().unwrap() * 1.25)
            .abs()
            < 0.001
    );
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
#[test]
fn native_gear_candidate_cancel_edit_history_and_reopen_preserve_one_body() {
    let mut s = Session::new(Document::new("Native gears").unwrap()).unwrap();
    let request = GearRequest::new(
        "Spur gear",
        GearParameters {
            face_width: length(4.),
            ..Default::default()
        },
    );
    let before = s.document().clone();
    let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
    let candidate = compiler().compile(p.prepared.document(), 1).unwrap();
    let volume = candidate
        .solid(request.feature)
        .unwrap()
        .volume_mm3()
        .unwrap();
    assert!(volume > 6000.);
    drop(p);
    assert_eq!(s.document(), &before);
    let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let committed = s.document().clone();
    let parameters = GearParameters {
        face_width: length(5.),
        ..request.parameters.clone()
    };
    let p = model::prepare_gear_edit(&s, 1, actor(), request.feature, &parameters).unwrap();
    let candidate = compiler().compile(p.prepared.document(), 2).unwrap();
    assert!(
        (candidate
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - volume * 1.25)
            .abs()
            < 0.001
    );
    assert_eq!(candidate.snapshot().bodies[&request.body], request.feature);
    s.commit(p.prepared).unwrap();
    let edited = s.document().clone();
    s.undo(2, actor()).unwrap();
    assert_eq!(s.document(), &committed);
    s.redo(3, actor()).unwrap();
    assert_eq!(s.document(), &edited);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(edited).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let rebuilt = compiler().compile(&reopened.document, 0).unwrap();
    assert_eq!(rebuilt.snapshot().bodies[&request.body], request.feature);
    assert!(
        (rebuilt
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - volume * 1.25)
            .abs()
            < 0.001
    );
}
#[test]
fn every_supported_family_builds_in_its_canonical_frame_and_survives_step_readback() {
    for family in GearFamily::ALL.into_iter().filter(|f| f.supported()) {
        let mut s = Session::new(Document::new("Framed gear").unwrap()).unwrap();
        let internal = family == GearFamily::InternalRing;
        let helical = !matches!(family, GearFamily::Spur | GearFamily::InternalRing);
        let request = GearRequest::new(
            "Gear",
            GearParameters {
                family,
                teeth: multicad_core::constraint_sketch::Dimension::Literal {
                    value: if internal { 48. } else { 24. },
                    unit: "count".into(),
                },
                bore_diameter: length(if internal { 0. } else { 8. }),
                outer_diameter: internal.then(|| length(112.)),
                face_width: length(4.),
                helix_angle: angle(if helical { 15. } else { 0. }),
                hand: if helical {
                    GearHand::Left
                } else {
                    GearHand::None
                },
                center_gap: length(if family == GearFamily::DoubleHelical {
                    1.
                } else {
                    0.
                }),
                frame: PrimitiveFrame {
                    origin: [length(10.), length(20.), length(30.)],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
                ..Default::default()
            },
        );
        let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        let built = compiler().compile(s.document(), 1).unwrap();
        let solid = built.solid(request.feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let bounds = solid.bounds().unwrap();
        assert!((bounds.min[0] - 10.).abs() < 1e-6);
        assert!((bounds.max[0] - 14.).abs() < 1e-6);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gear.step");
        solid.write_step(&path).unwrap();
        let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(readback.solid_count().unwrap(), 1);
        assert!((readback.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 0.001);
    }
}
