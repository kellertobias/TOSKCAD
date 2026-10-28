use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    drawings::{DrawingView, Scale, VectorCurve, ViewEvaluation, ViewKind, ViewSource},
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    references::ReferenceFrame,
};
use multicad_model::{Compiler, DrawingProviders};
use std::{
    process::{Command, Stdio},
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

const CHILD: &str = "MULTICAD_DRAWING_FIRST_USE_CHILD";
const TEST: &str = "concurrent_first_use_preserves_independent_exact_drawing_vectors";

#[test]
fn concurrent_first_use_preserves_independent_exact_drawing_vectors() {
    if std::env::var_os(CHILD).is_some() {
        simultaneous_first_use();
        return;
    }
    // Each child starts with OCCT's process-global lazy plane uninitialized.
    // Repeating work in one process would exercise that race only once.
    for _ in 0..4 {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!("Concurrent native drawing child timed out: {output:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "Fresh-process native drawing failed: status={}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn simultaneous_first_use() {
    const JOBS: usize = 32;
    let barrier = Arc::new(Barrier::new(JOBS));
    let threads = (0..JOBS)
        .map(|index| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let width = 40. + index as f64;
                let height = 30. + index as f64 * 0.25;
                let mut session =
                    Session::new(Document::new("Independent drawing").unwrap()).unwrap();
                let request = PrimitiveRequest::new(
                    format!("Panel {index}"),
                    PrimitiveParameters::Box {
                        size: [length(width), length(height), length(20.)],
                        frame: Default::default(),
                    },
                );
                let prepared = model::prepare_primitive(
                    &session,
                    session.revision(),
                    Actor::local(Channel::Mcp),
                    &request,
                )
                .unwrap();
                session.commit(prepared.prepared).unwrap();
                let model = Compiler::default()
                    .compile(session.document(), session.revision())
                    .unwrap();
                let view = DrawingView {
                    id: multicad_core::document::EntityId::new(),
                    name: "Front".into(),
                    source: ViewSource::Model {
                        entities: vec![request.body],
                    },
                    frame: ReferenceFrame {
                        origin_mm: [0.; 3],
                        u: [1., 0., 0.],
                        v: [0., 1., 0.],
                    },
                    kind: ViewKind::Projection,
                    position_mm: [0., 0.],
                    scale: Scale {
                        numerator: 1,
                        denominator: 1,
                    },
                    hidden_lines: true,
                    tangent_edges: false,
                    paper_chord_tolerance_mm: 0.01,
                };
                barrier.wait();
                for _ in 0..4 {
                    let result = model
                        .drawing_view(
                            session.document(),
                            session.revision(),
                            &view,
                            &DrawingProviders::default(),
                        )
                        .unwrap();
                    let ViewEvaluation::Ready { vectors, .. } = result else {
                        panic!("Independent drawing did not produce exact vectors: {result:?}")
                    };
                    assert!(vectors.len() >= 4);
                    let mut min = [f64::INFINITY; 2];
                    let mut max = [f64::NEG_INFINITY; 2];
                    for vector in vectors {
                        assert_eq!(vector.source, request.body);
                        assert_eq!(vector.material_part, request.body);
                        let VectorCurve::Line { a, b } = vector.curve else {
                            panic!("Planar box produced an unexpected curve")
                        };
                        assert!((a[0] - b[0]).hypot(a[1] - b[1]) > 1e-7);
                        for p in [a, b] {
                            for axis in 0..2 {
                                assert!(p[axis].is_finite());
                                min[axis] = min[axis].min(p[axis]);
                                max[axis] = max[axis].max(p[axis]);
                            }
                        }
                        assert!(vector.preview_mm.iter().flatten().all(|v| v.is_finite()));
                    }
                    assert!(min.into_iter().all(|v| v.abs() < 1e-7));
                    assert!((max[0] - width).abs() < 1e-7);
                    assert!((max[1] - height).abs() < 1e-7);
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
}
