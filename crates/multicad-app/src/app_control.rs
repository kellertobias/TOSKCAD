//! MCP control of the open desktop window. Tool jobs are queued for the UI
//! thread, which runs them against the project bound to each connection.
use multicad_mcp::{Host, Job, Scope};
use multicad_ui::{
    application::external_control::{
        ControlClient, ControlRequest, ControlStatus, ExternalControl,
    },
    egui,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
};

struct WindowHost {
    connection: u64,
    jobs: Mutex<Sender<ControlRequest>>,
    wake: egui::Context,
}
impl Host for WindowHost {
    fn scope(&self) -> Scope {
        Scope {
            mode: "application",
            transport: "local-socket",
            project: None,
            export_directory: None,
            read_only: false,
        }
    }
    fn submit(&self, job: Job) -> Result<(), String> {
        self.jobs
            .lock()
            .map_err(|_| "The window control queue failed")?
            .send(ControlRequest {
                connection: self.connection,
                job,
            })
            .map_err(|_| "The MultiCAD window closed")?;
        self.wake.request_repaint();
        Ok(())
    }
}

/// The per-user socket a window listens on and `--mcp --app` connects to.
pub fn default_endpoint() -> Result<PathBuf, String> {
    #[cfg(unix)]
    return multicad_mcp::local_socket::default_endpoint();
    #[cfg(not(unix))]
    Err("Window control is available on macOS and Linux".into())
}

