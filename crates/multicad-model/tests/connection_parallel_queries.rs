//! Serial/parallel COMMON equivalence on checked, world-placed connection solids.
//! Timings are diagnostics under the current machine load, not latency promises.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{OccurrencePath, prepare_screw_connection},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::Compiler;
use std::time::Instant;

fn query(a: &Solid, b: &Solid, parallel: bool, policy: GeometryPolicy) -> (Option<Solid>, f64) {
    let started = Instant::now();
    let result = if parallel {
        a.intersect_parallel(b, policy)
    } else {
        a.boolean(b, BooleanOperation::Intersect, policy)
    }
    .unwrap();
    (result, started.elapsed().as_secs_f64() * 1000.)
}
fn volume_budget(solid: &Solid, policy: GeometryPolicy) -> (f64, f64) {
    let volume = solid.volume_mm3().unwrap();
    let area = solid
        .faces()
        .unwrap()
        .iter()
        .map(|face| face.area_mm2)
        .sum::<f64>();
    // The established native metrology budget, with unchanged linear policy
    // and adaptive volume guard. This is not a new Boolean tolerance.
    (
        volume,
        policy.linear_tolerance_mm * area + 1e-7 * volume.abs(),
    )
}
#[test]
fn actual_connection_parallel_common_matches_serial_for_clear_and_wrong_phase_pairs() {
    let (document, request, _) = fixture::fixture();
    let session = Session::new(document.clone()).unwrap();
    let prepared =
        prepare_screw_connection(&session, 0, Actor::local(Channel::Mcp), &request).unwrap();
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(prepared.prepared.document(), 1)
    .unwrap();
    assert_eq!(session.document(), &document);
    let [proof] = model.snapshot().screw_connection_checks.as_slice() else {
        panic!("A complete native connection receipt is required")
    };
    assert!(proof.native_component_collision_checked);
    assert!((proof.actual_engagement_mm.unwrap() - 2.4).abs() < 1e-7);
    assert_eq!(model.snapshot().leaf_occurrences.len(), 3);
    let path = |instance| OccurrencePath {
        root: instance,
        members: vec![],
    };
    let screw_path = path(request.hardware[0].instance);
    let nut_path = path(request.hardware[1].instance);
    let screw = model.leaf_solid(&screw_path).unwrap();
    let nut = model.leaf_solid(&nut_path).unwrap();
    let stock = model
        .leaf_solid(&request.receivers[0].source.occurrence)
        .unwrap();
    let nut_pose = model.leaf_snapshot(&nut_path).unwrap().pose;
    let center = nut_pose.translation_mm;
    // Rotate the actual placed nut by half a turn about the actual common
    // axis. The hex outline is unchanged, but the helical phase is wrong.
    let wrong_nut = nut
        .transformed(RigidTransform {
            rotation: [[-1., 0., 0.], [0., -1., 0.], [0., 0., 1.]],
            translation_mm: [2. * center[0], 2. * center[1], 0.],
        })
        .unwrap();
    let policy = GeometryPolicy::default();
    assert_eq!(policy.boolean_fuzzy_mm, 0.);
    let sources = [screw, nut, stock, &wrong_nut];
    let before = sources.map(|solid| solid.to_brep().unwrap());
    let mut records = vec![];
    for (case, other, collision) in [
        ("actual_screw_nut", nut, false),
        ("actual_screw_receiving_cut_stock", stock, false),
        ("wrong_half_turn_screw_nut", &wrong_nut, true),
    ] {
        let mut reference: Option<(f64, f64)> = None;
        for run in 0..2 {
            // Reverse method order on the second paired run. No speed assertion
            // is made; source byte checks and volume measurements are untimed.
            for parallel in if run == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let (result, elapsed_ms) = query(screw, other, parallel, policy);
                let volume = if collision {
                    let result = result.expect("The wrong phase must produce a real native COMMON");
                    assert!(result.solid_count().unwrap() > 0);
                    let (volume, budget) = volume_budget(&result, policy);
                    assert!(
                        volume > 1e-3,
                        "Wrong phase overlap is not a meaningful positive control: {volume}"
                    );
                    if let Some((expected, previous_budget)) = reference {
                        assert!(
                            (volume - expected).abs() <= budget + previous_budget,
                            "Serial/parallel native volumes differ: {volume} vs {expected}"
                        );
                    } else {
                        reference = Some((volume, budget));
                    }
                    Some(volume)
                } else {
                    assert!(
                        result.is_none(),
                        "A valid connection requires strict None, not a small-volume allowance"
                    );
                    None
                };
                for (solid, bytes) in sources.iter().zip(&before) {
                    assert_eq!(
                        &solid.to_brep().unwrap(),
                        bytes,
                        "A COMMON query changed an operand BRep"
                    );
                }
                let record = serde_json::json!({"case":case,"paired_run":run,"parallel":parallel,"elapsed_ms":elapsed_ms,"common_volume_mm3":volume});
                eprintln!("connection_parallel_query {record}");
                records.push(record);
            }
        }
    }
    assert_eq!(records.len(), 12);
    assert_eq!(session.document(), &document);
    eprintln!(
        "connection_parallel_summary {}",
        serde_json::json!({
            "scope":"Native operation timings under current load, not isolated end-user latency or p95",
            "linear_tolerance_mm":policy.linear_tolerance_mm,"boolean_fuzzy_mm":policy.boolean_fuzzy_mm,
            "operand_brep_bytes_unchanged":true,"strict_empty_pairs":2,"positive_collision_pairs":1,"paired_runs_per_pair":2,"records":records
        })
    );
}
