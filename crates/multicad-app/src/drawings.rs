//! Native drawing composition, sharing the bounded geometry worker while keeping
//! native solids and projection operations off the UI thread.
use multicad_core::{
    document::Document,
    drawings::{DrawingEvaluation, DrawingStamp},
    schedules::ScheduleSpec,
};
use multicad_model::{Compiler, DrawingProviders, ModelError, Runtime, schedules};
use multicad_ui::{
    drawings::{DrawingBackend, DrawingReply},
    schedules::{ScheduleReply, ScheduleTable},
};
use std::sync::Arc;
type Tables = Vec<(ScheduleSpec, Result<ScheduleTable, String>)>;
pub struct Backend {
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
impl Backend {
    pub fn new() -> Self {
        Self {
            runtime: Runtime::with_model_publisher(Compiler::default(), |document, model, _| {
                model
                    .drawings(
                        document,
                        model.snapshot().stamp.revision,
                        &DrawingProviders::default(),
                    )
                    .map(Arc::new)
            }),
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
                Runtime::with_model_publisher(Compiler::default(), |document, model, mask| {
                    Ok(build_tables(document, model, mask))
                })
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

    fn add_box(session: &mut Session, name: &str, x: f64) {
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
