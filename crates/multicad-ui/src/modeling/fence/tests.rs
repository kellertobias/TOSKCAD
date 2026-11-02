use super::*;
use multicad_core::{
    commands::{Actor, Channel},
    document::{Entity, Mutation},
    project_file,
};
use multicad_model::Compiler;
fn session() -> Session {
    let material = Entity::new(
        "Timber",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [150, 100, 70, 255],
            section_hatch: None,
        },
    );
    Session::new(
        Document::new("Fence form")
            .unwrap()
            .edited(&[Mutation::Insert(material)])
            .unwrap(),
    )
    .unwrap()
}
fn prepare(panel: &FencePanel, s: &Session) -> multicad_core::model::ModelPrepared {
    ModelRequest::Fence(Box::new(panel.request(s.document(), true).unwrap()))
        .prepare(s, Actor::local(Channel::Ui))
        .unwrap()
}

#[test]
fn drawn_closed_fence_previews_native_parts_and_cancel_restores_corner_identity() {
    let s = session();
    let before = s.document().clone();
    let mut panel = FencePanel::new(&s, "Site fence".into(), None).unwrap();
    panel.outline.start_drawing();
    for raw in [
        [2., 3.],
        [4004., -2.],
        [3998., 3003.],
        [-3., 2996.],
        [4., 4.],
    ] {
        let points = panel.outline.positions(s.document()).unwrap();
        panel.outline.click(raw, &points, &[], 25.);
    }
    assert!(panel.outline.closed() && !panel.outline.capturing());
    panel.invalidate();
    let preview = prepare(&panel, &s);
    let native = Compiler::default()
        .compile(preview.prepared.document(), 0)
        .unwrap();
    assert_eq!(native.snapshot().bodies.len(), 16);
    let ids = panel.outline.vertex_ids().to_vec();
    let points = panel.outline.dimensions(s.document()).unwrap();
    panel.outline.start_drawing();
    panel.outline.click([500., 500.], &[], &[], 25.);
    assert!(panel.outline.cancel());
    assert_eq!(panel.outline.vertex_ids(), ids);
    assert_eq!(panel.outline.dimensions(s.document()).unwrap(), points);
    assert_eq!(s.document(), &before);
}

#[test]
fn form_edits_keep_source_scope_member_ids_and_native_reopen() {
    let mut s = session();
    let panel = FencePanel::new(&s, "Boundary".into(), None).unwrap();
    let run = panel.run;
    let prepared = prepare(&panel, &s);
    s.commit(prepared.prepared).unwrap();
    let original = fence_members(s.document(), run).unwrap();
    let mut edit = FencePanel::existing(&s, run).unwrap();
    let ids = edit.outline.vertex_ids().to_vec();
    edit.outline.set_corner(1, [4500., 0.]);
    edit.invalidate();
    assert_eq!(edit.outline.vertex_ids(), ids);
    let preview = prepare(&edit, &s);
    let model = Compiler::default()
        .compile(preview.prepared.document(), s.revision())
        .unwrap();
    assert_eq!(model.snapshot().bodies.len(), 7);
    for (key, feature, body) in &original {
        if matches!(key, FenceMemberKey::Post { .. }) {
            assert!(
                fence_members(preview.prepared.document(), run)
                    .unwrap()
                    .contains(&(*key, *feature, *body))
            );
        }
    }
    s.commit(preview.prepared).unwrap();
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
    let edit = FencePanel::existing(&opened, run).unwrap();
    assert_eq!(edit.outline.vertex_ids(), ids);
    assert_eq!(
        Compiler::default()
            .compile(opened.document(), 0)
            .unwrap()
            .snapshot()
            .bodies
            .len(),
        7
    );
}

