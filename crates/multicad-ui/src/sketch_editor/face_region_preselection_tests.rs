use super::tests::{input, session, settle};
use super::*;
use multicad_core::{
    constraint_sketch::source::Source,
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    references::TopologyKind,
    sketch::{self, CreationPlane, PlaneOptions},
};
use multicad_kernel::NativeRegionQueries;
use multicad_model::Compiler;
use std::sync::Arc;

#[test]
fn finished_face_sketch_picks_nested_regions_in_model_in_both_orders() {
    let mut session = session();
    let actor = Actor::local(Channel::Ui);
    let stock = PrimitiveRequest::new(
        "Box",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(10.)],
            frame: PrimitiveFrame::default(),
        },
    );
    let created =
        model::prepare_primitive(&session, session.revision(), actor.clone(), &stock).unwrap();
    session.commit(created.prepared).unwrap();
    let compiler = Compiler::default();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let snapshot = &native.snapshot().shapes[&stock.feature].topology;
    let top = native
        .solid(stock.feature)
        .unwrap()
        .faces()
        .unwrap()
        .into_iter()
        .find(|face| {
            face.plane
                .is_some_and(|(point, normal)| (point[2] - 10.).abs() < 1e-7 && normal[2] > 0.9)
        })
        .unwrap();
    let element = snapshot
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.key.ordinal == top.index)
        .unwrap();
    let created = sketch::prepare_new(
        &session,
        session.revision(),
        actor.clone(),
        "Nested rectangle",
        CreationPlane::Face {
            snapshot,
            key: element.key,
            anchor: Some(&element.frames[0].origin),
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let id = created.sketch;
    let frame = created.frame;
    session.commit(created.prepared).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let (mut source, _, _) = Source::read_for_evaluation(
        session.document(),
        id,
        session.revision(),
        Some(&native.snapshot().references),
    )
    .unwrap();
    let corners = [
        [5., 5., 10.],
        [15., 5., 10.],
        [15., 15., 10.],
        [5., 15., 10.],
    ]
    .map(|point| source.add_point(frame.coordinates(point)));
    for (start, end) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
        source.add_line(start, end);
    }
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor,
            label: "Rectangle".into(),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let scene =
        crate::modeling::native_fixture::scene(session.document(), native.snapshot(), 1).unwrap();
    let mut editor = SketchEditor {
        evaluated: Some(native.snapshot().references.clone()),
        ..Default::default()
    };
    editor.set_query_factory(Arc::new(|| Ok(Box::new(NativeRegionQueries::default()))));
    editor.activate_sketch(id).unwrap();
    editor.orient_to(frame);
    settle(&mut editor, &session);
    editor.finish().unwrap();

    let ctx = egui::Context::default();
    ctx.run_ui(input(vec![]), |ui| {
        editor.canvas(ui, &mut session, false, Some(scene.clone()), true);
    })
    .drop_without_applying_deltas();
    let rect = editor.viewport_rect.unwrap();
    let points = [[10., 10., 10.], [2., 2., 10.]].map(|point| {
        let pixel = editor.camera.project(point, rect);
        Pos2::new(pixel[0], pixel[1])
    });
    let click = |editor: &mut SketchEditor, session: &mut Session, index: usize, shift: bool| {
        let pos = points[index];
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };
        for pressed in [true, false] {
            ctx.run_ui(
                input(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ]),
                |ui| {
                    editor.canvas(ui, session, false, Some(scene.clone()), true);
                },
            )
            .drop_without_applying_deltas();
        }
        settle(editor, session);
        assert!(editor.solid.selected_id().is_none());
    };
    for (first, second) in [(0, 1), (1, 0)] {
        click(&mut editor, &mut session, first, false);
        let single = editor
            .selected_profiles(session.document(), session.revision())
            .unwrap()
            .1;
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].holes.len(), first);
        click(&mut editor, &mut session, second, true);
        let both = editor
            .selected_profiles(session.document(), session.revision())
            .unwrap()
            .1;
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].holes.len(), first);
        assert_eq!(both[1].holes.len(), second);
        assert_eq!(
            editor.profile_inputs(session.document(), session.revision()),
            Ok((id, both.clone()))
        );
        click(&mut editor, &mut session, first, true);
        assert_eq!(editor.profiles.selected(), &both[1..]);
        click(&mut editor, &mut session, second, true);
        assert!(editor.profiles.selected().is_empty());
        assert!(
            editor
                .profile_inputs(session.document(), session.revision())
                .is_err()
        );
    }
}
