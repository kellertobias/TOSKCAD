//! Visual placement and the System 32 tool, driven through the production
//! native scene and the same form values the Inspector edits.
use super::*;
use crate::modeling::{
    panel_face,
    panel_fixture::{actor, carcass, flat_panel},
};
use multicad_core::model::{PanelRole, PanelSource};
use multicad_model::Compiler;

fn click(form: &mut MachiningPanel, s: &Session, point: [f32; 3]) -> bool {
    crate::modeling::panel_fixture::click(s, point, |view| {
        form.viewport(s.document(), s.revision(), view)
    })
}

#[test]
fn clicking_a_visible_panel_face_adds_the_target_and_places_and_moves_holes() {
    let (s, side) = flat_panel();
    let mut form = MachiningPanel::new("Cable".into());
    form.kind = Kind::Drill;
    form.holes.clear();
    form.through = true;
    // The top face at z = 18, clicked near (123.4, 57.6): snapped to whole mm.
    assert!(click(&mut form, &s, [123.4, 57.6, 18.]));
    assert_eq!(form.targets, vec![side]);
    assert_eq!(form.side, Side::Top);
    assert_eq!(form.hole_points(), vec![[123., 58.]]);
    // Near the center lines the point snaps onto them.
    assert!(click(&mut form, &s, [298., 203., 18.]));
    assert_eq!(form.hole_points(), vec![[123., 58.], [300., 200.]]);
    // Clicking an existing hole selects it; the next click moves it.
    assert!(click(&mut form, &s, [123., 58., 18.]));
    assert_eq!(form.selected_hole, Some(0));
    assert!(click(&mut form, &s, [450., 100., 18.]));
    assert_eq!(form.hole_points(), vec![[450., 100.], [300., 200.]]);
    // The Inspector values build the same exact cut.
    let prepared = form
        .request(s.document(), true)
        .unwrap()
        .prepare(&s, actor())
        .unwrap();
    let model = Compiler::default()
        .compile(prepared.prepared.document(), 2)
        .unwrap();
    let v = model
        .solid(prepared.outputs[0].feature)
        .unwrap()
        .volume_mm3()
        .unwrap();
    let hole = std::f64::consts::PI * 2.5 * 2.5 * 18.;
    assert!((v - (600. * 400. * 18. - 2. * hole)).abs() < 1e-3);
}

#[test]
fn a_hole_outside_the_face_is_refused_with_the_panel_named() {
    let (s, side) = flat_panel();
    let mut form = MachiningPanel::new("Edge hole".into());
    form.kind = Kind::Drill;
    form.diameter = field(20.);
    form.through = true;
    form.holes.clear();
    assert!(click(&mut form, &s, [3., 200., 18.]));
    // Snapped onto the edge: a 20 mm hole there leaves the face.
    assert_eq!(form.hole_points(), vec![[0., 200.]]);
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    let resolved = form.resolved(s.document(), &parameters);
    assert_eq!(resolved[0].0, side);
    let error = resolved[0].1.as_ref().unwrap_err();
    assert!(
        error.contains("leaves the 600 × 400 mm face of Side"),
        "{error}"
    );
    // A pocket click sets its center.
    form.kind = Kind::Pocket;
    assert!(click(&mut form, &s, [200., 100., 18.]));
    assert_eq!(form.center[0].text, "200");
    assert_eq!(form.center[1].text, "100");
}

#[test]
fn system32_tool_targets_both_sides_places_rows_and_reopens() {
    let (mut s, left, right) = carcass();
    // Selecting one side seeds both, so opposite rows stay aligned.
    let (panels, refused) = panel_face::selected_panels(s.document(), [left]);
    assert!(refused.is_empty());
    let targets = panel_face::with_opposite_sides(s.document(), panels);
    assert_eq!(targets, vec![left, right]);
    let mut form = MachiningPanel::system32("Shelf pins".into()).with_targets(targets);
    assert_eq!(form.label(), "System 32 shelf pins");
    assert_eq!(form.side, Side::Inside);
    // A click on the right side's inner face (x = 582) 50 mm from the front
    // moves row 1 there; the face stays "inside" for both sides.
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    let hit = panel_face::locate(s.document(), &parameters, right, [582., 50.2, 300.]).unwrap();
    assert_eq!(hit.face, MachiningFace::Bottom);
    let at = panel_face::snap(hit.at, hit.frame.size, 1., 5.);
    assert!(form.click(s.document(), &parameters, hit, at, |_, _| f32::MAX));
    assert_eq!(form.pin_rows[0].1.text, "50");
    assert_eq!(form.side, Side::Inside);
    // An edge click is explained, not applied.
    let edge = panel_face::locate(s.document(), &parameters, left, [9., 0., 300.]).unwrap();
    assert!(matches!(edge.face, MachiningFace::Edge { .. }));
    form.click(s.document(), &parameters, edge, edge.at, |_, _| f32::MAX);
    assert!(form.pick_note.as_deref().unwrap().contains("inside face"));
    assert_eq!(form.side, Side::Inside);

    let resolved = form.resolved(s.document(), &parameters);
    let faces: Vec<_> = resolved
        .iter()
        .map(|(_, r)| r.as_ref().unwrap().face)
        .collect();
    assert_eq!(faces, vec![MachiningFace::Top, MachiningFace::Bottom]);
    assert!(
        resolved
            .iter()
            .all(|(_, r)| r.as_ref().unwrap().holes.len() == 34)
    );
    let prepared = form
        .request(s.document(), true)
        .unwrap()
        .prepare(&s, actor())
        .unwrap();
    let cut = prepared.outputs[0].feature;
    s.commit(prepared.prepared).unwrap();
    let edit = MachiningPanel::existing(s.document(), cut).unwrap();
    assert_eq!(edit.kind, Kind::ShelfPins);
    assert_eq!(edit.side, Side::Inside);
    assert_eq!(edit.targets, vec![left, right]);
    assert_eq!(edit.pin_rows[0].1.text, "50");
    assert_eq!(edit.pin_rows[1].0, RowEdge::End);
    // Targets are fixed on an applied operation: another panel is refused.
    let mut edit = edit;
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    let top = model::carcass_panels(
        s.document(),
        model::read_panel(
            s.document(),
            model::panel_feature(s.document(), left).unwrap(),
        )
        .map(|p| match p.source {
            PanelSource::Carcass { carcass, .. } => carcass,
            _ => unreachable!(),
        })
        .unwrap(),
    )
    .unwrap()
    .into_iter()
    .find(|(r, ..)| *r == PanelRole::Top)
    .unwrap()
    .2;
    let hit = panel_face::locate(s.document(), &parameters, top, [300., 200., 720.]).unwrap();
    assert!(!edit.click(s.document(), &parameters, hit, hit.at, |_, _| f32::MAX));
    assert!(edit.pick_note.as_deref().unwrap().contains("fixed"));
}