#[test]
fn explicit_post_offsets_and_orphan_repair_use_the_same_canonical_plan() {
    let s = session();
    let mut panel = FencePanel::new(&s, "Adjusted".into(), None).unwrap();
    panel.outline.set_corner(1, [4500., 0.]);
    panel.invalidate();
    let automatic = panel
        .draft(s.document())
        .automatic
        .as_ref()
        .unwrap()
        .clone();
    let key = automatic
        .posts
        .iter()
        .find(|p| matches!(p.key, FencePostKey::Interior { ordinal: 2, .. }))
        .unwrap()
        .key;
    panel.positions.push(PositionRow {
        post: key,
        offset: field(3500.),
    });
    panel.invalidate();
    let preview = prepare(&panel, &s);
    assert_eq!(
        Compiler::default()
            .compile(preview.prepared.document(), 0)
            .unwrap()
            .snapshot()
            .bodies
            .len(),
        7
    );
    panel
        .outline
        .insert_midpoint(0, &panel.outline.positions(s.document()).unwrap());
    panel.invalidate();
    assert!(
        panel
            .request(s.document(), true)
            .unwrap_err()
            .contains("no longer exists")
    );
    assert!(panel.draft(s.document()).automatic.is_ok());
    panel.positions.clear();
    panel.invalidate();
    assert!(panel.request(s.document(), true).is_ok());
}

fn frame(
    panel: &mut FencePanel,
    ctx: &egui::Context,
    s: &Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0., 0.),
                egui::vec2(420., 2400.),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            panel.sync_source(s);
            panel.show(ui, s.document(), true, PreviewState::Idle);
        },
    );
    out.textures_delta.clear();
    out
}
fn click(panel: &mut FencePanel, ctx: &egui::Context, s: &Session, p: egui::Pos2) {
    for pressed in [true, false] {
        frame(
            panel,
            ctx,
            s,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
fn text_rect(output: &egui::FullOutput, label: &str, prefix: bool) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape {
                (if prefix {
                    text.galley.job.text.starts_with(label)
                } else {
                    text.galley.job.text == label
                })
                .then(|| egui::Rect::from_min_size(text.pos, text.galley.size()))
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("Missing control {label}"))
}

#[test]
fn painted_shared_and_duplicate_controls_keep_update_scope_explicit() {
    let mut s = session();
    let first = FencePanel::new(&s, "First".into(), None).unwrap();
    let first_run = first.run;
    let initial = prepare(&first, &s);
    s.commit(initial.prepared).unwrap();
    let old_type = read_fence_run(s.document(), first_run).unwrap().fence_type;
    let mut second = FencePanel::new(&s, "Second".into(), Some(old_type)).unwrap();
    second.outline.set_corner(0, [0., 3000.]);
    second.outline.set_corner(1, [4000., 3000.]);
    second.invalidate();
    let second_run = second.run;
    let initial = prepare(&second, &s);
    s.commit(initial.prepared).unwrap();
    let before = s.document().clone();
    let original = fence_members(s.document(), first_run).unwrap();
    let mut panel = FencePanel::existing(&s, first_run).unwrap();
    let ctx = egui::Context::default();
    let output = frame(&mut panel, &ctx, &s, vec![]);
    click(
        &mut panel,
        &ctx,
        &s,
        text_rect(&output, "Edit shared construction", false).center(),
    );
    assert!(panel.shared);
    let output = frame(&mut panel, &ctx, &s, vec![]);
    let label = text_rect(&output, "Post height (", true);
    let input = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::epaint::Shape::Rect(rect) = &shape.shape {
                (rect.rect.left() >= label.right() - 2.
                    && rect.rect.top() <= label.center().y
                    && rect.rect.bottom() >= label.center().y
                    && rect.rect.width() > 40.
                    && rect.rect.height() < 50.)
                    .then_some(rect.rect)
            } else {
                None
            }
        })
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("Painted height input");
    click(&mut panel, &ctx, &s, input.center());
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    frame(
        &mut panel,
        &ctx,
        &s,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: command,
            },
            egui::Event::Text("2 m".into()),
        ],
    );
    let request = panel.request(s.document(), true).unwrap();
    assert!(request.edit_type.is_some());
    let shared = prepare(&panel, &s);
    let native = Compiler::default()
        .compile(shared.prepared.document(), s.revision())
        .unwrap();
    assert_eq!(native.snapshot().bodies.len(), 10);
    let value = read_fence_type(shared.prepared.document(), old_type).unwrap();
    assert_eq!(
        value
            .height
            .value_in(
                multicad_core::parameters::Unit::Millimeter,
                Some(shared.prepared.parameters())
            )
            .unwrap(),
        2000.
    );
    assert_eq!(s.document(), &before);
    let output = frame(&mut panel, &ctx, &s, vec![]);
    click(
        &mut panel,
        &ctx,
        &s,
        text_rect(&output, "Duplicate as separate type", false).center(),
    );
    assert!(!panel.shared && panel.selected_type.is_none());
    let command = panel.request(s.document(), true).unwrap();
    assert!(command.edit_type.is_none() && command.new_type.is_some());
    let duplicate = prepare(&panel, &s);
    Compiler::default()
        .compile(duplicate.prepared.document(), s.revision())
        .unwrap();
    let first_type = read_fence_run(duplicate.prepared.document(), first_run)
        .unwrap()
        .fence_type;
    assert_ne!(first_type, old_type);
    assert_eq!(
        read_fence_run(duplicate.prepared.document(), second_run)
            .unwrap()
            .fence_type,
        old_type
    );
    assert_eq!(
        read_fence_type(duplicate.prepared.document(), old_type)
            .unwrap()
            .height,
        length(1800.)
    );
    assert_eq!(
        fence_members(duplicate.prepared.document(), first_run).unwrap(),
        original
    );
    assert_eq!(s.document(), &before);
}

