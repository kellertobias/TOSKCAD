use super::common::*;
use multicad_core::{
    model::{self, *},
    references::{self, Origin, Resolution},
};

#[test]
fn shell_signed_walls_follow_parameters_and_linked_occurrences_with_native_failure_neutrality() {
    let (mut session, input) = fixture();
    let opening = face(&session, &input, [0., 0., 1.]);
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Shell(ShellParameters {
            thickness: bound(&session, "wall", "mm"),
        }),
        vec![opening.clone()],
    );
    let native = commit(&mut session, &request);
    near(
        native.solid(input.body).unwrap().volume_mm3().unwrap(),
        6000. - 26. * 16. * 8.,
    );
    for occurrence in native.snapshot().occurrences.keys() {
        near(
            native.solid(*occurrence).unwrap().volume_mm3().unwrap(),
            2672.,
        );
    }
    apply_parameter(&mut session, "width", "40");
    let native = compile(&session);
    near(
        native.solid(input.body).unwrap().volume_mm3().unwrap(),
        8000. - 36. * 16. * 8.,
    );
    assert!(native.snapshot().references.all_resolved());
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Shell(p) = &mut edit.parameters else {
        panic!()
    };
    p.thickness = length(-2.);
    let candidate =
        model::prepare_advanced_edit(&session, session.revision(), actor(), &edit).unwrap();
    let outward = multicad_model::Compiler::default()
        .compile(candidate.prepared.document(), session.revision())
        .unwrap();
    assert!(outward.solid(input.body).unwrap().bounds().unwrap().min[0] < -1.99);
    let volume = outward.solid(input.body).unwrap().volume_mm3().unwrap();
    session.commit(candidate.prepared).unwrap();
    persist_and_check(&mut session, input.body, volume, 1);
    let before = session.document().clone();
    let revision = session.revision();
    let AdvancedParameters::Shell(p) = &mut edit.parameters else {
        panic!()
    };
    p.thickness = length(1000.);
    let candidate = model::prepare_advanced_edit(&session, revision, actor(), &edit).unwrap();
    assert!(
        multicad_model::Compiler::default()
            .compile(candidate.prepared.document(), revision)
            .is_err()
    );
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    assert_eq!(
        model::read_advanced(session.document(), request.feature)
            .unwrap()
            .1[0]
            .id,
        opening.id
    );
}

#[test]
fn draft_and_mirror_rebuild_current_geometry_and_saved_sources() {
    let (mut session, input) = fixture();
    let side = face(&session, &input, [1., 0., 0.]);
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Draft(DraftParameters {
            angle: bound(&session, "draft", "deg"),
            ..Default::default()
        }),
        vec![side],
    );
    commit(&mut session, &request);
    let volume = 6000. - 20. * 0.5 * 100. * 5f64.to_radians().tan();
    near(
        compile(&session)
            .solid(input.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        volume,
    );
    apply_parameter(&mut session, "draft", "-5");
    let volume = 6000. + 20. * 0.5 * 100. * 5f64.to_radians().tan();
    persist_and_check(&mut session, input.body, volume, 1);
    let (mut session, input) = fixture();
    let mut frame = PrimitiveFrame::default();
    frame.origin[2] = bound(&session, "mirror_z", "mm");
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Mirror(MirrorParameters { plane: frame }),
        vec![],
    );
    let native = commit(&mut session, &request);
    near(
        native.solid(input.body).unwrap().bounds().unwrap().min[2],
        -10.,
    );
    apply_parameter(&mut session, "mirror_z", "2");
    let native = compile(&session);
    let bounds = native.solid(input.body).unwrap().bounds().unwrap();
    near(bounds.min[2], -6.);
    near(bounds.max[2], 4.);
    apply_parameter(&mut session, "height", "15");
    near(
        compile(&session)
            .solid(input.body)
            .unwrap()
            .bounds()
            .unwrap()
            .min[2],
        -11.,
    );
    persist_and_check(&mut session, input.body, 9000., 1);
}

#[test]
fn compound_patterns_preserve_member_references_count_identity_and_explicit_merge() {
    let (mut session, input) = fixture();
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::LinearPattern(LinearPatternParameters {
            count: bound(&session, "copies", "count"),
            step: [bound(&session, "pitch", "mm"), length(0.), length(0.)],
            merge: false,
        }),
        vec![],
    );
    let native = commit(&mut session, &request);
    let snapshot = &native.snapshot().shapes[&request.feature].topology;
    let member = snapshot
        .elements
        .iter()
        .find(|e| {
            e.origins.contains(&Origin {
                source: request.feature,
                role: "member/1".into(),
            }) && e.origins.contains(&Origin {
                source: input.producer,
                role: "cap.end".into(),
            })
        })
        .unwrap();
    let capture = references::GeometryIntent::capture(snapshot, member.key).unwrap();
    near(
        native.solid(input.body).unwrap().volume_mm3().unwrap(),
        12000.,
    );
    assert_eq!(native.solid(input.body).unwrap().solid_count().unwrap(), 2);
    apply_parameter(&mut session, "copies", "3");
    apply_parameter(&mut session, "pitch", "50");
    let native = compile(&session);
    assert!(matches!(
        references::resolve(
            &capture,
            &native.snapshot().shapes[&request.feature].topology,
            session.revision()
        ),
        Resolution::Resolved { .. }
    ));
    assert_eq!(
        model::read_advanced(session.document(), request.feature)
            .unwrap()
            .0
            .output
            .body,
        input.body
    );
    persist_and_check(&mut session, input.body, 18000., 3);
    let mut reduced = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::LinearPattern(p) = &mut reduced.parameters else {
        panic!()
    };
    p.count = multicad_core::constraint_sketch::Dimension::Literal {
        value: 1.,
        unit: "count".into(),
    };
    let candidate =
        model::prepare_advanced_edit(&session, session.revision(), actor(), &reduced).unwrap();
    let reduced_native = multicad_model::Compiler::default()
        .compile(candidate.prepared.document(), session.revision())
        .unwrap();
    assert!(
        !matches!(
            references::resolve(
                &capture,
                &reduced_native.snapshot().shapes[&request.feature].topology,
                session.revision()
            ),
            Resolution::Resolved { .. }
        ),
        "A removed member must require repair rather than bind to a surviving member"
    );
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::LinearPattern(p) = &mut edit.parameters else {
        panic!()
    };
    p.count = multicad_core::constraint_sketch::Dimension::Literal {
        value: 2.,
        unit: "count".into(),
    };
    p.step[0] = length(20.);
    p.merge = true;
    let candidate =
        model::prepare_advanced_edit(&session, session.revision(), actor(), &edit).unwrap();
    session.commit(candidate.prepared).unwrap();
    persist_and_check(&mut session, input.body, 10000., 1);
    let (mut session, input) = fixture();
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::CircularPattern(CircularPatternParameters {
            count: multicad_core::constraint_sketch::Dimension::Literal {
                value: 4.,
                unit: "count".into(),
            },
            step_angle: bound(&session, "angle", "deg"),
            ..Default::default()
        }),
        vec![],
    );
    let native = commit(&mut session, &request);
    let bounds = native.solid(input.body).unwrap().bounds().unwrap();
    near(bounds.min[0], -30.);
    near(bounds.max[1], 30.);
    persist_and_check(&mut session, input.body, 24000., 4);
    apply_parameter(&mut session, "angle", "45");
    let native = compile(&session);
    assert_eq!(native.solid(input.body).unwrap().solid_count().unwrap(), 4);
    near(
        native.solid(input.body).unwrap().volume_mm3().unwrap(),
        24000.,
    );
}
