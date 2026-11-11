//! Native drawing composition, sharing the bounded geometry worker while keeping
//! native solids and projection operations off the UI thread.
use multicad_core::{
    document::{Document, EntityId},
    drawings::{DrawingEvaluation, DrawingStamp},
    schedules::ScheduleSpec,
};
use multicad_model::{Compiler, DrawingProviders, GeometryBackend, ModelError, Runtime, schedules};
use multicad_ui::{
    drawings::{DrawingBackend, DrawingReply},
    schedules::{ScheduleReply, ScheduleTable},
};
use std::sync::Arc;
#[path = "drawings/cache.rs"]
mod cache;
type Tables = Vec<(ScheduleSpec, Result<ScheduleTable, String>)>;
pub struct Backend {
    geometry_backend: GeometryBackend,
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
    pub fn with_geometry_backend(geometry_backend: GeometryBackend) -> Self {
        Self::with_cache_and_backend(cache::Cache::installed(), geometry_backend)
    }
    pub(crate) fn with_cache(cache: Option<cache::Cache>) -> Self {
        Self::with_cache_and_backend(cache, GeometryBackend::OpenCascade)
    }
    pub(crate) fn with_cache_and_backend(
        cache: Option<cache::Cache>,
        geometry_backend: GeometryBackend,
    ) -> Self {
        let cache = cache.map(|cache| cache.for_backend(geometry_backend));
        let reader = cache.clone();
        Self {
            geometry_backend,
            pair_cycle_proofs: multicad_model::current_pair_cycle_proofs(),
            runtime: Runtime::with_scoped_cached_model_publisher(
                Compiler {
                    omit_display: true,
                    ..Default::default()
                },
                geometry_backend,
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
                    Runtime::with_model_and_snapshot_backend(
                        Compiler {
                            omit_display: true,
                            ..Default::default()
                        },
                        self.geometry_backend,
                        |document, model, _, mask| Ok(build_tables(document, model, mask)),
                    )
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
        let session = open_example("toolcase.tcd");
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
    fn rust_worker_opens_and_restores_both_toolcase_side_views() {
        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let session = open_example("toolcase.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = drawing
            .pages
            .iter()
            .find(|page| page.number == "WK-101")
            .unwrap();

        let mut cold = Backend::with_cache_and_backend(Some(cache.clone()), GeometryBackend::Rust);
        let start = Instant::now();
        let original = preview(&mut cold, &session, page.id);
        let cold_elapsed = start.elapsed();
        assert_eq!(
            original
                .checked_page(document, session.revision(), book, page.id)
                .unwrap()
                .len(),
            2
        );
        for view in &page.views {
            let drawings::ViewEvaluation::Ready { vectors, .. } = &original.views[&view.id] else {
                panic!("{:?}", original.views[&view.id]);
            };
            assert_eq!(vectors.len(), 96);
        }
        assert!(
            cold.runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_some()
        );
        drop(cold);

        let mut reopened = Backend::with_cache_and_backend(Some(cache), GeometryBackend::Rust);
        let start = Instant::now();
        let restored = preview(&mut reopened, &session, page.id);
        let warm_elapsed = start.elapsed();
        assert_eq!(restored.views, original.views);
        assert!(
            reopened
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
        );
        eprintln!(
            "WK-101 Rust drawing worker cold={cold_elapsed:.3?}, restored={warm_elapsed:.3?}"
        );
    }

    #[test]
    fn rust_worker_opens_and_restores_the_saved_apartment_isometric() {
        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let session = open_example("apartment.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = drawing
            .pages
            .iter()
            .find(|page| page.number == "AP-102")
            .unwrap();

        let mut cold = Backend::with_cache_and_backend(Some(cache.clone()), GeometryBackend::Rust);
        let start = Instant::now();
        let original = preview(&mut cold, &session, page.id);
        let cold_elapsed = start.elapsed();
        let [view] = page.views.as_slice() else {
            panic!("Expected the saved apartment isometric view");
        };
        let drawings::ViewEvaluation::Ready { vectors, .. } = &original.views[&view.id] else {
            panic!("{:?}", original.views[&view.id]);
        };
        assert_eq!(vectors.len(), 212);
        assert!(
            original
                .checked_page(document, session.revision(), book, page.id)
                .is_ok()
        );
        drop(cold);

        let mut reopened = Backend::with_cache_and_backend(Some(cache), GeometryBackend::Rust);
        let start = Instant::now();
        let restored = preview(&mut reopened, &session, page.id);
        let warm_elapsed = start.elapsed();
        assert_eq!(restored.views, original.views);
        assert!(
            reopened
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
        );
        eprintln!(
            "AP-102 Rust drawing worker cold={cold_elapsed:.3?}, restored={warm_elapsed:.3?}"
        );
    }

    #[test]
    fn cold_apartment_plan_preview_keeps_its_native_dimensions() {
        let mut session = open_example("apartment.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let mut page = drawing.pages[0].clone();
        assert!(multicad_model::drawing_preview_sources(document, page.id).is_some());
        let mut backend = Backend::with_cache(None);
        let start = Instant::now();
        let evaluation = preview(&mut backend, &session, page.id);
        eprintln!(
            "Cold apartment {} backend preview: {:.3?}",
            page.number,
            start.elapsed()
        );
        assert_eq!(evaluation.annotations.len(), 8);
        assert!(
            evaluation.annotations.values().all(|annotation| matches!(
                annotation,
                drawings::AnnotationEvaluation::Ready { .. }
            ))
        );
        assert_eq!(
            evaluation
                .checked_page(document, session.revision(), book, page.id)
                .unwrap()
                .len(),
            page.views.len()
        );
        let compiler = Compiler {
            omit_display: true,
            ..Default::default()
        };
        let providers = DrawingProviders::default();
        let revision = session.revision();
        let model = compiler.compile(document, revision).unwrap();
        let started = Instant::now();
        for view in &page.views {
            assert_eq!(
                model
                    .drawing_view(document, revision, view, &providers)
                    .unwrap(),
                evaluation.views[&view.id]
            );
        }
        let projection = started.elapsed();
        let started = Instant::now();
        for annotation in &page.annotations {
            if let Some(actual) = evaluation.annotations.get(&annotation.id) {
                assert_eq!(&model.dimension(&page, annotation), actual);
            }
        }
        eprintln!(
            "Full model compile: {:.0}ms, projection: {projection:.3?}, dimensions: {:.3?}",
            model.timings().total_ms,
            started.elapsed()
        );

        let (detached, reference) = page
            .annotations
            .iter_mut()
            .find_map(|annotation| {
                let AnnotationKind::Dimension { measure, .. } = &mut annotation.kind else {
                    return None;
                };
                let drawings::DimensionMeasure::Linear { a, .. } = measure.as_mut() else {
                    return None;
                };
                Some((annotation.id, a))
            })
            .unwrap();
        reference.intent.origins = [multicad_core::references::Origin {
            source: reference.source,
            role: "removed drawing face".into(),
        }]
        .into();
        reference.intent.attachment = None;
        let prepared = drawings::prepare(
            &session,
            revision,
            Actor::local(Channel::Ui),
            book,
            drawings::PageAction::Replace(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let document = session.document();
        let revision = session.revision();
        assert!(multicad_model::drawing_preview_sources(document, page.id).is_some());
        let actual = preview(&mut backend, &session, page.id);
        let full = compiler
            .compile(document, revision)
            .unwrap()
            .drawing_page(document, revision, &providers, page.id)
            .unwrap();
        assert!(matches!(
            full.annotations[&detached],
            drawings::AnnotationEvaluation::Detached { .. }
        ));
        assert_eq!(actual.annotations, full.annotations);
    }

    #[test]
    fn cold_expanded_apartment_plan_preview() {
        use multicad_core::document::EntityData;
        let mut session = open_example("apartment.tcd");
        let book = drawings::books(session.document()).next().unwrap();
        let mut page = drawings::read(session.document(), book).unwrap().pages[0].clone();
        let drawings::ViewSource::Model { entities } = &mut page.views[0].source else {
            unreachable!("the apartment plan uses model sources");
        };
        let original_count = entities.len();
        let walls: Vec<_> = entities
            .iter()
            .filter_map(|&body| {
                let EntityData::Part { features, .. } = &session.document().get(body)?.data else {
                    return None;
                };
                let producer = *features.last()?;
                matches!(&session.document().get(producer)?.data, EntityData::Feature { definition, .. }
                    if definition.type_id == model::WALL_TYPE)
                    .then_some(model::BodyInput { body, producer })
            })
            .collect();
        assert!(walls.len() >= 4, "apartment exterior walls are missing");
        // Duplicate a wing 12 m to the east. Both wings project on the same
        // sheet. The original eight dimensions and its furniture stay intact.
        let copies: Vec<_> = walls
            .iter()
            .cloned()
            .map(|source| {
                model::InstanceCopyRequest::new(
                    model::InstanceCopySource::Part { source },
                    model::NumericTransform {
                        translation: [length(12_000.), length(0.), length(0.)],
                        ..Default::default()
                    },
                )
            })
            .collect();
        let actor = Actor::local(Channel::Ui);
        let prepared =
            model::prepare_instance_copies(&session, session.revision(), actor.clone(), &copies)
                .unwrap();
        entities.extend(&prepared.instances);
        session.commit(prepared.prepared).unwrap();
        page.views[0].scale.denominator = 100;
        page.views[0].frame.origin_mm[0] += 6_000.;
        let prepared = drawings::prepare(
            &session,
            session.revision(),
            actor,
            book,
            drawings::PageAction::Replace(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();

        let mut backend = Backend::with_cache(None);
        let started = Instant::now();
        let result = preview(&mut backend, &session, page.id);
        let elapsed = started.elapsed();
        assert!(
            matches!(&result.views[&page.views[0].id], drawings::ViewEvaluation::Ready { vectors, .. } if !vectors.is_empty())
        );
        assert_eq!(result.annotations.len(), 8);
        assert!(
            result.annotations.values().all(|annotation| matches!(
                annotation,
                drawings::AnnotationEvaluation::Ready { .. }
            ))
        );
        eprintln!(
            "Expanded AP-101 cold backend preview: {elapsed:.3?}, {original_count} original sources plus {} placed walls",
            walls.len()
        );
    }

    fn projection_page(source: EntityId) -> drawings::Page {
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
        page
    }

    #[test]
    fn persisted_native_page_matches_fresh_vectors_and_rebuilds_after_corruption_or_view_edits() {
        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let mut session = Session::new(Document::new("Native cached drawing").unwrap()).unwrap();
        let source = add_box(&mut session, "Panel", 0.).body;
        let mut page = projection_page(source);
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

    #[test]
    fn rust_drawing_worker_projects_scoped_exact_geometry_and_reuses_its_publication() {
        use drawings::{VectorCurve, VectorRole, ViewEvaluation};
        use multicad_core::document::{Definition, Entity, EntityData, Mutation};

        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let mut session = Session::new(Document::new("Rust drawing").unwrap()).unwrap();
        let source = add_box(&mut session, "Panel", 0.);
        let unrelated = Entity::new(
            "An unsupported feature outside the requested view",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.drawing_scope".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let document = session
            .document()
            .edited(&[Mutation::Insert(unrelated)])
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let mut page = projection_page(source.body);
        let cut_id = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: cut_id,
            name: "Bounded section".into(),
            kind: drawings::ViewKind::Section {
                offset_mm: 4.,
                range: drawings::SectionRange::Depth { depth_mm: 3. },
                window: drawings::SectionWindow {
                    u_mm: Some([3., 13.]),
                    v_mm: Some([1., 9.]),
                },
            },
            ..page.views[0].clone()
        });
        let (_, prepared) = drawings::prepare_create(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            "Drawing",
            Some(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        assert!(
            Compiler::default()
                .compile_with_backend(
                    session.document(),
                    session.revision(),
                    GeometryBackend::Rust,
                    None,
                )
                .is_err()
        );

        let mut first = Backend::with_cache_and_backend(Some(cache.clone()), GeometryBackend::Rust);
        let original = preview(&mut first, &session, page.id);
        let ViewEvaluation::Ready { vectors, .. } = &original.views[&page.views[0].id] else {
            panic!("{:?}", original.views);
        };
        assert_eq!(vectors.len(), 4);
        assert!(vectors.iter().all(|vector| {
            vector.source == source.body
                && vector.role == VectorRole::Visible
                && matches!(vector.curve, VectorCurve::Line { a, b }
                    if [a, b].into_iter().all(|[x, y]| [0., 20.].contains(&x) && [0., 10.].contains(&y)))
        }));
        let ViewEvaluation::Ready { vectors, .. } = &original.views[&cut_id] else {
            panic!("{:?}", original.views[&cut_id]);
        };
        let section: Vec<_> = vectors
            .iter()
            .filter(|vector| vector.role == VectorRole::Section)
            .collect();
        assert_eq!(section.len(), 4);
        assert!(section.iter().all(|vector| {
            vector.source == source.body
                && matches!(vector.curve, VectorCurve::Line { a, b }
                    if [a, b].into_iter().all(|[x, y]| [3., 13.].contains(&x) && [1., 9.].contains(&y)))
        }));
        assert_eq!(
            first
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .unwrap()
                .evaluated,
            [source.producer].into(),
        );
        let mut reopened = Backend::with_cache_and_backend(Some(cache), GeometryBackend::Rust);
        assert_eq!(original, preview(&mut reopened, &session, page.id));
        assert!(
            reopened
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
        );
    }

    #[test]
    fn rust_drawing_worker_resolves_sphere_and_stock_visibility() {
        use drawings::{VectorCurve, VectorRole, ViewEvaluation};

        let mut session =
            Session::new(Document::new("Spherical assembly drawing").unwrap()).unwrap();
        let stock = add_box(&mut session, "Stock", 0.).body;
        let sphere = PrimitiveRequest::new(
            "Sphere",
            PrimitiveParameters::Sphere {
                radius: length(4.),
                frame: PrimitiveFrame {
                    origin: [10., 0., 11.].map(length),
                    ..Default::default()
                },
            },
        );
        let actor = Actor::local(Channel::Ui);
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor.clone(), &sphere).unwrap();
        session.commit(prepared.prepared).unwrap();

        let mut page = projection_page(stock);
        let projection = &mut page.views[0];
        projection.source = drawings::ViewSource::Model {
            entities: vec![stock, sphere.body],
        };
        projection.hidden_lines = true;
        let id = projection.id;
        let mut section = projection.clone();
        section.id = EntityId::new();
        section.name = "Sphere depth section".into();
        section.kind = drawings::ViewKind::Section {
            offset_mm: 12.,
            range: drawings::SectionRange::Depth { depth_mm: 9. },
            window: drawings::SectionWindow {
                u_mm: Some([6.5, 14.5]),
                v_mm: Some([-3.5, 4.5]),
            },
        };
        let section_id = section.id;
        page.views.push(section);
        let (_, prepared) = drawings::prepare_create(
            &session,
            session.revision(),
            actor,
            "Drawing",
            Some(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let mut backend = Backend::with_cache_and_backend(None, GeometryBackend::Rust);
        let result = preview(&mut backend, &session, page.id);
        let ViewEvaluation::Ready { vectors, .. } = &result.views[&id] else {
            panic!("{:?}", result.views[&id]);
        };
        assert!(vectors.iter().any(|v| v.source == sphere.body
            && v.role == VectorRole::Visible
            && matches!(
                v.curve,
                VectorCurve::Ellipse {
                    center: [10., 0.],
                    major_mm: 4.,
                    minor_mm: 4.,
                    ..
                }
            )));
        assert!(vectors.iter().any(|v| v.source == stock
            && v.role == VectorRole::Hidden
            && matches!(v.curve, VectorCurve::Line { a,b }
                if [a,b].into_iter().all(|p| p[1].abs() < 1e-7)
                    && (a[0].min(b[0])-6.).abs() < 1e-6 && (a[0].max(b[0])-14.).abs() < 1e-6)));
        let ViewEvaluation::Ready { vectors, .. } = &result.views[&section_id] else {
            panic!("{:?}", result.views[&section_id]);
        };
        assert!(vectors.iter().any(|v| v.source == sphere.body
            && v.role == VectorRole::Section
            && matches!(v.curve, VectorCurve::Ellipse { major_mm, .. }
                if (major_mm - 15_f64.sqrt()).abs() < 1e-8)));
        assert!(
            vectors
                .iter()
                .any(|v| v.source == stock && v.role == VectorRole::Hidden)
        );
    }

    #[test]
    fn rust_drawing_worker_keeps_the_torus_bore_open_in_an_assembly() {
        use drawings::{VectorCurve, VectorRole, ViewEvaluation};
        use multicad_core::{
            constraint_sketch::{Curve, CurveKind, source::Source},
            sketch::Datum,
            sketch_regions,
        };

        let (sketch, edits) = Source::create(Datum::Xz, "Torus section").unwrap();
        let document = Document::new("Torus drawing")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let mut source = Source::read(&document, sketch).unwrap().0;
        let center = source.add_point([5., 0.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 1.,
            },
        });
        let region = sketch_regions::extract(&source).regions[0].intent();
        let document = document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let actor = Actor::local(Channel::Ui);
        let torus = model::RevolveRequest::new(sketch, vec![region], Default::default());
        let torus_body = torus.outputs[0].body;
        let prepared =
            model::prepare_revolve(&session, session.revision(), actor.clone(), &torus).unwrap();
        session.commit(prepared.prepared).unwrap();
        let stock = PrimitiveRequest::new(
            "Stock",
            PrimitiveParameters::Box {
                size: [16., 1., 1.].map(length),
                frame: PrimitiveFrame {
                    origin: [-8., 0., -4.].map(length),
                    ..Default::default()
                },
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor.clone(), &stock).unwrap();
        session.commit(prepared.prepared).unwrap();

        let mut page = projection_page(torus_body);
        page.views[0].source = drawings::ViewSource::Model {
            entities: vec![torus_body, stock.body],
        };
        page.views[0].hidden_lines = true;
        let id = page.views[0].id;
        let section_id = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: section_id,
            name: "Torus and stock depth section".into(),
            kind: drawings::ViewKind::Section {
                offset_mm: 0.6,
                range: drawings::SectionRange::Depth { depth_mm: 4.7 },
                window: drawings::SectionWindow {
                    u_mm: Some([-9., 9.]),
                    v_mm: None,
                },
            },
            ..page.views[0].clone()
        });
        let transverse_id = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: transverse_id,
            name: "Torus transverse section".into(),
            source: drawings::ViewSource::Model {
                entities: vec![torus_body],
            },
            frame: multicad_core::references::ReferenceFrame {
                origin_mm: [0.; 3],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            kind: drawings::ViewKind::Section {
                offset_mm: 0.,
                range: drawings::SectionRange::SectionOnly,
                window: Default::default(),
            },
            ..page.views[0].clone()
        });
        let offset_id = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: offset_id,
            name: "Torus offset transverse section".into(),
            kind: drawings::ViewKind::Section {
                offset_mm: 4.5,
                range: drawings::SectionRange::SectionOnly,
                window: Default::default(),
            },
            ..page.views.last().unwrap().clone()
        });
        let retained_id = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: retained_id,
            name: "Torus retained transverse section".into(),
            kind: drawings::ViewKind::Section {
                offset_mm: -0.5,
                range: drawings::SectionRange::Depth { depth_mm: 2.5 },
                window: Default::default(),
            },
            ..page.views.last().unwrap().clone()
        });
        let oblique_id = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: oblique_id,
            name: "Torus oblique section".into(),
            frame: multicad_core::references::ReferenceFrame {
                origin_mm: [0.; 3],
                u: [0.8, 0., 0.6],
                v: [0., 1., 0.],
            },
            kind: drawings::ViewKind::Section {
                offset_mm: 3.5,
                range: drawings::SectionRange::SectionOnly,
                window: Default::default(),
            },
            ..page.views.last().unwrap().clone()
        });
        let tilted_id = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: tilted_id,
            name: "Torus oblique projection".into(),
            kind: drawings::ViewKind::Projection,
            ..page.views.last().unwrap().clone()
        });
        let (_, prepared) = drawings::prepare_create(
            &session,
            session.revision(),
            actor,
            "Drawing",
            Some(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let mut backend = Backend::with_cache_and_backend(None, GeometryBackend::Rust);
        let result = preview(&mut backend, &session, page.id);
        for view in [id, section_id] {
            let ViewEvaluation::Ready { vectors, .. } = &result.views[&view] else {
                panic!("{:?}", result.views[&view]);
            };
            assert!(vectors.iter().any(|v| v.source == torus_body
                && v.role == VectorRole::Visible
                && matches!(v.curve, VectorCurve::Ellipse { major_mm: 4., .. })));
            assert!(vectors.iter().any(|v| v.source == stock.body
                && v.role == VectorRole::Visible
                && matches!(v.curve, VectorCurve::Line { a,b }
                    if [a,b].into_iter().all(|p| p[1].abs() < 1e-8)
                        && (a[0].min(b[0])+4.).abs() < 1e-6 && (a[0].max(b[0])-4.).abs() < 1e-6)));
            if view == section_id {
                for radius in [4.2, 5.8] {
                    assert!(vectors.iter().any(|v| v.source == torus_body
                        && v.role == VectorRole::Section
                        && matches!(v.curve, VectorCurve::Ellipse { major_mm, .. }
                            if (major_mm - radius).abs() < 1e-8)));
                }
            }
        }
        let ViewEvaluation::Ready { vectors, .. } = &result.views[&transverse_id] else {
            panic!("{:?}", result.views[&transverse_id]);
        };
        assert_eq!(vectors.len(), 2);
        for expected in [-5., 5.] {
            assert!(vectors.iter().any(|v| v.source == torus_body
                && v.role == VectorRole::Section
                && matches!(v.curve, VectorCurve::Ellipse {
                    center: [x, y], major_mm: 1., minor_mm: 1., ..
                } if x == expected && y == 0.)));
        }
        let ViewEvaluation::Ready { vectors, .. } = &result.views[&offset_id] else {
            panic!("{:?}", result.views[&offset_id]);
        };
        assert!(vectors.len() > 16);
        assert!(vectors.iter().all(|edge| edge.source == torus_body
            && edge.role == VectorRole::Section
            && matches!(edge.curve, VectorCurve::Line { .. })));
        assert!(vectors.iter().any(|edge| {
            edge.preview_mm
                .iter()
                .any(|[u, v]| u.abs() < 1e-8 && (v.abs() - 0.75_f64.sqrt()).abs() < 1e-8)
        }));
        let ViewEvaluation::Ready { vectors, .. } = &result.views[&retained_id] else {
            panic!("{:?}", result.views[&retained_id]);
        };
        assert!(vectors.iter().any(|v| v.role == VectorRole::Section));
        assert!(vectors.iter().any(|v| v.role == VectorRole::Visible
            && matches!(v.curve, VectorCurve::Line { a, b }
                if (a[1] - 1.).abs() < 1e-8 && (b[1] - 1.).abs() < 1e-8)));
        let ViewEvaluation::Ready { vectors, .. } = &result.views[&oblique_id] else {
            panic!("{:?}", result.views[&oblique_id]);
        };
        assert!(vectors.len() > 16);
        assert!(vectors.iter().all(|v| v.source == torus_body
            && v.role == VectorRole::Section
            && matches!(v.curve, VectorCurve::Line { a, b }
            if [a,b].into_iter().all(|[u,v]| {
                let radius = 0.8_f64.mul_add(u, -2.1).hypot(v);
                let axial = 0.6_f64.mul_add(u, 2.8);
                ((radius - 5.).hypot(axial) - 1.).abs() < 1e-7
            }))));
        let ViewEvaluation::Ready { vectors, .. } = &result.views[&tilted_id] else {
            panic!("{:?}", result.views[&tilted_id]);
        };
        for expected in [-5., -3., 3., 5.] {
            assert!(vectors.iter().any(|v| {
                v.role == VectorRole::Visible
                    && v.preview_mm
                        .iter()
                        .any(|[u, v]| (u - expected).abs() < 1e-8 && v.abs() < 1e-8)
            }));
        }
    }

    #[test]
    fn rust_drawing_worker_publishes_curved_projection_and_detail_together() {
        use drawings::{VectorCurve, VectorRole, ViewEvaluation};

        let mut session = Session::new(Document::new("Rust curved drawing").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let cylinder = PrimitiveRequest::new(
            "Round stock",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(12.),
                frame: PrimitiveFrame::default(),
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor.clone(), &cylinder)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        let mut page = projection_page(cylinder.body);
        let projection = &mut page.views[0];
        projection.frame.v = [0., 0.8, 0.6];
        projection.hidden_lines = true;
        let projection_id = projection.id;
        let detail_id = EntityId::new();
        let detail = drawings::DrawingView {
            id: detail_id,
            name: "Rim detail".into(),
            kind: drawings::ViewKind::Detail {
                center_mm: [4., 2.],
                radius_mm: 2.5,
            },
            ..projection.clone()
        };
        page.views.push(detail);
        let (_, prepared) = drawings::prepare_create(
            &session,
            session.revision(),
            actor,
            "Drawing",
            Some(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let mut worker = Backend::with_cache_and_backend(None, GeometryBackend::Rust);
        let result = preview(&mut worker, &session, page.id);
        let view = |id| {
            let ViewEvaluation::Ready { vectors, .. } = &result.views[&id] else {
                panic!("{:?}", result.views[&id]);
            };
            assert!(vectors.iter().all(|v| v.source == cylinder.body));
            vectors
        };
        let projected = view(projection_id);
        assert_eq!(projected.len(), 5);
        assert_eq!(
            projected
                .iter()
                .filter(|v| v.role == VectorRole::Hidden)
                .count(),
            1,
        );
        assert!(
            projected
                .iter()
                .any(|v| matches!(v.curve, VectorCurve::Ellipse {
            major_mm, minor_mm, ..
        } if (major_mm - 5.).abs() < 1e-9 && (minor_mm - 4.).abs() < 1e-9))
        );
        let detail = view(detail_id);
        assert!(
            detail
                .iter()
                .any(|v| matches!(v.curve, VectorCurve::Ellipse { .. }))
        );
        assert!(
            detail
                .iter()
                .flat_map(|v| &v.preview_mm)
                .all(|[x, y]| { (x - 4.).hypot(y - 2.) <= 2.5 + 1e-9 })
        );
        assert_eq!(
            worker
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .unwrap()
                .evaluated,
            [cylinder.feature].into(),
        );
    }

    #[test]
    fn rust_drawing_worker_restores_annular_views_from_the_saved_project_and_cache() {
        use drawings::{VectorCurve, VectorRole, ViewEvaluation};
        use multicad_core::project_file::ProjectFile;
        use multicad_ui::{application::DocumentStorage, storage::SaveMode};

        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().join("drawing-cache"));
        let mut session = Session::new(Document::new("Annular drawing").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let washer = model::MechanicalPartRequest::new(
            "M6 washer",
            model::MechanicalPartParameters::Washer {
                bore_diameter: length(6.4),
                outside_diameter: length(12.),
                thickness: length(1.6),
                frame: Default::default(),
            },
        );
        let prepared =
            model::prepare_mechanical_part(&session, session.revision(), actor.clone(), &washer)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        let mut page = projection_page(washer.body);
        let end = page.views[0].id;
        let section = EntityId::new();
        let oblique = EntityId::new();
        let detail = EntityId::new();
        page.views.push(drawings::DrawingView {
            id: section,
            name: "Mid-thickness section".into(),
            kind: drawings::ViewKind::Section {
                offset_mm: 0.8,
                range: drawings::SectionRange::SectionOnly,
                window: drawings::SectionWindow::default(),
            },
            position_mm: [50., 80.],
            ..page.views[0].clone()
        });
        let mut angled = page.views[0].clone();
        angled.id = oblique;
        angled.name = "Oblique".into();
        angled.frame.v = [0., 0.8, 0.6];
        angled.position_mm = [90., 50.];
        angled.hidden_lines = true;
        page.views.push(angled.clone());
        let detail_center = [3.2, 0.96];
        page.views.push(drawings::DrawingView {
            id: detail,
            name: "Bore detail".into(),
            kind: drawings::ViewKind::Detail {
                center_mm: detail_center,
                radius_mm: 1.,
            },
            position_mm: [130., 50.],
            ..angled
        });
        let (_, prepared) = drawings::prepare_create(
            &session,
            session.revision(),
            actor,
            "Washer drawing",
            Some(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();

        let mut worker =
            Backend::with_cache_and_backend(Some(cache.clone()), GeometryBackend::Rust);
        let original = preview(&mut worker, &session, page.id);
        let view = |id| {
            let ViewEvaluation::Ready { vectors, .. } = &original.views[&id] else {
                panic!("{:?}", original.views[&id]);
            };
            assert!(vectors.iter().all(|v| v.source == washer.body));
            vectors
        };
        for (id, role) in [(end, VectorRole::Visible), (section, VectorRole::Section)] {
            let mut radii: Vec<_> = view(id)
                .iter()
                .map(|vector| {
                    assert_eq!(vector.role, role);
                    let VectorCurve::Ellipse {
                        major_mm,
                        minor_mm,
                        start_rad,
                        end_rad,
                        ..
                    } = vector.curve
                    else {
                        panic!("View lost its circular geometry: {:?}", vector.curve);
                    };
                    assert_eq!(major_mm, minor_mm);
                    assert!((end_rad - start_rad - std::f64::consts::TAU).abs() < 1e-9);
                    major_mm
                })
                .collect();
            radii.sort_by(f64::total_cmp);
            assert_eq!(radii, [3.2, 6.]);
        }
        let is_bore = |curve: &VectorCurve| {
            matches!(curve, VectorCurve::Ellipse { major_mm, minor_mm, .. }
                if (major_mm - 3.2).abs() < 1e-9 && (minor_mm - 2.56).abs() < 1e-9)
        };
        for role in [VectorRole::Visible, VectorRole::Hidden] {
            assert!(
                view(oblique)
                    .iter()
                    .any(|vector| vector.role == role && is_bore(&vector.curve))
            );
        }
        assert!(view(detail).iter().any(|vector| is_bore(&vector.curve)));
        assert!(
            view(detail)
                .iter()
                .flat_map(|v| &v.preview_mm)
                .all(|p| { (p[0] - detail_center[0]).hypot(p[1] - detail_center[1]) <= 1. + 1e-9 })
        );
        assert_eq!(
            worker
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .unwrap()
                .evaluated,
            [washer.feature].into(),
        );
        drop(worker);

        let path = directory.path().join("washer.tcd");
        let mut store = crate::files::ProjectStore::default();
        store
            .save_project(
                &path,
                &ProjectFile::new(session.document().clone()).unwrap(),
                SaveMode::Create,
            )
            .unwrap();
        let reopened = Session::new(store.open_project(&path).unwrap().file.document).unwrap();
        assert_eq!(reopened.document(), session.document());
        let mut cached = Backend::with_cache_and_backend(Some(cache), GeometryBackend::Rust);
        let restored = preview(&mut cached, &reopened, page.id);
        assert_eq!(restored.stamp.revision, reopened.revision());
        assert_eq!(restored.stamp.source_hash, original.stamp.source_hash);
        assert_eq!(restored.views, original.views);
        assert!(
            cached
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
        );
    }

    fn cabinet_panel_curve_counts(
        vectors: &[drawings::DrawingVector],
        role: drawings::VectorRole,
    ) -> [usize; 3] {
        use drawings::VectorCurve;
        use std::f64::consts::{PI, TAU};

        let mut counts = [0; 3];
        for vector in vectors.iter().filter(|vector| vector.role == role) {
            match vector.curve {
                VectorCurve::Line { .. } => counts[0] += 1,
                VectorCurve::Ellipse {
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                    ..
                } => {
                    assert!((major_mm - minor_mm).abs() < 1e-8);
                    let sweep = (end_rad - start_rad).abs();
                    if (sweep - TAU).abs() < 1e-8 {
                        counts[1] += 1;
                    } else {
                        assert!((sweep - PI).abs() < 1e-8);
                        assert!((major_mm - 4.).abs() < 1e-8);
                        counts[2] += 1;
                    }
                }
                _ => panic!("Unexpected cabinet panel curve: {:?}", vector.curve),
            }
        }
        counts
    }

    #[test]
    fn rust_worker_opens_the_saved_cabinet_drilling_view() {
        use drawings::{VectorRole, ViewEvaluation};

        let session = open_example("cabinet.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = drawing
            .pages
            .iter()
            .find(|page| page.number == "C-103")
            .unwrap();
        let saved = page
            .views
            .iter()
            .find(|view| view.name == "Left side, inner face")
            .unwrap();
        let [panel] = saved.source.entities().try_into().unwrap();
        assert_eq!(document.get(panel).unwrap().name, "Cabinet left side");

        let mut worker = Backend::with_cache_and_backend(None, GeometryBackend::Rust);
        let published = preview(&mut worker, &session, page.id);
        published.stamp.check(document, session.revision()).unwrap();
        assert_eq!(published.views.len(), page.views.len());
        let ViewEvaluation::Ready { vectors, .. } = &published.views[&saved.id] else {
            panic!(
                "Saved drilling view failed: {:?}",
                published.views[&saved.id]
            );
        };
        assert!(
            vectors
                .iter()
                .all(|vector| vector.source == panel && vector.role == VectorRole::Visible)
        );
        assert_eq!(
            cabinet_panel_curve_counts(vectors, VectorRole::Visible),
            [28, 66, 24]
        );
        let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
        for point in vectors.iter().flat_map(|vector| &vector.preview_mm) {
            for (axis, coordinate) in point.iter().enumerate() {
                bounds[0][axis] = bounds[0][axis].min(*coordinate);
                bounds[1][axis] = bounds[1][axis].max(*coordinate);
            }
        }
        assert_eq!(bounds, [[-250., -1000.], [250., 1000.]]);
    }

    type CardinalDrawingLines<K> = std::collections::BTreeMap<(K, usize, i64), Vec<[i64; 2]>>;

    fn cardinal_drawing_lines<'a, K: Ord>(
        vectors: impl Iterator<Item = &'a drawings::DrawingVector>,
        identity: impl Fn(&drawings::DrawingVector) -> K,
    ) -> CardinalDrawingLines<K> {
        // Compare geometric coverage even when one provider splits an edge
        // where it meets an adjacent coplanar panel.
        let mut lines = CardinalDrawingLines::new();
        let micrometers = |point: [f64; 2]| point.map(|value| (value * 1000.).round() as i64);
        for vector in vectors {
            let mut add = |a: [i64; 2], b: [i64; 2]| {
                let axis = usize::from(a[0] != b[0]);
                assert_eq!(a[axis], b[axis]);
                let [lo, hi] = [a[1 - axis].min(b[1 - axis]), a[1 - axis].max(b[1 - axis])];
                if lo != hi {
                    lines
                        .entry((identity(vector), axis, a[axis]))
                        .or_default()
                        .push([lo, hi]);
                }
            };
            match &vector.curve {
                drawings::VectorCurve::Line { a, b } => add(micrometers(*a), micrometers(*b)),
                // The native projector can encode a side-on circle as a
                // nonperiodic degree-one spline. Its collinear projected
                // poles describe the same straight span without arc sampling.
                drawings::VectorCurve::BSpline {
                    degree: 1,
                    periodic: false,
                    poles,
                    knots,
                    first,
                    last,
                    ..
                } if knots.first() == Some(first) && knots.last() == Some(last) => {
                    let base = micrometers(poles[0]);
                    assert!(
                        (0..2)
                            .any(|axis| poles.iter().all(|p| micrometers(*p)[axis] == base[axis]))
                    );
                    for pair in poles.windows(2) {
                        add(micrometers(pair[0]), micrometers(pair[1]));
                    }
                }
                _ => panic!("Expected a straight drawing edge, got {:?}", vector.curve),
            }
        }
        for spans in lines.values_mut() {
            spans.sort_unstable();
            let mut merged: Vec<[i64; 2]> = Vec::new();
            for [lo, hi] in spans.drain(..) {
                if let Some(last) = merged.last_mut()
                    && lo <= last[1]
                {
                    last[1] = last[1].max(hi);
                } else {
                    merged.push([lo, hi]);
                }
            }
            *spans = merged;
        }
        lines
    }

    #[test]
    fn rust_worker_preserves_exact_saved_cabinet_joint_geometry() {
        use drawings::{VectorCurve, VectorRole, ViewEvaluation};
        use std::f64::consts::TAU;

        let session = open_example("cabinet.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = drawing
            .pages
            .iter()
            .find(|page| page.number == "C-102")
            .unwrap();
        let drawer_view = page
            .views
            .iter()
            .find(|view| view.name == "Detail D: drawer 1 box, finger joints (top)")
            .unwrap();
        let tenon_view = page
            .views
            .iter()
            .find(|view| view.name == "Detail C: Domino joint, left side to lower divider")
            .unwrap();
        let section_view = page
            .views
            .iter()
            .find(|view| view.name == "Section A-A: lower divider with Domino tenons")
            .unwrap();
        let side_section = page
            .views
            .iter()
            .find(|view| view.name.starts_with("Section B-B:"))
            .unwrap();
        let [drawer] = drawer_view.source.entities().try_into().unwrap();
        assert_eq!(document.get(drawer).unwrap().name, "Drawer 1");

        let coverage = |backend| {
            let mut worker = Backend::with_cache_and_backend(None, backend);
            let published = preview(&mut worker, &session, page.id);
            let ready = |id| {
                let ViewEvaluation::Ready { vectors, .. } = &published.views[&id] else {
                    panic!(
                        "{backend:?} failed saved detail {id}: {:?}",
                        published.views[&id]
                    );
                };
                vectors
            };
            let drawer_vectors = ready(drawer_view.id);
            for vector in drawer_vectors {
                assert_eq!((vector.source, vector.role), (drawer, VectorRole::Visible));
            }
            let drawer_lines = cardinal_drawing_lines(drawer_vectors.iter(), |_| ());

            let tenon_vectors = ready(tenon_view.id);
            assert!(tenon_vectors.iter().all(|vector| {
                vector.source == vector.material_part
                    && matches!(vector.role, VectorRole::Section | VectorRole::Visible)
            }));
            let tenon_lines = [VectorRole::Section, VectorRole::Visible].map(|role| {
                cardinal_drawing_lines(
                    tenon_vectors.iter().filter(|vector| vector.role == role),
                    |vector| vector.source,
                )
            });
            let section_vectors = ready(section_view.id);
            assert!(section_vectors.iter().all(|vector| {
                matches!(vector.role, VectorRole::Section | VectorRole::Visible)
            }));
            let section_lines = [VectorRole::Section, VectorRole::Visible].map(|role| {
                cardinal_drawing_lines(
                    section_vectors.iter().filter(|vector| vector.role == role),
                    |vector| (vector.source, vector.material_part),
                )
            });
            let side_vectors = ready(side_section.id);
            assert!(side_vectors.iter().all(|vector| {
                matches!(vector.role, VectorRole::Section | VectorRole::Visible)
            }));
            let (side_lines, side_circles): (Vec<_>, Vec<_>) = side_vectors
                .iter()
                .partition(|vector| matches!(vector.curve, VectorCurve::Line { .. }));
            let side_lines = [VectorRole::Section, VectorRole::Visible].map(|role| {
                cardinal_drawing_lines(
                    side_lines
                        .iter()
                        .copied()
                        .filter(|vector| vector.role == role),
                    |vector| (vector.source, vector.material_part),
                )
            });
            let micrometers = |coordinate: f64| (coordinate * 1000.).round() as i64;
            let mut side_circles = side_circles
                .into_iter()
                .map(|vector| {
                    let VectorCurve::Ellipse {
                        center,
                        major_mm,
                        minor_mm,
                        start_rad,
                        end_rad,
                        ..
                    } = vector.curve
                    else {
                        panic!("Unexpected saved cutaway curve {:?}", vector.curve);
                    };
                    assert_eq!(vector.role, VectorRole::Visible);
                    assert!((major_mm - 2.5).abs() < 1e-8 && (minor_mm - major_mm).abs() < 1e-8);
                    assert!(((end_rad - start_rad).abs() - TAU).abs() < 1e-8);
                    (vector.source, vector.material_part, center.map(micrometers))
                })
                .collect::<Vec<_>>();
            side_circles.sort_unstable();
            (
                drawer_lines,
                tenon_lines,
                section_lines,
                side_lines,
                side_circles,
            )
        };
        let actual = coverage(GeometryBackend::Rust);
        assert_eq!(actual.0[&((), 1, -243_000)], [[-398_000, 398_000]]);
        assert_eq!(actual.0[&((), 0, -357_300)], [[-213_000, 213_000]]);
        assert_eq!(actual.1[0], actual.1[1]);
        let tenons = tenon_view
            .source
            .entities()
            .into_iter()
            .find(|&id| document.get(id).unwrap().name == "Carcass Dominos tenons 5")
            .unwrap();
        assert_eq!(
            actual.1[0][&(tenons, 0, -4_500)],
            [[-201_000, -179_000], [-20_000, 2_000], [161_000, 183_000]]
        );
        assert!(actual.2[0].keys().all(|key| actual.2[1].contains_key(key)));
        assert!(actual.2[1].len() > actual.2[0].len());
        assert_eq!(actual.4.len(), 54);
        #[cfg(feature = "occt")]
        assert_eq!(actual, coverage(GeometryBackend::OpenCascade));
    }

    #[test]
    fn rust_worker_preserves_exact_saved_cabinet_assembly_views() {
        use drawings::{AnnotationEvaluation, VectorRole, ViewEvaluation};

        let session = open_example("cabinet.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = drawing
            .pages
            .iter()
            .find(|page| page.number == "C-101")
            .unwrap();
        let coverage = |backend| {
            let mut worker = Backend::with_cache_and_backend(None, backend);
            let published = preview(&mut worker, &session, page.id);
            published
                .checked_page(document, session.revision(), book, page.id)
                .unwrap();
            let mut dimensions = published
                .annotations
                .values()
                .map(|annotation| {
                    let AnnotationEvaluation::Ready { graphic } = annotation else {
                        panic!("{backend:?} lost a saved cabinet dimension: {annotation:?}");
                    };
                    graphic.text.parse::<u32>().unwrap()
                })
                .collect::<Vec<_>>();
            dimensions.sort_unstable();
            assert_eq!(dimensions, [500, 800, 2000]);
            let ready = |name| {
                let saved = page.views.iter().find(|view| view.name == name).unwrap();
                let ViewEvaluation::Ready { vectors, .. } = &published.views[&saved.id] else {
                    panic!(
                        "{backend:?} failed saved {name}: {:?}",
                        published.views[&saved.id]
                    );
                };
                assert!(
                    vectors
                        .iter()
                        .all(|vector| vector.role == VectorRole::Visible)
                );
                vectors
            };
            // Near and far shelf pins can share the same front-view span.
            // Compare geometric coverage independently of their source labels.
            (
                cardinal_drawing_lines(ready("Front").iter(), |_| ()),
                cardinal_drawing_lines(ready("Top").iter(), |vector| {
                    (vector.source, vector.material_part)
                }),
                cardinal_drawing_lines(ready("Right side").iter(), |vector| {
                    (vector.source, vector.material_part)
                }),
            )
        };
        let actual = coverage(GeometryBackend::Rust);
        assert_eq!(actual.0[&((), 1, 1_000_000)], [[-400_000, 400_000]]);
        #[cfg(feature = "occt")]
        assert_eq!(actual, coverage(GeometryBackend::OpenCascade));
    }

    #[test]
    fn rust_worker_preserves_exact_hardware_in_the_saved_cabinet_drilling_page() {
        use drawings::{VectorCurve, VectorRole, ViewEvaluation};
        use std::{collections::BTreeMap, f64::consts::TAU};

        let session = open_example("cabinet.tcd");
        let document = session.document();
        let book = drawings::books(document).next().unwrap();
        let drawing = drawings::read(document, book).unwrap();
        let page = drawing
            .pages
            .iter()
            .find(|page| page.number == "C-103")
            .unwrap();
        let door_view = page
            .views
            .iter()
            .find(|view| view.name == "Left door with hinges, inner face")
            .unwrap();
        let runner_view = page
            .views
            .iter()
            .find(|view| view.name == "Drawer 1 box left side, outer face (runner cut away)")
            .unwrap();
        let [door] = door_view.source.entities().try_into().unwrap();
        let [drawer] = runner_view.source.entities().try_into().unwrap();
        assert_eq!(document.get(door).unwrap().name, "Left door");
        assert_eq!(document.get(drawer).unwrap().name, "Drawer 1");

        let coverage = |published: &DrawingEvaluation, backend| {
            let ready = |id| {
                let ViewEvaluation::Ready { vectors, .. } = &published.views[&id] else {
                    panic!(
                        "{backend:?} failed saved drilling view {id}: {:?}",
                        published.views[&id]
                    );
                };
                vectors
            };
            let door_vectors = ready(door_view.id);
            let mut circles = BTreeMap::<[i64; 3], Vec<[f64; 2]>>::new();
            for vector in door_vectors {
                assert_eq!((vector.source, vector.role), (door, VectorRole::Visible));
                match vector.curve {
                    VectorCurve::Line { .. } => {}
                    VectorCurve::Ellipse {
                        center,
                        u,
                        v,
                        major_mm,
                        minor_mm,
                        start_rad,
                        end_rad,
                    } => {
                        assert!((major_mm - minor_mm).abs() <= 1e-8);
                        let key =
                            [center[0], center[1], major_mm].map(|mm| (mm * 1000.).round() as i64);
                        let direction = (u[0] * v[1] - u[1] * v[0]).signum();
                        let base = u[1].atan2(u[0]);
                        let [a, b] = [start_rad, end_rad]
                            .map(|parameter| direction.mul_add(parameter, base));
                        let start = a.min(b).rem_euclid(TAU);
                        let end = start + (b - a).abs();
                        let spans = circles.entry(key).or_default();
                        spans.push([start, end.min(TAU)]);
                        if end > TAU {
                            spans.push([0., end - TAU]);
                        }
                    }
                    _ => panic!("Unexpected exact door edge: {:?}", vector.curve),
                }
            }
            for spans in circles.values_mut() {
                spans.sort_by(|a, b| a[0].total_cmp(&b[0]));
                let mut covered: f64 = 0.;
                for [a, b] in spans {
                    assert!(*a <= covered + 1e-8);
                    covered = covered.max(*b);
                }
                assert!((covered - TAU).abs() <= 1e-8);
            }
            let door_lines = cardinal_drawing_lines(
                door_vectors
                    .iter()
                    .filter(|vector| matches!(vector.curve, VectorCurve::Line { .. })),
                |_| (),
            );
            let runner_vectors = ready(runner_view.id);
            assert!(runner_vectors.iter().all(|vector| vector.source == drawer
                && matches!(vector.role, VectorRole::Visible | VectorRole::Section)));
            let runner_lines = [VectorRole::Section, VectorRole::Visible].map(|role| {
                cardinal_drawing_lines(
                    runner_vectors.iter().filter(|vector| vector.role == role),
                    |_| (),
                )
            });
            (
                door_lines,
                circles.into_keys().collect::<Vec<_>>(),
                runner_lines,
            )
        };

        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let mut cold = Backend::with_cache_and_backend(Some(cache.clone()), GeometryBackend::Rust);
        let published = preview(&mut cold, &session, page.id);
        let actual = coverage(&published, GeometryBackend::Rust);
        assert_eq!(
            actual.1,
            [
                [166_000, -334_500, 1_250],
                [166_000, -289_500, 1_250],
                [166_000, 289_000, 1_250],
                [166_000, 334_000, 1_250],
                [175_500, -312_000, 17_500],
                [175_500, 311_500, 17_500],
            ]
        );
        assert_eq!(actual.2[0][&((), 1, 22_500)], [[-225_000, 225_000]]);
        #[cfg(feature = "occt")]
        {
            let mut native = Backend::with_cache_and_backend(None, GeometryBackend::OpenCascade);
            assert_eq!(
                actual,
                coverage(
                    &preview(&mut native, &session, page.id),
                    GeometryBackend::OpenCascade
                )
            );
        }
        published
            .checked_page(document, session.revision(), book, page.id)
            .unwrap();
        drop(cold);

        let mut warm = Backend::with_cache_and_backend(Some(cache), GeometryBackend::Rust);
        let cached = preview(&mut warm, &session, page.id);
        assert_eq!(cached.views, published.views);
        assert!(
            warm.runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
        );
    }

    #[test]
    fn rust_worker_publishes_all_saved_front_panel_views_and_restores_the_disk_cache() {
        use drawings::{VectorRole, ViewEvaluation};

        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let session = open_example("front-panel.tcd");
        let document = session.document();
        let (book, page) = drawings::books(document)
            .find_map(|id| {
                drawings::read(document, id)
                    .unwrap()
                    .pages
                    .into_iter()
                    .find(|page| page.number == "FP-101")
                    .map(|page| (id, page))
            })
            .unwrap();
        assert_eq!(page.views.len(), 3);

        let mut cold = Backend::with_cache_and_backend(Some(cache.clone()), GeometryBackend::Rust);
        let published = preview(&mut cold, &session, page.id);
        published
            .checked_page(document, session.revision(), book, page.id)
            .unwrap();
        for name in ["Front", "Top", "Left end"] {
            let view = page.views.iter().find(|view| view.name == name).unwrap();
            let ViewEvaluation::Ready { vectors, .. } = &published.views[&view.id] else {
                panic!("{name}: {:?}", published.views[&view.id]);
            };
            assert!(
                vectors
                    .iter()
                    .any(|vector| vector.role == VectorRole::Visible)
            );
            assert!(
                vectors
                    .iter()
                    .any(|vector| vector.role == VectorRole::Hidden)
            );
        }
        assert_eq!(
            cold.runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .unwrap()
                .evaluated
                .len(),
            29
        );
        drop(cold);

        let mut reopened = Backend::with_cache_and_backend(Some(cache), GeometryBackend::Rust);
        let restored = preview(&mut reopened, &open_example("front-panel.tcd"), page.id);
        assert_eq!(restored.views, published.views);
        assert!(
            reopened
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
        );
    }

    #[test]
    fn rust_worker_draws_the_shipped_cabinet_panel_and_reuses_the_cached_page() {
        use drawings::{VectorRole, ViewEvaluation};

        let directory = tempfile::tempdir().unwrap();
        let cache = cache::Cache::isolated(directory.path().to_owned());
        let mut session = open_example("cabinet.tcd");
        let panel = session
            .document()
            .entities()
            .find(|entity| {
                entity.name == "Cabinet left side"
                    && matches!(
                        entity.data,
                        multicad_core::document::EntityData::Part { .. }
                    )
            })
            .expect("bundled left cabinet panel")
            .id;
        let mut page = projection_page(panel);
        let front = &mut page.views[0];
        front.name = "Inside face".into();
        front.frame.u = [0., 1., 0.];
        front.frame.v = [0., 0., 1.];
        let front_id = front.id;
        let mut back = front.clone();
        back.id = EntityId::new();
        back.name = "Outside face".into();
        back.frame.u = [0., -1., 0.];
        back.hidden_lines = true;
        let back_id = back.id;
        page.views.push(back);
        let (_, prepared) = drawings::prepare_create(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            "Cabinet panel machining",
            Some(page.clone()),
        )
        .unwrap();
        session.commit(prepared).unwrap();

        let mut worker =
            Backend::with_cache_and_backend(Some(cache.clone()), GeometryBackend::Rust);
        let original = preview(&mut worker, &session, page.id);
        let count = |id, role| {
            let ViewEvaluation::Ready { vectors, .. } = &original.views[&id] else {
                panic!("{:?}", original.views[&id]);
            };
            assert!(vectors.iter().all(|vector| vector.source == panel));
            cabinet_panel_curve_counts(vectors, role)
        };
        assert_eq!(count(front_id, VectorRole::Visible), [28, 66, 24]);
        assert_eq!(count(front_id, VectorRole::Hidden), [0; 3]);
        assert_eq!(count(back_id, VectorRole::Visible), [4, 0, 0]);
        assert_eq!(count(back_id, VectorRole::Hidden), [24, 66, 24]);
        assert!(
            worker
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_some()
        );
        drop(worker);

        let mut worker = Backend::with_cache_and_backend(Some(cache), GeometryBackend::Rust);
        let restored = preview(&mut worker, &session, page.id);
        assert_eq!(restored.views, original.views);
        assert!(
            worker
                .runtime
                .as_ref()
                .unwrap()
                .last_compile_stats()
                .is_none()
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
    fn fresh_rows(
        session: &Session,
        spec: ScheduleSpec,
        backend: GeometryBackend,
    ) -> Vec<Vec<String>> {
        let model = Compiler::default()
            .compile_with_backend(session.document(), session.revision(), backend, None)
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

    #[cfg(feature = "occt")]
    #[test]
    fn native_backend_places_checks_and_refreshes_a_real_bom_table() {
        places_checks_and_refreshes_a_real_bom_table(GeometryBackend::OpenCascade);
    }

    #[test]
    fn rust_backend_places_checks_and_refreshes_a_real_bom_table() {
        places_checks_and_refreshes_a_real_bom_table(GeometryBackend::Rust);
    }

    fn places_checks_and_refreshes_a_real_bom_table(backend: GeometryBackend) {
        let mut session = Session::new(Document::new("Native schedules").unwrap()).unwrap();
        add_box(&mut session, "Leg", 0.);
        add_box(&mut session, "Top", 40.);
        let mut editor = DrawingEditor::default();
        editor.set_backend_factory(Arc::new(move || {
            Box::new(Backend::with_cache_and_backend(None, backend))
        }));
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
        assert_eq!(placed(&session), fresh_rows(&session, spec, backend));
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
        assert_eq!(placed(&session), fresh_rows(&session, spec, backend));
        until(&mut editor, &mut session, |e, s| {
            e.schedule_status(s, table) == ScheduleStatus::Current
        });
    }
}
