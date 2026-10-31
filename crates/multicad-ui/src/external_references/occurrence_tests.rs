//! Detached transport/form fixtures test guards and explicit UI choices. Native
//! curve/pose accuracy belongs to the compiler/kernel and native host suites.
use super::{
    controller::Controller,
    queries::{Queries, Task},
    *,
};
use crate::{egui, viewport::Transform};
use multicad_core::{
    commands::{Actor, Channel, PreparationSnapshot, Request, Session},
    document::{EntityId, Mutation},
    model::{OccurrencePath, RigidPose},
    project_file::{ContentHash, ProjectResources},
    references::ReferenceReport,
    sketch_external::{
        self, Behavior, ExternalBranch, ExternalCatalog, ExternalChoice, ExternalGeometry,
        ExternalGeometryFamily, ExternalQuery, OccurrenceCaptureRequest, OccurrenceSource,
        QueryMethod, ResultSelector,
    },
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default)]
pub(super) struct State {
    next: u64,
    tasks: Vec<Task>,
    replies: VecDeque<ExternalQueryReply>,
    cancelled: usize,
}
struct Backend(Arc<Mutex<State>>);
impl ExternalQueryBackend for Backend {
    fn request_catalog(
        &mut self,
        _: PreparationSnapshot,
        _: ProjectResources,
        query: ExternalQuery,
    ) -> Result<u64, String> {
        let mut state = self.0.lock().unwrap();
        state.next += 1;
        state.tasks.push(Task::Catalog(query));
        Ok(state.next)
    }
    fn request_prepare(
        &mut self,
        _: PreparationSnapshot,
        _: ProjectResources,
        _: Actor,
        input: OccurrenceCaptureRequest,
        repair: bool,
    ) -> Result<u64, String> {
        let mut state = self.0.lock().unwrap();
        state.next += 1;
        state.tasks.push(Task::Prepare { input, repair });
        Ok(state.next)
    }
    fn request_remove(
        &mut self,
        _: PreparationSnapshot,
        _: ProjectResources,
        _: Actor,
        sketch: EntityId,
        reference: EntityId,
    ) -> Result<u64, String> {
        let mut state = self.0.lock().unwrap();
        state.next += 1;
        state.tasks.push(Task::Remove { sketch, reference });
        Ok(state.next)
    }
    fn poll(&mut self) -> Option<ExternalQueryReply> {
        self.0.lock().unwrap().replies.pop_front()
    }
    fn cancel(&mut self) {
        self.0.lock().unwrap().cancelled += 1;
    }
}
pub(super) fn backend() -> (Box<dyn ExternalQueryBackend>, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    (Box::new(Backend(state.clone())), state)
}
pub(super) fn catalog_fixture(count: usize) -> (super::tests::Fixture, ExternalCatalog) {
    let fixture = super::tests::fixture();
    let scene = super::tests::scene(&fixture, 1);
    let hit = super::tests::hit(&fixture, &scene, false);
    let query = ExternalQuery {
        reference: EntityId::new(),
        sketch: fixture.sketch,
        source: OccurrenceSource {
            component: fixture.source.body,
            source: fixture.source.body,
            path: OccurrencePath {
                root: fixture.source.body,
                members: vec![],
            },
        },
        sketch_context: None,
        intent: hit.intent.unwrap(),
        method: QueryMethod::Coplanar,
    };
    let results = (0..count)
        .map(|index| ExternalChoice {
            selector: ResultSelector {
                support: query.intent.origins.iter().cloned().collect(),
                family: ExternalGeometryFamily::Line,
                branch: Some(ExternalBranch {
                    source_interval: [index as f64, index as f64 + 0.5],
                    crossing: None,
                }),
                expected_count: count,
            },
            geometry: ExternalGeometry::Line {
                a_mm: [index as f64 * 20., 0.],
                b_mm: [index as f64 * 20. + 10., 0.],
            },
            source_parameter: Some(index as f64 + 0.25),
            source_interval: None,
            tangent: false,
        })
        .collect();
    let catalog = ExternalCatalog {
        document: fixture.session.document().id(),
        revision: fixture.session.revision(),
        source_hash: ContentHash::document(fixture.session.document()).unwrap(),
        query_hash: query.hash().unwrap(),
        query,
        results,
        coincident: false,
        diagnostics: vec![],
        sketch_world_pose: RigidPose::default(),
    };
    catalog.validate().unwrap();
    (fixture, catalog)
}
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
#[test]
fn exact_query_echo_session_and_resources_all_guard_detached_catalogue_adoption() {
    let (fixture, catalog) = catalog_fixture(1);
    let before = fixture.session.document().clone();
    for corruption in 0..4 {
        let (backend, state) = backend();
        let mut queries = Queries::default();
        queries.set_backend(backend);
        let resources = ProjectResources::default();
        queries
            .begin(
                &fixture.session,
                &resources,
                actor(),
                Task::Catalog(catalog.query.clone()),
            )
            .unwrap();
        let mut result = catalog.clone();
        let mut echo = catalog.query.clone();
        match corruption {
            0 => result.source_hash = ContentHash::of(b"foreign"),
            1 => result.revision += 1,
            2 => echo.reference = EntityId::new(),
            _ => result.query_hash = ContentHash::of(b"wrong query"),
        }
        state
            .lock()
            .unwrap()
            .replies
            .push_back(ExternalQueryReply::Catalog {
                request: 1,
                query: echo,
                result: Ok(Box::new(result)),
            });
        assert!(queries.poll(&fixture.session, &resources).unwrap().is_err());
        assert!(!queries.pending());
        assert_eq!(fixture.session.document(), &before);
    }
    let (backend, state) = backend();
    let mut queries = Queries::default();
    queries.set_backend(backend);
    let resources = ProjectResources::default();
    queries
        .begin(
            &fixture.session,
            &resources,
            actor(),
            Task::Catalog(catalog.query.clone()),
        )
        .unwrap();
    let mut changed = resources.clone();
    changed
        .metadata
        .insert("changed".into(), serde_json::json!(true));
    assert!(queries.poll(&fixture.session, &changed).unwrap().is_err());
    assert!(!queries.pending());
    state
        .lock()
        .unwrap()
        .replies
        .push_back(ExternalQueryReply::Catalog {
            request: 1,
            query: catalog.query.clone(),
            result: Ok(Box::new(catalog)),
        });
    assert!(queries.poll(&fixture.session, &resources).is_none());
}
#[test]
fn same_document_hash_and_revision_from_a_foreign_session_cannot_adopt_a_query_reply() {
    let (fixture, catalog) = catalog_fixture(1);
    let original_name = fixture.session.document().name().to_string();
    let before = fixture
        .session
        .document()
        .edited(&[Mutation::RenameDocument("Before foreign".into())])
        .unwrap();
    let mut foreign = Session::new(before).unwrap();
    foreign
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Restore name".into(),
            edits: vec![Mutation::RenameDocument(original_name)],
        })
        .unwrap();
    assert_eq!(foreign.revision(), fixture.session.revision());
    assert_eq!(foreign.document(), fixture.session.document());
    let (backend, state) = backend();
    let mut queries = Queries::default();
    queries.set_backend(backend);
    queries
        .begin(
            &fixture.session,
            &Default::default(),
            actor(),
            Task::Catalog(catalog.query.clone()),
        )
        .unwrap();
    state
        .lock()
        .unwrap()
        .replies
        .push_back(ExternalQueryReply::Catalog {
            request: 1,
            query: catalog.query.clone(),
            result: Ok(Box::new(catalog)),
        });
    assert!(
        queries
            .poll(&foreign, &Default::default())
            .unwrap()
            .is_err()
    );
    assert!(!queries.pending());
}
#[test]
fn echoed_prepare_input_cannot_hide_a_different_canonical_binding_in_its_candidate() {
    let (fixture, mut catalog) = catalog_fixture(1);
    let scene = super::tests::scene(&fixture, 1);
    let mut legacy = Controller::default();
    legacy.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(scene.clone()),
        Transform::default(),
    );
    legacy
        .begin(&fixture.session, ExternalTool::Project)
        .unwrap();
    legacy
        .select(&fixture.session, super::tests::hit(&fixture, &scene, false))
        .unwrap();
    let proposal = legacy.prepare(&fixture.session, Channel::Ui).unwrap();
    catalog.query.reference = proposal.reference;
    catalog.query_hash = catalog.query.hash().unwrap();
    let expected = OccurrenceCaptureRequest::new(
        &catalog,
        &catalog.results[0].selector,
        Behavior::ReferenceOnly,
    )
    .unwrap();
    let mut actual = expected.clone();
    actual.behavior = Behavior::Derived;
    let changes = sketch_external::occurrence_capture_edits(
        fixture.session.document(),
        fixture.session.revision(),
        &actual,
        &catalog,
        false,
    )
    .unwrap();
    let preview = fixture
        .session
        .prepare(Request {
            expected_revision: fixture.session.revision(),
            actor: actor(),
            label: "Forged echo".into(),
            edits: changes,
        })
        .unwrap();
    let mut edit = proposal.edit;
    edit.preview = Some(preview);
    let (backend, state) = backend();
    let mut queries = Queries::default();
    queries.set_backend(backend);
    queries
        .begin(
            &fixture.session,
            &Default::default(),
            actor(),
            Task::Prepare {
                input: expected.clone(),
                repair: false,
            },
        )
        .unwrap();
    state
        .lock()
        .unwrap()
        .replies
        .push_back(ExternalQueryReply::Prepared {
            request: 1,
            source_hash: catalog.source_hash,
            input: expected,
            repair: false,
            result: Ok(Box::new(ExternalPrepared {
                reference: catalog.query.reference,
                edit,
                report: ReferenceReport {
                    revision: fixture.session.revision(),
                    entries: vec![],
                    sketch_supports: Default::default(),
                    external_results: Default::default(),
                    external_errors: Default::default(),
                },
            })),
        });
    let error = queries
        .poll(&fixture.session, &Default::default())
        .unwrap()
        .err()
        .expect("Forged canonical request rejected");
    assert!(error.contains("canonical query"), "{error}");
    assert!(
        fixture
            .session
            .document()
            .get(catalog.query.reference)
            .is_none()
    );
}
fn controller_with_catalog(
    count: usize,
) -> (
    super::tests::Fixture,
    Controller,
    Arc<Mutex<State>>,
    ExternalCatalog,
) {
    let (fixture, catalog) = catalog_fixture(count);
    let mut controller = Controller::default();
    controller.refresh(
        &fixture.session,
        Some(fixture.sketch),
        Some(super::tests::scene(&fixture, 1)),
        Transform::from_rigid_pose(RigidPose {
            translation_mm: [120., 0., 0.],
            ..Default::default()
        })
        .unwrap(),
    );
    let (backend, state) = backend();
    controller.set_query_backend(backend);
    controller
        .begin(&fixture.session, ExternalTool::Project)
        .unwrap();
    let form = controller.active.as_mut().unwrap().form.as_mut().unwrap();
    form.reference = catalog.query.reference;
    form.source = Some(catalog.query.source.clone());
    form.intent = Some(catalog.query.intent.clone());
    (fixture, controller, state, catalog)
}
pub(super) fn frame(
    ctx: &egui::Context,
    controller: &mut Controller,
    session: &Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            focused: true,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300., 1000.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            assert!(
                controller
                    .inspector(
                        ui,
                        session,
                        controller.context.as_ref().unwrap().sketch,
                        None,
                        true,
                        &Default::default()
                    )
                    .is_none()
            );
        },
    );
    output.textures_delta.clear();
    output
}
pub(super) fn text_center(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
    fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.size() * 0.5),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, text))
        .unwrap_or_else(|| panic!("Visible control {text}"))
}
pub(super) fn click(
    ctx: &egui::Context,
    controller: &mut Controller,
    session: &Session,
    point: egui::Pos2,
) {
    for pressed in [true, false] {
        frame(
            ctx,
            controller,
            session,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
#[test]
fn multiple_results_need_a_real_choice_and_world_context_overrides_old_auto_placement() {
    let (fixture, mut controller, state, catalog) = controller_with_catalog(2);
    assert_eq!(controller.editing_pose(), Some(Transform::default()));
    assert!(
        controller
            .request_preview(&fixture.session, &Default::default(), Channel::Ui)
            .unwrap()
            .is_none()
    );
    state
        .lock()
        .unwrap()
        .replies
        .push_back(ExternalQueryReply::Catalog {
            request: 1,
            query: catalog.query.clone(),
            result: Ok(Box::new(catalog.clone())),
        });
    assert!(
        controller
            .poll_query(&fixture.session, &Default::default())
            .is_none()
    );
    assert!(!controller.can_preview(&fixture.session));
    assert!(
        controller
            .active
            .as_ref()
            .unwrap()
            .form
            .as_ref()
            .unwrap()
            .selected
            .is_none()
    );
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut controller, &fixture.session, vec![]);
    click(
        &ctx,
        &mut controller,
        &fixture.session,
        text_center(&output, "Choose exact result"),
    );
    let output = frame(&ctx, &mut controller, &fixture.session, vec![]);
    click(
        &ctx,
        &mut controller,
        &fixture.session,
        text_center(&output, "2 · Line · 10.0000 mm"),
    );
    assert!(controller.can_preview(&fixture.session));
    let form = controller.active.as_ref().unwrap().form.as_ref().unwrap();
    assert_eq!(form.selected, Some(catalog.results[1].selector.clone()));
    let first = form
        .request(fixture.session.document(), Behavior::ReferenceOnly)
        .unwrap();
    let second = form
        .request(fixture.session.document(), Behavior::ReferenceOnly)
        .unwrap();
    assert_eq!(first.output, second.output);
    controller
        .request_preview(&fixture.session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(controller.query_pending());
    assert!(
        matches!(state.lock().unwrap().tasks.last(), Some(Task::Prepare { input, .. }) if input.selector == catalog.results[1].selector)
    );
    controller.cancel();
    assert!(!controller.active());
    assert_eq!(controller.editing_pose(), Some(Transform::default()));
    assert!(
        fixture
            .session
            .document()
            .get(catalog.query.reference)
            .is_none()
    );
}
#[test]
fn empty_catalogue_is_honest_and_resource_changes_revoke_completed_choices() {
    for count in [0, 1] {
        let (fixture, mut controller, state, catalog) = controller_with_catalog(count);
        controller
            .request_preview(&fixture.session, &Default::default(), Channel::Ui)
            .unwrap();
        state
            .lock()
            .unwrap()
            .replies
            .push_back(ExternalQueryReply::Catalog {
                request: 1,
                query: catalog.query.clone(),
                result: Ok(Box::new(catalog.clone())),
            });
        assert!(
            controller
                .poll_query(&fixture.session, &Default::default())
                .is_none()
        );
        if count == 1 {
            controller
                .active
                .as_mut()
                .unwrap()
                .form
                .as_mut()
                .unwrap()
                .select_result(&catalog.results[0].selector, Behavior::ReferenceOnly)
                .unwrap();
            assert!(controller.can_preview(&fixture.session));
        } else {
            assert!(!controller.can_preview(&fixture.session));
        }
        let mut changed = ProjectResources::default();
        changed
            .metadata
            .insert("updated".into(), serde_json::json!(1));
        controller.poll_query(&fixture.session, &changed);
        assert!(
            controller
                .active
                .as_ref()
                .unwrap()
                .form
                .as_ref()
                .unwrap()
                .catalog
                .is_none()
        );
        assert!(!controller.query_pending());
        assert!(
            fixture
                .session
                .document()
                .get(catalog.query.reference)
                .is_none()
        );
    }
}
