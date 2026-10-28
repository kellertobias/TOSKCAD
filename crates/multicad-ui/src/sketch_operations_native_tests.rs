//! Form-to-native contracts. Host pointer routing and document lifecycle have
//! separate integration coverage; these tests never substitute a query mock.
use super::*;
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{Curve, SplineBasis},
    document::Document,
    sketch::Datum,
    sketch_trim::{Action, Parameterization},
};
use multicad_kernel::NativeRegionQueries;
use multicad_solver::{extension_contacts_with_queries, prepare_trim_with_queries};

fn fixture(
    rational: bool,
    extension: bool,
) -> (Session, EntityId, Source, EntityId, Vec<EntityId>) {
    let (sketch, edits) = Source::create(Datum::Xy, "Native spline controls").unwrap();
    let document = Document::new("Native form query")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut source, _, _) = Source::read(&document, sketch).unwrap();
    let poles = [[0., 0.], [10., 4.], [20., -4.], [30., 0.]]
        .into_iter()
        .map(|point| source.add_point(point))
        .collect();
    let mut basis = SplineBasis::open_uniform(3, 4).unwrap();
    if rational {
        basis.weights = Some(vec![1., 2., 2., 1.]);
    }
    let target = EntityId::new();
    source.sketch.curves.push(Curve {
        id: target,
        construction: false,
        kind: CurveKind::BSpline { poles, basis },
    });
    let boundaries = if extension { [40., 50.] } else { [10., 20.] }
        .into_iter()
        .map(|x| {
            let a = source.add_point([x, -50.]);
            let b = source.add_point([x, 50.]);
            source.add_line(a, b)
        })
        .collect();
    let session = Session::new(
        document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap(),
    )
    .unwrap();
    (session, sketch, source, target, boundaries)
}
fn configure(
    panel: &mut Panel,
    source: &Source,
    target: EntityId,
    boundaries: &[EntityId],
    tool: OperationTool,
) {
    panel.begin(tool);
    panel
        .set_curve_pick(source, target, [15., 0.], false)
        .unwrap();
    for boundary in boundaries {
        panel
            .set_curve_pick(source, *boundary, [0., 0.], true)
            .unwrap();
    }
}
fn request(panel: &Panel, source: &Source) -> (TrimRequest, TrimPolicy) {
    let OperationRequest::Trim(request, policy) = panel.request(source, &[]).unwrap() else {
        panic!("Trim/Extend form")
    };
    (request, policy)
}
#[test]
fn rational_and_nonrational_trim_forms_produce_actual_native_solver_previews_without_source_edits()
{
    for rational in [false, true] {
        let (session, sketch, source, target, boundaries) = fixture(rational, false);
        let before = session.document().clone();
        let mut panel = Panel::default();
        configure(
            &mut panel,
            &source,
            target,
            &boundaries,
            OperationTool::Trim,
        );
        let (request, policy) = request(&panel, &source);
        let result = prepare_trim_with_queries(
            &session,
            0,
            Actor::local(Channel::Ui),
            sketch,
            None,
            &request,
            policy,
            &NativeRegionQueries::default(),
        )
        .unwrap();
        assert_eq!(result.report.target, target);
        assert_eq!(
            result.report.parameterization,
            Parameterization::CurveFraction
        );
        assert_eq!(result.report.contacts.len(), 2);
        assert_eq!(result.report.replacements[&target].len(), 2);
        let preview = result
            .edit
            .preview
            .as_ref()
            .expect("Actual checked native trim preview");
        session.validate_preview(preview).unwrap();
        assert!(result.edit.semantic.as_ref().unwrap().solved().is_some());
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
        assert!(!session.can_undo());
        panel.cancel();
        assert_eq!(session.document(), &before);
    }
}
#[test]
fn actual_native_contacts_require_a_real_form_choice_before_extension_can_prepare() {
    use super::spline_tests::{click, frame, text_center};
    let (session, sketch, source, target, boundaries) = fixture(true, true);
    let before = session.document().clone();
    let mut panel = Panel::default();
    configure(
        &mut panel,
        &source,
        target,
        &boundaries,
        OperationTool::Extend,
    );
    let (TrimRequest::ExtendSpline(extension), policy) = request(&panel, &source) else {
        panic!("Spline extension")
    };
    let queries = NativeRegionQueries::default();
    let contacts =
        extension_contacts_with_queries(&session, 0, sketch, None, &extension, policy, &queries)
            .unwrap();
    assert_eq!(contacts.len(), 2);
    assert!(contacts[0].parameter < contacts[1].parameter);
    let selected = contacts[1].point_mm;
    assert!((selected[0] - 50.).abs() < 1e-7);
    assert!((selected[1] - 8.).abs() < 1e-7);
    panel.set_extension_contacts(&extension, contacts).unwrap();
    assert!(
        panel
            .request(&source, &[])
            .unwrap_err()
            .contains("Choose an exact contact")
    );
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(
        &ctx,
        &mut panel,
        &source,
        text_center(&output, "Choose exact contact"),
    );
    let output = frame(&ctx, &mut panel, &source, vec![]);
    let label = format!("Contact 2 · X {:.6} / Y {:.6} mm", selected[0], selected[1]);
    click(&ctx, &mut panel, &source, text_center(&output, &label));
    let (request, policy) = request(&panel, &source);
    let TrimRequest::ExtendSpline(chosen) = &request else {
        panic!("Chosen spline")
    };
    assert_eq!(chosen.contact_index, Some(1));
    let result = prepare_trim_with_queries(
        &session,
        0,
        Actor::local(Channel::Ui),
        sketch,
        None,
        &request,
        policy,
        &queries,
    )
    .unwrap();
    session
        .validate_preview(
            result
                .edit
                .preview
                .as_ref()
                .expect("Checked native extension"),
        )
        .unwrap();
    let Action::Extend { to_mm, .. } = result.report.action else {
        panic!("Extension report")
    };
    assert!((to_mm[0] - selected[0]).abs() < 1e-7);
    assert!((to_mm[1] - selected[1]).abs() < 1e-7);
    panel.clear_extension_contacts();
    panel.max_length_mm = 5.;
    let (TrimRequest::ExtendSpline(short), policy) = self::request(&panel, &source) else {
        panic!("Bounded extension")
    };
    assert!(
        extension_contacts_with_queries(&session, 0, sketch, None, &short, policy, &queries)
            .unwrap()
            .is_empty()
    );
    assert!(
        prepare_trim_with_queries(
            &session,
            0,
            Actor::local(Channel::Ui),
            sketch,
            None,
            &TrimRequest::ExtendSpline(short),
            policy,
            &queries
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
    assert!(!session.can_undo());
}
