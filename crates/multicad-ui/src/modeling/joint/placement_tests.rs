//! The Domino joint form: panels picked on their visible faces in the
//! production scene, previewed natively, applied and reopened.
use super::*;
use crate::modeling::panel_fixture::{actor, carcass, click};
use multicad_model::Compiler;

fn pick(form: &mut JointPanel, s: &Session, point: [f32; 3]) -> bool {
    click(s, point, |view| {
        form.viewport(s.document(), s.revision(), view)
    })
}

#[test]
fn domino_form_picks_a_top_and_side_and_builds_mortises_and_tenons() {
    let (mut s, _left, right) = carcass();
    let mut form = JointPanel::loose_tenon("Top joint".into());
    form.pairs.clear();
    assert_eq!(form.label(), "Loose tenon (Domino)");
    // A top view: the top panel's upper face, then the right side's top edge.
    assert!(pick(&mut form, &s, [300., 200., 720.]));
    assert!(pick(&mut form, &s, [591., 200., 720.]));
    let top = form.pairs[0][0].unwrap();
    assert_eq!(s.document().get(top).unwrap().name, "Cabinet top");
    assert_eq!(form.pairs[0][1], Some(right));
    // The same panel cannot be both sides of a pair.
    assert!(pick(&mut form, &s, [300., 200., 720.]));
    assert!(!pick(&mut form, &s, [300., 200., 720.]));
    assert!(
        form.pick_note
            .as_deref()
            .unwrap()
            .contains("already panel A")
    );
    form.pairs.truncate(1);
    let request = form.request(s.document(), true).unwrap();
    let PanelJointKind::LooseTenon(spec) = &request.joint.kind else {
        panic!("not a loose tenon");
    };
    assert_eq!(spec.count, 2);
    let prepared = request.prepare(&s, actor()).unwrap();
    assert_eq!(prepared.outputs.len(), 3);
    let tenon = prepared.outputs[2].clone();
    let model = Compiler::default()
        .compile(prepared.prepared.document(), s.revision() + 1)
        .unwrap();
    let section = (22. - 8.) * 8. + std::f64::consts::PI * 16.;
    let v = model.solid(tenon.feature).unwrap().volume_mm3().unwrap();
    assert!((v - 2. * section * 40.).abs() < 1e-3, "{v}");
    s.commit(prepared.prepared).unwrap();
    // Opening the tenon Part reopens its joint with the pair fixed.
    let mut edit = JointPanel::existing(s.document(), tenon.feature).unwrap();
    assert_eq!(edit.kind, Kind::LooseTenon);
    assert_eq!(edit.pairs, vec![[Some(top), Some(right)]]);
    assert_eq!(edit.tenon.face_depth.text, "15");
    assert!(!edit.pick(s.document(), right));
    assert!(
        edit.pick_note
            .as_deref()
            .unwrap()
            .contains("keeps its panel pairs")
    );
}

#[test]
fn a_domino_between_panels_that_do_not_touch_is_explained() {
    let (s, left, right) = carcass();
    let form = JointPanel::loose_tenon("Bad".into()).with_pair(Some(left), Some(right));
    let request = form.request(s.document(), true).unwrap();
    let error = request.prepare(&s, actor()).unwrap_err();
    assert!(error.contains("do not meet face to edge"), "{error}");
}
