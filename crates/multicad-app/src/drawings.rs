//! Native drawing composition, sharing the bounded geometry worker while keeping
//! native solids and projection operations off the UI thread.
use multicad_core::{
    document::{Document, EntityId},
    drawings::{DrawingEvaluation, DrawingStamp},
    schedules::ScheduleSpec,
};
use multicad_model::{Compiler, DrawingProviders, ModelError, Runtime, schedules};
use multicad_ui::{
    drawings::{DrawingBackend, DrawingReply},
    schedules::{ScheduleReply, ScheduleTable},
};
use std::sync::Arc;
#[path = "drawings/cache.rs"]
mod cache;
type Tables = Vec<(ScheduleSpec, Result<ScheduleTable, String>)>;
pub struct Backend {
    pair_cycle_proofs: Option<multicad_model::PairCycleProofStore>,
    runtime: Result<Runtime<Arc<DrawingEvaluation>>, String>,
    expected: Option<(u64, DrawingStamp)>,
    /// Schedules run on their own worker, created on first use, so view
    /// previews and schedule checks never supersede each other.
    schedules: Option<Result<Runtime<Tables>, String>>,
    expected_schedules: Option<(u64, DrawingStamp, u64)>,
}
/// Every schedule named by the request mask, built on the model worker.
fn build_tables(document: &Document, model: &multicad_model::Model, mask: u64) -> Tables {
    let revision = model.snapshot().stamp.revision;
    ScheduleSpec::from_mask(mask)
        .into_iter()
        .map(|spec| {
            let table = schedules::build(model, document, revision, spec)
                .map(|s| ScheduleTable {
                    spec,
                    revision: s.revision,
                    table: s.placed_table(spec, [0.; 2]),
                })
                .map_err(|e: ModelError| e.message);
            (spec, table)
        })
        .collect()
}
/// Request tags identify pages without changing the canonical document/hash.
fn page_ids(document: &Document) -> Result<Vec<EntityId>, ModelError> {
    let mut pages = Vec::new();
    for id in multicad_core::drawings::books(document) {
        let book = multicad_core::drawings::read(document, id).map_err(|e| ModelError {
            entity: Some(id),
            message: e.to_string(),
        })?;
        pages.extend(book.pages.into_iter().map(|page| page.id));
    }
    Ok(pages)
}
fn page_for_tag(document: &Document, tag: u64) -> Result<EntityId, ModelError> {
    page_ids(document)?
        .get(tag.saturating_sub(1) as usize)
        .copied()
        .ok_or_else(|| ModelError {
            entity: None,
            message: "The selected drawing page is missing".into(),
        })
}
impl Backend {
    pub fn new() -> Self {
        Self::with_cache(cache::Cache::installed())
    }
    pub(crate) fn with_cache(cache: Option<cache::Cache>) -> Self {
        let reader = cache.clone();
        Self {
            pair_cycle_proofs: multicad_model::current_pair_cycle_proofs(),
            runtime: Runtime::with_scoped_cached_model_publisher(
                Compiler {
                    omit_display: true,
                    ..Default::default()
                },
                move |document, revision, tag| {
                    reader.as_ref()?.read(document, revision, tag).map(Arc::new)
                },
                |document, tag| {
                    if tag == 0 {
                        return None;
                    }
                    multicad_model::drawing_preview_sources(
                        document,
                        page_for_tag(document, tag).ok()?,
                    )
                },
                move |document, model, tag| {
                    let revision = model.snapshot().stamp.revision;
                    let providers = DrawingProviders::default();
                    let result = if tag == 0 {
                        model.drawings(document, revision, &providers)
                    } else {
                        let page = page_for_tag(document, tag)?;
                        model.drawing_page_preview(document, revision, &providers, page)
                    }?;
                    if let Some(cache) = &cache {
                        cache.write(document, tag, &result);
                    }
                    Ok(Arc::new(result))
                },
            ),
            expected: None,
            schedules: None,
            expected_schedules: None,
        }
    }
}
impl DrawingBackend for Backend {
    fn cancel(&mut self) {
        self.expected = None;
        if let Ok(runtime) = self.runtime.as_mut() {
            runtime.cancel();
        }
    }
    fn request(&mut self, document: &Document, stamp: DrawingStamp) -> Result<(), String> {
        stamp.check(document, stamp.revision)?;
        let runtime = self.runtime.as_mut().map_err(|e| e.clone())?;
        let generation = runtime.request(document, stamp.revision)?;
        self.expected = Some((generation, stamp));
        Ok(())
    }
    fn request_page(
        &mut self,
        document: &Document,
        stamp: DrawingStamp,
        page: EntityId,
    ) -> Result<(), String> {
        stamp.check(document, stamp.revision)?;
        let index = page_ids(document)
            .map_err(|e| e.to_string())?
            .iter()
            .position(|id| *id == page)
            .ok_or("The selected drawing page is missing")?;
        let runtime = self.runtime.as_mut().map_err(|e| e.clone())?;
        let generation = runtime.request_tagged(document, stamp.revision, index as u64 + 1)?;
        self.expected = Some((generation, stamp));
        Ok(())
    }
    fn request_schedules(
        &mut self,
        document: &Document,
        stamp: DrawingStamp,
        specs: &[ScheduleSpec],
    ) -> Result<(), String> {
        stamp.check(document, stamp.revision)?;
        let runtime = self
            .schedules
            .get_or_insert_with(|| {
                let create = || {
                    Runtime::with_model_publisher(Compiler::default(), |document, model, mask| {
                        Ok(build_tables(document, model, mask))
                    })
                };
                if let Some(store) = &self.pair_cycle_proofs {
                    multicad_model::with_pair_cycle_proofs(store.clone(), create)
                } else {
                    create()
                }
            })
            .as_mut()
            .map_err(|e| e.clone())?;
        let mask = ScheduleSpec::mask(specs.iter().copied());
        let generation = runtime.request_tagged(document, stamp.revision, mask)?;
        self.expected_schedules = Some((generation, stamp, mask));
        Ok(())
    }
    fn poll_schedules(&mut self) -> Option<ScheduleReply> {
        let (generation, stamp, mask) = self.expected_schedules?;
        let reply = self.schedules.as_mut()?.as_mut().ok()?.poll()?;
        if reply.generation != generation {
            return None;
        }
        self.expected_schedules = None;
        // A model that fails to build fails every requested schedule.
        let tables = reply.result.unwrap_or_else(|error| {
            ScheduleSpec::from_mask(mask)
                .into_iter()
                .map(|spec| (spec, Err(error.message.clone())))
                .collect()
        });
        Some(ScheduleReply { stamp, tables })
    }
    fn poll(&mut self) -> Option<DrawingReply> {
        let (generation, stamp) = self.expected?;
        let reply = self.runtime.as_mut().ok()?.poll()?;
        if reply.generation != generation {
            return None;
        }
        self.expected = None;
        let result = reply.result.map_err(|e| e.to_string()).and_then(|result| {
            if result.stamp == stamp {
                Ok(result)
            } else {
                Err("Native drawing result does not match the requested source".into())
            }
        });
        Some(DrawingReply { stamp, result })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        drawings::{self, AnnotationKind},
        model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
        schedules::ScheduleKind,
    };
    use multicad_ui::drawings::{DrawingEditor, DrawingTool, ScheduleStatus};
    use std::time::{Duration, Instant};