#[test]
fn a_visual_drill_edit_on_the_six_panel_cabinet_keeps_identity_and_undoes() {
    let (mut s, left, _right) = crate::modeling::panel_fixture::six_panel_carcass();
    let panels = model::panels(s.document()).count();
    assert_eq!(panels, 6);
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    // Click the left side's inner face (x = 18) 100 mm from the front, 300 up.
    let mut form = MachiningPanel::new("Hinge holes".into());
    form.kind = Kind::Drill;
    form.holes.clear();
    form.diameter = field(5.);
    let hit = panel_face::locate(s.document(), &parameters, left, [18., 100.4, 299.6]).unwrap();
    assert_eq!(hit.face, MachiningFace::Top);
    let at = panel_face::snap(hit.at, hit.frame.size, 1., 5.);
    assert!(form.click(s.document(), &parameters, hit, at, |_, _| f32::MAX));
    assert_eq!(form.targets, vec![left]);
    assert_eq!(form.side, Side::Top);
    assert_eq!(form.hole_points(), vec![[100., 300.]]);
    // A hole that would leave the face is flagged while hovering.
    let edge = panel_face::locate(s.document(), &parameters, left, [18., 1., 300.]).unwrap();
    let near_edge = panel_face::snap(edge.at, edge.frame.size, 1., 5.);
    assert!(
        form.hover_problem(s.document(), &parameters, &edge, near_edge)
            .unwrap()
            .contains("Outside")
    );
    assert!(
        form.hover_problem(s.document(), &parameters, &hit, at)
            .is_none()
    );
    let prepared = form
        .request(s.document(), true)
        .unwrap()
        .prepare(&s, actor())
        .unwrap();
    let cut = prepared.outputs[0].feature;
    s.commit(prepared.prepared).unwrap();
    let applied = s.revision();
    let center = |s: &Session| {
        let m = Compiler::default()
            .compile(s.document(), s.revision())
            .unwrap();
        m.machining(s.document()).unwrap()[0].holes[0].center
    };
    assert_eq!(center(&s), [18., 100., 300.]);

    // Reopen, select the hole on the face and move it with a second click.
    let mut edit = MachiningPanel::existing(s.document(), cut).unwrap();
    let op = edit.existing.unwrap();
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    let hole = panel_face::locate(s.document(), &parameters, left, [18., 100., 300.]).unwrap();
    // The pointer is on the hole marker: it selects instead of adding.
    assert!(edit.click(s.document(), &parameters, hole, hole.at, |_, _| 0.));
    assert_eq!(edit.selected_hole, Some(0));
    let to = panel_face::locate(s.document(), &parameters, left, [18., 150., 420.]).unwrap();
    assert!(edit.click(s.document(), &parameters, to, to.at, |_, _| f32::MAX));
    assert_eq!(edit.hole_points(), vec![[150., 420.]]);
    let prepared = edit
        .request(s.document(), true)
        .unwrap()
        .prepare(&s, actor())
        .unwrap();
    // The same operation and cut feature on the same Part.
    assert_eq!(prepared.outputs.len(), 1);
    assert_eq!(prepared.outputs[0].feature, cut);
    assert_eq!(prepared.outputs[0].body, left);
    s.commit(prepared.prepared).unwrap();
    assert_eq!(
        model::read_machining(s.document(), op).unwrap().targets,
        vec![left]
    );
    assert_eq!(center(&s), [18., 150., 420.]);
    assert_eq!(model::panels(s.document()).count(), 6);

    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.revision(), applied + 2);
    assert_eq!(center(&s), [18., 100., 300.]);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(center(&s), [18., 150., 420.]);
    assert!(model::read_machining_cut(s.document(), cut).is_ok());
}
