#[path = "../../multicad-app/examples/spline_trim_review.rs"]
mod review;
use multicad_core::{
    constraint_sketch::{ConstraintKind, CurveKind},
    document::{EntityData, Mutation},
    sketch_trim::{Endpoint, TrimErrorKind, TrimRequest},
};
use multicad_kernel::{GeometryPolicy, NativeRegionQueries};
use multicad_model::Compiler;
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn point_near(a: [f64; 2], b: [f64; 2]) {
    near(a[0], b[0]);
    near(a[1], b[1]);
}
#[test]
fn genuine_rational_and_polynomial_splines_trim_against_every_named_boundary_without_replacing_surviving_ids()
 {
    for rational in [false, true] {
        for boundary in [
            review::BoundaryKind::Line,
            review::BoundaryKind::Arc,
            review::BoundaryKind::Circle,
            review::BoundaryKind::Spline,
        ] {
            let mut f = review::seed(rational, boundary).unwrap();
            let original = f.session.document().clone();
            let old = review::source(&f).unwrap();
            let native = NativeRegionQueries::curve(&review::current_curve(&f).unwrap());
            let policy = GeometryPolicy::default();
            let [a, b] = native.domain(policy).unwrap();
            let edit =
                review::trim(&f, 0.99).unwrap_or_else(|e| panic!("{rational}/{boundary:?}: {e}"));
            assert_eq!(f.session.document(), &original);
            assert_eq!(edit.report.replacements[&f.target], vec![f.target]);
            let limit = edit.report.contacts.last().unwrap().parameter;
            assert!(limit > 0. && limit < 1.);
            assert!(edit.report.removed_curves.is_empty());
            let created = edit.report.created_points.clone();
            f.session.commit(edit.edit.preview.unwrap()).unwrap();
            let after = f.session.document().clone();
            let source = review::source(&f).unwrap();
            assert_eq!(
                source.sketch.points[..old.sketch.points.len()],
                old.sketch.points
            );
            assert_eq!(source.sketch.constraints, old.sketch.constraints);
            let retained = NativeRegionQueries::curve(&review::current_curve(&f).unwrap());
            let [x, y] = retained.domain(policy).unwrap();
            for i in 0..=40 {
                let t = i as f64 / 40.;
                point_near(
                    retained.evaluate(x + (y - x) * t, policy).unwrap().point,
                    native
                        .evaluate(a + (b - a) * limit * t, policy)
                        .unwrap()
                        .point,
                );
            }
            let reopened = review::reopen(&review::archive(f.session.document()).unwrap()).unwrap();
            assert_eq!(reopened.document(), &after);
            assert!(
                created
                    .iter()
                    .all(|id| source.sketch.points.iter().any(|p| p.id == *id))
            );
            f.session.undo(1, review::actor()).unwrap();
            assert_eq!(f.session.document(), &original);
            f.session.redo(2, review::actor()).unwrap();
            assert_eq!(f.session.document(), &after);
        }
    }
}
#[test]
fn genuine_both_end_tangent_continuations_require_explicit_contacts_and_keep_original_interval() {
    for endpoint in [Endpoint::Start, Endpoint::End] {
        let mut f = review::seed(true, review::BoundaryKind::Line).unwrap();
        let original_curve = review::current_curve(&f).unwrap();
        let original_native = NativeRegionQueries::curve(&original_curve);
        let policy = GeometryPolicy::default();
        let [a, b] = original_native.domain(policy).unwrap();
        let mut r = review::continue_request(&mut f, endpoint).unwrap();
        let before = f.session.document().clone();
        let revision = f.session.revision();
        let contacts = multicad_solver::extension_contacts_with_queries(
            &f.session,
            revision,
            f.sketch,
            None,
            &r,
            Default::default(),
            &NativeRegionQueries::default(),
        )
        .unwrap();
        assert_eq!(contacts.len(), 2);
        let error = multicad_solver::prepare_trim_with_queries(
            &f.session,
            revision,
            review::actor(),
            f.sketch,
            None,
            &TrimRequest::ExtendSpline(r.clone()),
            Default::default(),
            &NativeRegionQueries::default(),
        )
        .err()
        .unwrap();
        assert!(
            matches!(error,multicad_solver::TrimEditError::Geometry(ref e) if e.kind==TrimErrorKind::AmbiguousIntersection)
        );
        assert_eq!(f.session.document(), &before);
        r.contact_index = Some(if endpoint == Endpoint::Start { 0 } else { 1 });
        let edit = multicad_solver::prepare_trim_with_queries(
            &f.session,
            revision,
            review::actor(),
            f.sketch,
            None,
            &TrimRequest::ExtendSpline(r),
            Default::default(),
            &NativeRegionQueries::default(),
        )
        .unwrap();
        assert_eq!(edit.report.replacements[&f.target], vec![f.target]);
        f.session.commit(edit.edit.preview.unwrap()).unwrap();
        let result = NativeRegionQueries::curve(&review::current_curve(&f).unwrap());
        for i in 0..=50 {
            let p = original_native
                .evaluate(a + (b - a) * i as f64 / 50., policy)
                .unwrap()
                .point;
            let hits = multicad_core::sketch_regions::RegionQueries::project_parameters(
                &NativeRegionQueries::default(),
                &review::current_curve(&f).unwrap(),
                p,
                1e-7,
            )
            .unwrap();
            assert!(hits.iter().any(|h| h.distance_mm <= 1e-7));
        }
        assert!(result.self_intersections(policy).unwrap().points.is_empty());
        let saved = review::archive(f.session.document()).unwrap();
        assert_eq!(
            review::reopen(&saved).unwrap().document(),
            f.session.document()
        );
        let after = f.session.document().clone();
        f.session
            .undo(f.session.revision(), review::actor())
            .unwrap();
        assert_eq!(f.session.document(), &before);
        f.session
            .redo(f.session.revision(), review::actor())
            .unwrap();
        assert_eq!(f.session.document(), &after);
    }
}
#[test]
fn native_trim_uses_current_parameter_boundary_and_rejects_changed_pole_constraints_without_commit()
{
    let mut f = review::seed(true, review::BoundaryKind::Line).unwrap();
    let first = review::trim(&f, 0.99).unwrap();
    let before = f.session.document().clone();
    let x = first.report.contacts[0].point_mm[0];
    near(x, 10.);
    drop(first);
    assert_eq!(f.session.document(), &before);
    assert!(!f.session.can_undo());
    let mut p = f.session.document().get(f.cut_parameter).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut p.data else {
        panic!()
    };
    *expression = "12".into();
    review::apply(
        &mut f,
        "Move exact cut boundary by parameter",
        vec![Mutation::Replace(p)],
    )
    .unwrap();
    let edit = review::trim(&f, 0.99).unwrap();
    near(edit.report.contacts[0].point_mm[0], 12.);
    let mut s = review::source(&f).unwrap();
    let CurveKind::BSpline { ref poles, .. } = s
        .sketch
        .curves
        .iter()
        .find(|c| c.id == f.target)
        .unwrap()
        .kind
    else {
        panic!()
    };
    let pole = poles[1];
    s.add_constraint(ConstraintKind::Fix { point: pole });
    let edits = s.replace(f.session.document(), f.sketch).unwrap();
    review::apply(
        &mut f,
        "Constrain pole that segmentation would replace",
        edits,
    )
    .unwrap();
    let before = f.session.document().clone();
    let e = review::trim(&f, 0.99).err().unwrap();
    assert!(e.to_string().contains("constrained"));
    assert_eq!(f.session.document(), &before);
    assert!(
        f.session.commit(edit.edit.preview.unwrap()).is_err(),
        "stale receipt cannot commit after parameter/constraint change"
    );
}
#[test]
fn trimmed_exact_regions_generate_real_native_solids_and_generator_reopens_without_overwrite() {
    for rational in [false, true] {
        let mut f = review::seed(rational, review::BoundaryKind::Line).unwrap();
        let edit = review::trim(&f, 0.99).unwrap();
        f.session.commit(edit.edit.preview.unwrap()).unwrap();
        let (body, feature, area) = review::close_and_extrude(&mut f).unwrap();
        let model = Compiler::default()
            .compile(f.session.document(), f.session.revision())
            .unwrap();
        let shape = &model.snapshot().shapes[&feature];
        assert!(area > 0.);
        assert_eq!(shape.solid_count, 1);
        near(shape.volume_mm3, area * 10.);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("trimmed.step");
        model.solid(feature).unwrap().write_step(&path).unwrap();
        let step = multicad_kernel::Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        near(step.volume_mm3().unwrap(), shape.volume_mm3);
        let reopened = review::reopen(&review::archive(f.session.document()).unwrap()).unwrap();
        let again = Compiler::default().compile(reopened.document(), 0).unwrap();
        assert_eq!(again.snapshot().bodies[&body], feature);
        near(
            again.snapshot().shapes[&feature].volume_mm3,
            shape.volume_mm3,
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("new-spline-review");
    let manifest = review::generate(&dir).unwrap();
    assert_eq!(manifest["projects"].as_array().unwrap().len(), 10);
    let bytes = std::fs::read(dir.join("manifest.json")).unwrap();
    assert!(review::generate(&dir).is_err());
    assert_eq!(std::fs::read(dir.join("manifest.json")).unwrap(), bytes);
}
#[test]
fn genuine_missing_hit_tangent_and_overlap_have_no_commit_capable_result() {
    let mut f = review::seed(false, review::BoundaryKind::Line).unwrap();
    let mut request = review::continue_request(&mut f, Endpoint::End).unwrap();
    request.max_length_mm = 0.1;
    let before = f.session.document().clone();
    let contacts = multicad_solver::extension_contacts_with_queries(
        &f.session,
        f.session.revision(),
        f.sketch,
        None,
        &request,
        Default::default(),
        &NativeRegionQueries::default(),
    )
    .unwrap();
    assert!(contacts.is_empty());
    let error = multicad_solver::prepare_trim_with_queries(
        &f.session,
        f.session.revision(),
        review::actor(),
        f.sketch,
        None,
        &TrimRequest::ExtendSpline(request),
        Default::default(),
        &NativeRegionQueries::default(),
    )
    .err()
    .unwrap();
    assert!(
        matches!(error,multicad_solver::TrimEditError::Geometry(ref e) if e.kind==TrimErrorKind::NoIntersection)
    );
    assert_eq!(f.session.document(), &before);
    let mut s = review::source(&f).unwrap();
    let tangent = review::line(&mut s, [-10., 7.5], [30., 7.5], true);
    let mut overlap = s
        .sketch
        .curves
        .iter()
        .find(|c| c.id == f.target)
        .unwrap()
        .clone();
    overlap.id = multicad_core::document::EntityId::new();
    overlap.construction = true;
    let overlapping = overlap.id;
    s.sketch.curves.push(overlap);
    let edits = s.replace(f.session.document(), f.sketch).unwrap();
    review::apply(
        &mut f,
        "Add native tangent and overlap boundary fixtures",
        edits,
    )
    .unwrap();
    let source = f.session.document().clone();
    for (boundary, want) in [
        (tangent, TrimErrorKind::AmbiguousIntersection),
        (overlapping, TrimErrorKind::Overlap),
    ] {
        let request = TrimRequest::TrimCurve(multicad_core::sketch_trim::TrimCurve {
            target: f.target,
            pick_mm: review::point_at(&f, 0.9).unwrap(),
            boundaries: std::collections::BTreeSet::from([boundary]),
        });
        let e = multicad_solver::prepare_trim_with_queries(
            &f.session,
            f.session.revision(),
            review::actor(),
            f.sketch,
            None,
            &request,
            Default::default(),
            &NativeRegionQueries::default(),
        )
        .err()
        .unwrap();
        assert!(
            matches!(e,multicad_solver::TrimEditError::Geometry(ref e) if e.kind==want),
            "{e}"
        );
        assert_eq!(f.session.document(), &source);
    }
}