#[test]
fn embedded_fence_definition_opens_without_local_materials_and_uses_real_typed_placement() {
    use crate::symbols::architecture::test_support;
    use multicad_core::symbols::architecture::{
        ArchitectureOutput, ArchitectureRole, read_architecture_instance,
    };
    let actor = || Actor::local(Channel::Ui);
    let (definition, _) = test_support::definition(ArchitectureRole::Fence);
    let (mut session, _) = test_support::consumer(&definition);
    assert!(
        !session
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Material { .. }))
    );
    let mut panel = FencePanel::new(&session, "Pinned boundary".into(), None).unwrap();
    assert!(panel.symbol.active());
    assert!(panel.types.is_empty());
    let first = panel.draft(session.document());
    let second = panel.draft(session.document());
    assert!(
        Arc::ptr_eq(&first, &second),
        "idle frames reuse the resolved interface/layout"
    );
    assert_eq!(first.layout.as_ref().unwrap().posts.len(), 3);
    let request = panel.model_request(session.document(), true).unwrap();
    let ModelRequest::SymbolConstruction(command) = request else {
        panic!("Pinned source must not become an independently edited fence type")
    };
    let before = session.document().clone();
    let prepared = command.prepare(&session, actor()).unwrap();
    assert_eq!(prepared.outputs.len(), 5);
    let native = Compiler::default()
        .compile(prepared.prepared.document(), session.revision())
        .unwrap();
    assert_eq!(native.snapshot().bodies.len(), 5);
    assert_eq!(session.document(), &before);
    let run = panel.run;
    session.commit(prepared.prepared).unwrap();
    let mut reopened = FencePanel::existing(&session, run).unwrap();
    assert!(reopened.symbol.active());
    assert!(
        reopened.types.is_empty(),
        "private construction is absent from the local picker"
    );
    let request = reopened.model_request(session.document(), true).unwrap();
    let ModelRequest::SymbolConstruction(command) = request else {
        panic!("Reopening must retain the pin")
    };
    assert_eq!(command.edit_id(), Some(run));
    let prepared = command.prepare(&session, actor()).unwrap();
    let (record, _) = multicad_core::symbols::architecture::architecture_instance_for_output(
        prepared.prepared.document(),
        panel.group,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        read_architecture_instance(prepared.prepared.document(), record)
            .unwrap()
            .output,
        ArchitectureOutput::Fence {
            run,
            group: panel.group
        }
    );
    let unrelated = Entity::new(
        "Unrelated project material",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [1, 2, 3, 255],
            section_hatch: None,
        },
    );
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "External source change".into(),
            edits: vec![Mutation::Insert(unrelated)],
        })
        .unwrap();
    session.commit(prepared).unwrap();
    reopened.sync_source(&session);
    assert!(
        reopened.model_request(session.document(), true).is_err(),
        "a stale inspector cannot overwrite concurrent source"
    );
    panel.sync_source(&session);
}