pub struct WindowControl {
    endpoint: Result<PathBuf, String>,
    wake: egui::Context,
    sender: Sender<ControlRequest>,
    receiver: Receiver<ControlRequest>,
    clients: Arc<Mutex<BTreeMap<u64, String>>>,
    #[cfg(unix)]
    listener: Option<multicad_mcp::local_socket::Listener>,
    error: Option<String>,
}
impl WindowControl {
    pub fn new(wake: egui::Context, endpoint: Result<PathBuf, String>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            endpoint,
            wake,
            sender,
            receiver,
            clients: Arc::default(),
            #[cfg(unix)]
            listener: None,
            error: None,
        }
    }
    #[cfg(unix)]
    fn start(&mut self) -> Result<(), String> {
        let path = self.endpoint.clone()?;
        let sender = self.sender.clone();
        let wake = self.wake.clone();
        let clients = Arc::clone(&self.clients);
        let events_wake = self.wake.clone();
        self.listener = Some(multicad_mcp::local_socket::Listener::bind(
            path,
            move |connection| {
                Arc::new(WindowHost {
                    connection,
                    jobs: Mutex::new(sender.clone()),
                    wake: wake.clone(),
                })
            },
            move |event| {
                let mut clients = clients.lock().unwrap_or_else(|e| e.into_inner());
                match event {
                    multicad_mcp::local_socket::Event::Connected { connection, client } => {
                        clients.insert(connection, client);
                    }
                    multicad_mcp::local_socket::Event::Disconnected { connection } => {
                        clients.remove(&connection);
                    }
                }
                events_wake.request_repaint();
            },
        )?);
        Ok(())
    }
    #[cfg(not(unix))]
    fn start(&mut self) -> Result<(), String> {
        Err("Window control is available on macOS and Linux. On this platform, use multicad --mcp for a headless document.".into())
    }
    fn enabled(&self) -> bool {
        #[cfg(unix)]
        return self.listener.is_some();
        #[cfg(not(unix))]
        false
    }
}
impl ExternalControl for WindowControl {
    fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.error = None;
        if enabled {
            if self.enabled() {
                return Ok(());
            }
            self.start().inspect_err(|e| self.error = Some(e.clone()))
        } else {
            #[cfg(unix)]
            {
                self.listener = None;
            }
            // Calls queued before the switch never reach the document.
            while let Ok(request) = self.receiver.try_recv() {
                (request.job)(Err("MCP connections were turned off".into()));
            }
            self.clients
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            Ok(())
        }
    }
    fn status(&self) -> ControlStatus {
        ControlStatus {
            enabled: self.enabled(),
            endpoint: self.endpoint.as_ref().ok().map(|p| p.display().to_string()),
            clients: self
                .clients
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|(connection, name)| ControlClient {
                    connection: *connection,
                    name: name.clone(),
                })
                .collect(),
            error: self
                .error
                .clone()
                .or_else(|| self.endpoint.as_ref().err().cloned()),
        }
    }
    fn poll(&mut self) -> Option<ControlRequest> {
        self.receiver.try_recv().ok()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use multicad_mcp::{
        Headless, Workspace,
        tools::{Call, dispatch},
    };
    use multicad_ui::application::Application;
    use serde_json::{Value, json};
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
        sync::atomic::AtomicBool,
    };

    /// One parameter-driven part with a cut, then Undo and Redo. Returns the
    /// observable results; entity IDs differ between runs by design.
    fn workflow(call: &mut dyn FnMut(&str, Value) -> Value) -> Value {
        let mm = |v: f64| json!({ "value": v, "unit": "mm" });
        call(
            "set_parameter",
            json!({ "expected_revision": 0, "name": "width", "expression": "40", "unit": "mm" }),
        );
        call(
            "set_parameter",
            json!({ "expected_revision": 1, "name": "depth", "expression": "=$width/2", "unit": "mm" }),
        );
        let sketch = call(
            "create_sketch",
            json!({ "expected_revision": 2, "plane": "xz", "name": "Base" }),
        )["sketch"]
            .clone();
        call(
            "sketch_add_rectangle",
            json!({ "expected_revision": 3, "sketch": sketch, "origin_mm": [0, 0],
                    "width": { "expression": "=$width", "unit": "mm" },
                    "height": { "expression": "=$depth", "unit": "mm" } }),
        );
        let part = call(
            "extrude",
            json!({ "expected_revision": 4, "sketch": sketch, "regions": [0],
                    "distance": mm(5.), "operation": "new_body" }),
        )["created"][0]["part"]
            .clone();
        let hole = call(
            "create_sketch",
            json!({ "expected_revision": 5, "plane": "xz", "name": "Hole" }),
        )["sketch"]
            .clone();
        call(
            "sketch_add_circle",
            json!({ "expected_revision": 6, "sketch": hole, "center_mm": [10, 10], "diameter": mm(4.) }),
        );
        call(
            "extrude",
            json!({ "expected_revision": 7, "sketch": hole, "regions": [0], "distance": mm(5.),
                    "operation": "cut", "target_part": part, "reverse": true }),
        );
        call(
            "set_parameter",
            json!({ "expected_revision": 8, "name": "width", "expression": "60", "unit": "mm" }),
        );
        let edited = call("evaluate_model", json!({}));
        call("undo", json!({ "expected_revision": 9 }));
        let undone = call("evaluate_model", json!({}));
        call("redo", json!({ "expected_revision": 10 }));
        let info = call("document_info", json!({}));
        let parameters = call("list_parameters", json!({}));
        let strip = |model: &Value| {
            model["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| json!({ "volume": p["volume_mm3"], "bounds": p["bounds_mm"], "solids": p["solid_count"] }))
                .collect::<Vec<_>>()
        };
        // Entity IDs are random, so list order is not part of equivalence.
        let mut rows: Vec<Value> = parameters["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| json!([p["name"], p["expression"], p["value"], p["role"]]))
            .collect();
        rows.sort_by_key(|r| r.to_string());
        json!({
            "edited": strip(&edited),
            "undone": strip(&undone),
            "revision": info["revision"],
            "counts": info["entity_counts"],
            "parameters": rows,
        })
    }

    struct Wire {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
        next: u64,
    }
    impl Wire {
        fn request(&mut self, method: &str, params: Value) -> Value {
            let id = self.next;
            self.next += 1;
            let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
            writeln!(self.writer, "{message}").unwrap();
            loop {
                let mut line = String::new();
                assert!(
                    self.reader.read_line(&mut line).unwrap() > 0,
                    "window closed"
                );
                let reply: Value = serde_json::from_str(&line).unwrap();
                if reply["id"] == json!(id) {
                    return reply;
                }
            }
        }
        fn call(&mut self, name: &str, arguments: Value) -> Value {
            let reply = self.request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            );
            reply["result"]["structuredContent"].clone()
        }
    }
    fn connect(path: &std::path::Path) -> Wire {
        let stream = UnixStream::connect(path).unwrap();
        let mut wire = Wire {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
            next: 1,
        };
        wire.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "window-test", "version": "1" } }),
        );
        writeln!(
            wire.writer,
            "{}",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
        )
        .unwrap();
        wire
    }

    /// Pump the window's UI-thread queue until `client` finishes.
    fn pump<T: Send + 'static>(
        app: &mut Application,
        client: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let worker = std::thread::spawn(client);
        while !worker.is_finished() {
            app.run_external_jobs();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        worker.join().unwrap()
    }

    #[test]
    fn window_and_headless_control_produce_equivalent_documents() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("private").join("app.sock");
        let mut control = WindowControl::new(egui::Context::default(), Ok(socket.clone()));
        control.set_enabled(true).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(socket.parent().unwrap()), 0o700);
        assert_eq!(mode(&socket), 0o600);
        let mut app = Application::new(Box::new(crate::files::SnapshotStore)).unwrap();
        app.set_external_control(Box::new(control));
        // Equivalence is about results, so this client applies without review.
        app.set_client_trusted("window-test", true);

        let path = socket.clone();
        let window = pump(&mut app, move || {
            let mut wire = connect(&path);
            let info = wire.call("document_info", json!({}));
            assert_eq!(info["scope"]["mode"], "application");
            assert_eq!(info["scope"]["transport"], "local-socket");
            workflow(&mut |name, arguments| {
                let value = wire.call(name, arguments);
                assert!(value.get("code").is_none(), "{name}: {value}");
                value
            })
        });

        let headless = Headless::new(
            Workspace::new(multicad_core::document::Document::new("Untitled").unwrap()).unwrap(),
            None,
            false,
        );
        let call = Call {
            actor: multicad_core::commands::Actor {
                id: "mcp:window-test".into(),
                channel: multicad_core::commands::Channel::Mcp,
            },
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let reference =
            workflow(&mut |name, arguments| dispatch(&headless, &call, name, arguments).unwrap());
        assert_eq!(window, reference);
        assert_eq!(window["revision"], 11);

        // The window's own Session holds the result, attributed to MCP.
        let session = &app.current().session;
        assert_eq!(session.revision(), 11);
        let last = session.activity().last().unwrap();
        assert_eq!(last.actor.id, "mcp:window-test");
        assert_eq!(last.actor.channel, multicad_core::commands::Channel::Mcp);
        let canonical = |d: &multicad_core::document::Document| {
            let mut kinds: Vec<_> = d
                .entities()
                .map(|e| (format!("{:?}", e.data.kind()), e.name.clone()))
                .collect();
            kinds.sort();
            kinds
        };
        assert_eq!(
            canonical(session.document()),
            canonical(&headless.document())
        );
    }

    #[test]
    fn reviewed_window_changes_preview_natively_and_apply_only_when_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("private").join("app.sock");
        let mut control = WindowControl::new(egui::Context::default(), Ok(socket.clone()));
        control.set_enabled(true).unwrap();
        let mut app = Application::new(Box::new(crate::files::SnapshotStore)).unwrap();
        app.set_external_control(Box::new(control));
        app.set_scene_backends(Arc::new(|| Box::new(crate::modeling::Backend::new())));
        let path = socket.clone();
        let (mut wire, sketch) = pump(&mut app, move || {
            let mut wire = connect(&path);
            // Set up a rectangle while trusted (see below), then review.
            let sketch = wire.call(
                "create_sketch",
                json!({ "expected_revision": 0, "plane": "xy", "name": "Base" }),
            );
            (wire, sketch)
        });
        // The first call was reviewed: nothing changed until it is accepted.
        assert_eq!(sketch["status"], "pending_review");
        assert_eq!(app.current().session.revision(), 0);
        let review = sketch["review"].as_u64().unwrap();
        assert_eq!(app.accept_review(review).unwrap(), 1);
        let sketch = sketch["sketch"].as_str().unwrap().to_owned();
        app.set_client_trusted("window-test", true);
        let mut wire = pump(&mut app, move || {
            wire.call(
                "sketch_add_rectangle",
                json!({ "expected_revision": 1, "sketch": sketch, "origin_mm": [0, 0],
                        "width": { "value": 30, "unit": "mm" }, "height": { "value": 20, "unit": "mm" } }),
            );
            wire
        });
        assert_eq!(
            app.current().session.revision(),
            2,
            "trusted calls apply directly"
        );
        app.set_client_trusted("window-test", false);
        let sketch_id = app
            .current()
            .session
            .document()
            .entities()
            .find(|e| e.name == "Base")
            .unwrap()
            .id
            .to_string();
        let (mut wire, extrude) = pump(&mut app, move || {
            let extrude = wire.call(
                "extrude",
                json!({ "expected_revision": 2, "sketch": sketch_id, "regions": [0],
                        "distance": { "value": 10, "unit": "mm" }, "operation": "new_body" }),
            );
            (wire, extrude)
        });
        assert_eq!(extrude["status"], "pending_review", "{extrude}");
        let review = extrude["review"].as_u64().unwrap();
        assert_eq!(app.current().session.revision(), 2);

        // Preview the candidate natively in the viewport without committing.
        app.preview_review(review).unwrap();
        for _ in 0..2000 {
            app.tick_reviews();
            if app.review_preview_scene().is_some() || app.review_preview_error().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(
            app.review_preview_error().is_none(),
            "{:?}",
            app.review_preview_error()
        );
        let scene = app.review_preview_scene().expect("native preview scene");
        let bounds = scene
            .native_instance_bounds(scene.instances()[0].id)
            .unwrap();
        assert!((bounds[1][0] - bounds[0][0] - 30.).abs() < 1e-6);
        assert!((bounds[1][2] - bounds[0][2] - 10.).abs() < 1e-6);
        assert!(app.current().sketch.review_scene_shown());
        assert_eq!(
            app.current().session.revision(),
            2,
            "a preview never commits"
        );

        assert_eq!(app.accept_review(review).unwrap(), 3);
        app.tick_reviews();
        assert!(
            !app.current().sketch.review_scene_shown(),
            "the preview closes on accept"
        );
        let accepted = app.current().session.activity().last().unwrap().clone();
        assert_eq!(accepted.actor.id, "mcp:window-test");

        // A queued change goes stale when the window edits first.
        let (mut wire, queued) = pump(&mut app, move || {
            let queued = wire.call(
                "set_parameter",
                json!({ "expected_revision": 3, "name": "w", "expression": "1", "unit": "mm" }),
            );
            (wire, queued)
        });
        let stale = queued["review"].as_u64().unwrap();
        app.current_mut()
            .session
            .undo(
                3,
                multicad_core::commands::Actor::local(multicad_core::commands::Channel::Ui),
            )
            .unwrap();
        app.tick_reviews();
        assert!(app.accept_review(stale).is_err());
        let (mut wire, rejected) = pump(&mut app, move || {
            let rejected = wire.call(
                "set_parameter",
                json!({ "expected_revision": 4, "name": "w", "expression": "1", "unit": "mm" }),
            );
            (wire, rejected)
        });
        let rejected = rejected["review"].as_u64().unwrap();
        app.reject_review(rejected).unwrap();
        let (states, activity) = pump(&mut app, move || {
            let states = [stale, rejected].map(|review| {
                wire.call("review_status", json!({ "review": review }))["state"]["state"].clone()
            });
            (states, wire.call("activity", json!({ "limit": 100 })))
        });
        assert_eq!(states, [json!("stale"), json!("rejected")]);
        let outcomes: Vec<_> = activity["entries"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .map(|e| {
                format!(
                    "{}:{}",
                    e["outcome"].as_str().unwrap(),
                    e["actor"]["id"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                "previewed:mcp:window-test",
                "applied:mcp:window-test",
                "applied:mcp:window-test",
                "previewed:mcp:window-test",
                "applied:mcp:window-test",
                "previewed:mcp:window-test",
                "undone:local-user",
                "stale:mcp:window-test",
                "previewed:mcp:window-test",
                "rejected:mcp:window-test",
                "inspected:mcp:window-test",
                "inspected:mcp:window-test",
            ],
            "Undo in the window added an entry and removed none"
        );
    }

    #[test]
    fn window_drafts_block_external_edits_and_disabling_rejects_queued_calls() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("private").join("app.sock");
        let mut control = WindowControl::new(egui::Context::default(), Ok(socket.clone()));
        control.set_enabled(true).unwrap();
        assert!(
            WindowControl::new(egui::Context::default(), Ok(socket.clone()))
                .set_enabled(true)
                .unwrap_err()
                .contains("already accepts")
        );
        let mut app = Application::new(Box::new(crate::files::SnapshotStore)).unwrap();
        app.set_external_control(Box::new(control));
        let row = app.current_mut().editor.rows.last_mut().unwrap();
        row.name = "draft".into();
        row.expression = "1".into();
        assert!(app.current().editor.dirty());
        let path = socket.clone();
        let (busy, info) = pump(&mut app, move || {
            let mut wire = connect(&path);
            let busy = wire.call(
                "set_parameter",
                json!({ "expected_revision": 0, "name": "w", "expression": "1", "unit": "mm" }),
            );
            (busy, wire.call("document_info", json!({})))
        });
        assert_eq!(busy["code"], "busy");
        assert_eq!(busy["current_revision"], 0);
        assert_eq!(info["revision"], 0, "inspection stays available");
        assert_eq!(app.current().session.revision(), 0);

        // A job still queued when connections are turned off is refused.
        let mut control = WindowControl::new(egui::Context::default(), Ok(socket.clone()));
        control.set_enabled(true).unwrap_err();
        let (sender, receiver) = mpsc::sync_channel(1);
        control
            .sender
            .send(ControlRequest {
                connection: 9,
                job: Box::new(move |owner| {
                    let _ = sender.send(owner.err());
                }),
            })
            .unwrap();
        control.set_enabled(false).unwrap();
        assert!(control.poll().is_none());
        assert_eq!(
            receiver.recv().unwrap().as_deref(),
            Some("MCP connections were turned off")
        );
    }
}