    fn preview(backend: &mut Backend, session: &Session, page: EntityId) -> Arc<DrawingEvaluation> {
        let stamp = DrawingStamp {
            document: session.document().id(),
            revision: session.revision(),
            source_hash: multicad_core::project_file::ContentHash::document(session.document())
                .unwrap(),
        };
        backend
            .request_page(session.document(), stamp, page)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(reply) = backend.poll() {
                return reply.result.unwrap();
            }
            assert!(Instant::now() < deadline, "Drawing preview timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn open_example(name: &str) -> Session {
        use multicad_ui::application::DocumentStorage;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name);
        let loaded = crate::files::ProjectStore::default()
            .open_project(&path)
            .unwrap();
        Session::new(loaded.file.document).unwrap()
    }

    #[test]
    fn cold_tool_case_preview_builds_only_the_drawn_parts() {
        let session = open_example("werkzeugkoffer-2.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = &drawing.pages[0];
        assert_eq!(page.number, "WK-101");
        let mut backend = Backend::with_cache(None);
        let start = Instant::now();
        let evaluation = preview(&mut backend, &session, page.id);
        let elapsed = start.elapsed();
        assert_eq!(
            evaluation
                .checked_page(document, session.revision(), book, page.id)
                .unwrap()
                .len(),
            2
        );
        let evaluated = backend
            .runtime
            .as_ref()
            .unwrap()
            .last_compile_stats()
            .unwrap()
            .evaluated;
        let unrelated = document
            .entities()
            .find(|e| {
                e.name.contains("M4 × 12 countersunk · fixed slide")
                    && matches!(e.data, multicad_core::document::EntityData::Feature { .. })
            })
            .unwrap();
        assert!(!evaluated.contains(&unrelated.id));
        eprintln!(
            "Cold WK-101 backend preview: {elapsed:.3?}, {} native features evaluated",
            evaluated.len()
        );
    }

    #[test]
    fn cold_apartment_plan_preview_keeps_its_native_dimensions() {
        let session = open_example("apartment.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = &drawing.pages[0];
        assert!(multicad_model::drawing_preview_sources(document, page.id).is_none());
        let mut backend = Backend::with_cache(None);
        let start = Instant::now();
        let evaluation = preview(&mut backend, &session, page.id);
        eprintln!(
            "Cold apartment {} backend preview: {:.3?}",
            page.number,
            start.elapsed()
        );
        assert!(!evaluation.annotations.is_empty());
        assert_eq!(
            evaluation
                .checked_page(document, session.revision(), book, page.id)
                .unwrap()
                .len(),
            page.views.len()
        );
    }

    #[test]
    fn persisted_native_page_matches_fresh_vectors_and_rebuilds_after_corruption_or_view_edits() {
        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let mut session = Session::new(Document::new("Native cached drawing").unwrap()).unwrap();
        add_box(&mut session, "Panel", 0.);
        let source = session
            .document()
            .entities()
            .find(|e| matches!(e.data, multicad_core::document::EntityData::Part { .. }))
            .unwrap()
            .id;
        let mut page = drawings::Page::new("1");
        page.views.push(drawings::DrawingView {
            id: EntityId::new(),
            name: "Front".into(),
            source: drawings::ViewSource::Model {
                entities: vec![source],
            },
            frame: multicad_core::references::ReferenceFrame {
                origin_mm: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
            kind: drawings::ViewKind::Projection,
            position_mm: [50., 50.],
            scale: drawings::Scale {
                numerator: 1,
                denominator: 1,
            },
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.01,
            guide: None,
            equipment_categories: None,
        });
        let (book, prepared) = drawings::prepare_create(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            "Drawing",
            Some(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let mut first = Backend::with_cache(Some(cache.clone()));
        let original = preview(&mut first, &session, page.id);
        assert!(
            original
                .views
                .values()
                .all(|v| matches!(v, drawings::ViewEvaluation::Ready { .. }))
        );
        drop(first);
        let mut reopened = Backend::with_cache(Some(cache.clone()));
        assert_eq!(original, preview(&mut reopened, &session, page.id));
        let entry = std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(entry, b"corrupt optional cache").unwrap();
        let mut rebuilt = Backend::with_cache(Some(cache.clone()));
        assert_eq!(original, preview(&mut rebuilt, &session, page.id));
        page.views[0].hidden_lines = true;
        let prepared = drawings::prepare(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            book,
            drawings::PageAction::Replace(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let actual = preview(&mut rebuilt, &session, page.id);
        let model = Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let expected = model
            .drawing_page(
                session.document(),
                session.revision(),
                &DrawingProviders::default(),
                page.id,
            )
            .unwrap();
        assert_eq!(*actual, expected);
        assert_ne!(actual.stamp.source_hash, original.stamp.source_hash);
        assert_ne!(actual.views, original.views);

        // The selected Part may become a Boolean tool in another Part's
        // history. A scoped model of the drawing alone cannot see that use.
        let target = add_box(&mut session, "Receiver", 5.);
        let producer = match &session.document().get(source).unwrap().data {
            multicad_core::document::EntityData::Part { features, .. } => features[0],
            _ => unreachable!(),
        };
        let cut = model::BooleanRequest::new(
            model::Operation::Cut,
            target,
            vec![model::BodyInput {
                body: source,
                producer,
            }],
        );
        let prepared = model::prepare_boolean(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &cut,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        assert!(multicad_model::drawing_preview_sources(session.document(), page.id).is_none());
        let consumed = preview(&mut rebuilt, &session, page.id);
        assert!(
            matches!(&consumed.views[&page.views[0].id], drawings::ViewEvaluation::Unresolved { message } if message.contains("consumed"))
        );
    }

    fn add_box(session: &mut Session, name: &str, x: f64) -> model::BodyInput {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [length(20.), length(10.), length(5.)],
                frame: PrimitiveFrame {
                    origin: [length(x), length(0.), length(0.)],
                    ..Default::default()
                },
            },
        );
        let p = model::prepare_primitive(
            session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        session.commit(p.prepared).unwrap();
        model::BodyInput {
            body: request.body,
            producer: request.feature,
        }
    }
    fn until(
        editor: &mut DrawingEditor,
        session: &mut Session,
        done: impl Fn(&DrawingEditor, &Session) -> bool,
    ) {
        let end = Instant::now() + Duration::from_secs(20);
        while !done(editor, session) {
            assert!(Instant::now() < end, "native schedule timeout");
            editor.tick_schedules(session);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn fresh_rows(session: &Session, spec: ScheduleSpec) -> Vec<Vec<String>> {
        let model = Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        schedules::build(&model, session.document(), session.revision(), spec)
            .unwrap()
            .rows
    }
    fn placed(session: &Session) -> Vec<Vec<String>> {
        let book = drawings::books(session.document()).next().unwrap();
        let page = drawings::read(session.document(), book).unwrap().pages[0].clone();
        page.annotations
            .iter()
            .find_map(|a| match &a.kind {
                AnnotationKind::Table { rows, .. } => Some(rows.clone()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn native_backend_places_checks_and_refreshes_a_real_bom_table() {
        let mut session = Session::new(Document::new("Native schedules").unwrap()).unwrap();
        add_box(&mut session, "Leg", 0.);
        add_box(&mut session, "Top", 40.);
        let mut editor = DrawingEditor::default();
        editor.set_backend_factory(Arc::new(|| Box::new(Backend::new())));
        editor
            .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
            .unwrap();
        editor
            .invoke(&mut session, DrawingTool::ScheduleTable, Channel::Ui)
            .unwrap();
        let spec = ScheduleSpec::new(ScheduleKind::Bom);
        let revision = session.revision();
        editor
            .place_schedule(&mut session, spec, Channel::Ui)
            .unwrap();
        until(&mut editor, &mut session, |_, s| s.revision() > revision);
        assert_eq!(placed(&session), fresh_rows(&session, spec));
        assert_eq!(placed(&session).len(), 2);
        let table = editor.selected_annotation().unwrap();
        until(&mut editor, &mut session, |e, s| {
            e.schedule_status(s, table) == ScheduleStatus::Current
        });

        add_box(&mut session, "Shelf", 80.);
        editor.sync(&session).unwrap();
        until(&mut editor, &mut session, |e, s| {
            matches!(e.schedule_status(s, table), ScheduleStatus::Stale(_))
        });
        editor
            .refresh_schedule_table(&mut session, table, Channel::Ui)
            .unwrap();
        assert_eq!(placed(&session).len(), 3);
        assert_eq!(placed(&session), fresh_rows(&session, spec));
        until(&mut editor, &mut session, |e, s| {
            e.schedule_status(s, table) == ScheduleStatus::Current
        });
    }
}
