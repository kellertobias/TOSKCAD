//! Native candidate reconciliation is independent of any thread-family increment.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData},
    project_file,
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:owner-reconciliation".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
fn empty() -> Headless {
    Headless::new(
        Workspace::new(Document::new("Owner reconciliation").unwrap()).unwrap(),
        None,
        false,
    )
}

#[derive(Clone, Copy)]
enum OwnerRace {
    Edit,
    SwitchSession,
    Cancel,
    Draft,
}
struct ReviewedTarget<'a>(&'a mut dyn multicad_mcp::Target, bool);
impl multicad_core::control::SessionOwner for ReviewedTarget<'_> {
    fn session(&self) -> &Session {
        self.0.session()
    }
    fn session_mut(&mut self) -> &mut Session {
        self.0.session_mut()
    }
    fn resources(&self) -> &project_file::ProjectResources {
        self.0.resources()
    }
    fn project_path(&self) -> Option<&std::path::Path> {
        self.0.project_path()
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        self.0.save()
    }
    fn control(&mut self) -> (&mut Session, &mut multicad_core::control::ControlLedger) {
        self.0.control()
    }
    fn ledger(&self) -> &multicad_core::control::ControlLedger {
        self.0.ledger()
    }
    fn review_policy(&self, _: &Actor) -> multicad_core::control::ReviewPolicy {
        multicad_core::control::ReviewPolicy::Reviewed
    }
    fn edit_blocked(&self) -> Option<String> {
        if self.1 {
            Some("Owner has an unapplied draft".into())
        } else {
            self.0.edit_blocked()
        }
    }
}
struct InterleavedReviewedHost {
    owner: Headless,
    race: OwnerRace,
    submissions: std::sync::atomic::AtomicUsize,
    cancellation: Arc<AtomicBool>,
    after_owner_change: Arc<std::sync::Mutex<Option<(Document, u64)>>>,
}
impl multicad_mcp::Host for InterleavedReviewedHost {
    fn scope(&self) -> multicad_mcp::Scope {
        multicad_mcp::Host::scope(&self.owner)
    }
    fn submit(&self, job: multicad_mcp::Job) -> Result<(), String> {
        // Both tools snapshot on submission 0, prepare on 1, then return
        // after native evaluation on 2. Interleave precisely before that final
        // closure, without threads, sleeps, or instrumentation of the compiler.
        let finish = self
            .submissions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 2;
        let race = self.race;
        let cancellation = self.cancellation.clone();
        let expected = self.after_owner_change.clone();
        multicad_mcp::Host::submit(
            &self.owner,
            Box::new(move |target| match target {
                Ok(target) => {
                    if finish {
                        match race {
                            OwnerRace::Edit => {
                                let revision = target.session().revision();
                                let parameter = multicad_core::document::Entity::new(
                                    "owner_change",
                                    EntityData::Parameter {
                                        expression: "1".into(),
                                        unit: "mm".into(),
                                        exposed: false,
                                        inputs: vec![],
                                    },
                                );
                                target
                                    .session_mut()
                                    .apply(multicad_core::commands::Request {
                                        expected_revision: revision,
                                        actor: Actor::local(Channel::Ui),
                                        label: "Owner edit during native preflight".into(),
                                        edits: vec![multicad_core::document::Mutation::Insert(
                                            parameter,
                                        )],
                                    })
                                    .unwrap();
                            }
                            OwnerRace::SwitchSession => {
                                let document = target.session().document().clone();
                                *target.session_mut() = Session::new(document).unwrap();
                            }
                            OwnerRace::Cancel => {
                                cancellation.store(true, std::sync::atomic::Ordering::SeqCst)
                            }
                            OwnerRace::Draft => {}
                        }
                        *expected.lock().unwrap() = Some((
                            target.session().document().clone(),
                            target.session().revision(),
                        ));
                    }
                    job(Ok(&mut ReviewedTarget(
                        target,
                        finish && matches!(race, OwnerRace::Draft),
                    )));
                }
                Err(error) => job(Err(error)),
            }),
        )
    }
}

#[test]
fn thread_and_mechanical_review_queues_refuse_owner_edits_session_replacement_cancellation_and_drafts_after_native_preflight()
 {
    // A short existing metric profile and a native annular washer exercise the
    // shared commit guard through two independent canonical provider paths.
    for (tool, args) in [
        (
            "set_thread",
            json!({"expected_revision":0,"name":"Preflight thread","family":"iso_metric","major_diameter":mm(3.),"pitch":mm(0.5),"length":mm(1.5)}),
        ),
        (
            "set_mechanical_part",
            json!({"expected_revision":0,"name":"Preflight washer","recipe":{"kind":"washer","bore_diameter":mm(3.2),"outside_diameter":mm(7.),"thickness":mm(0.5)}}),
        ),
    ] {
        for (race, code) in [
            (OwnerRace::Edit, "stale_revision"),
            (OwnerRace::SwitchSession, "stale_preview"),
            (OwnerRace::Cancel, "cancelled"),
            (OwnerRace::Draft, "busy"),
        ] {
            let cancellation = Arc::new(AtomicBool::new(false));
            let h = InterleavedReviewedHost {
                owner: empty(),
                race,
                submissions: std::sync::atomic::AtomicUsize::new(0),
                cancellation: cancellation.clone(),
                after_owner_change: Arc::default(),
            };
            let call = Call {
                actor: call(false).actor,
                cancel: cancellation,
            };
            let result = dispatch(&h, &call, tool, args.clone());
            assert_eq!(result.unwrap_err().code, code, "{tool}");
            let (expected, revision) = h
                .after_owner_change
                .lock()
                .unwrap()
                .clone()
                .expect("Owner action ran after preparation and native evaluation");
            assert_eq!(
                h.owner.document(),
                expected,
                "{tool}: only the independent owner action may change source"
            );
            multicad_mcp::host::with(&h, move |owner| {
                assert_eq!(owner.session().revision(), revision);
                assert_eq!(owner.session().document(), &expected);
                assert!(
                    owner.ledger().pending().is_empty(),
                    "A stale candidate must not become an owner review"
                );
                assert!(
                    !owner
                        .session()
                        .document()
                        .entities()
                        .any(|entity| matches!(entity.data, EntityData::Feature { .. })),
                    "No provider source may be partially committed"
                );
            })
            .unwrap();
        }
    }
}
