use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, PrimitiveParameters, PrimitiveRequest, SurfaceStructureDirection,
        SurfaceStructureParameters, SurfaceStructurePattern, SurfaceStructureRequest, length,
    },
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn surface_structure_captures_previews_commits_and_edits_one_part() {
    let mut session = Session::new(Document::new("Relief transport").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Coupon",
        PrimitiveParameters::Box {
            size: [40., 30., 4.].map(length),
            frame: Default::default(),
        },
    );
    let actor = Actor::local(Channel::Ui);
    let prepared = model::prepare_primitive(&session, 0, actor, &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = session.document().clone();
    let host = Headless::new(Workspace::new(original.clone()).unwrap(), None, false);
    let call = Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let run = |tool, args| dispatch(&host, &call, tool, args).unwrap();
    let candidates = run(
        "reference_candidates",
        json!({"source": stock.feature, "kind": "face"}),
    );
    let top = &candidates["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| {
            let frame = &row["geometry"]["frame"];
            let component = |axis: &str, index: usize| frame[axis][index].as_f64().unwrap_or(0.);
            let normal_z =
                component("u", 0) * component("v", 1) - component("u", 1) * component("v", 0);
            normal_z.abs() > 0.99 && component("origin_mm", 2) == 4.
        })
        .unwrap()["element"]["ordinal"];
    let captured = run(
        "capture_surface_structure_faces",
        json!({"expected_revision":0,"part":stock.body,"producer":stock.feature,"faces":[top]}),
    );
    let mut request = SurfaceStructureRequest {
        feature: EntityId::new(),
        input: serde_json::from_value(captured["input"].clone()).unwrap(),
        faces: serde_json::from_value(captured["faces"].clone()).unwrap(),
        parameters: SurfaceStructureParameters::default(),
    };
    let preview = run(
        "preview_surface_structure",
        json!({"expected_revision":0,"request":request}),
    );
    assert_eq!(preview["solid_count"], 1);
    assert!(preview["volume_mm3"].as_f64().unwrap() > 4800.);
    assert_eq!(host.document(), original);
    let created = run(
        "set_surface_structure",
        json!({"expected_revision":0,"request":request}),
    );
    assert_eq!(created["revision"], 1);
    assert_eq!(created["part"], json!(stock.body));
    assert_eq!(created["feature"], json!(request.feature));
    let live = host.document();
    let mut invalid = request.clone();
    invalid.parameters.direction = SurfaceStructureDirection::Recessed;
    invalid.parameters.amplitude = length(4.);
    let error = dispatch(
        &host,
        &call,
        "set_surface_structure",
        json!({"expected_revision":1,"request":invalid}),
    )
    .unwrap_err();
    assert_eq!(error.code, "model_failed", "{error}");
    assert!(error.message.contains("break through"), "{error}");
    assert_eq!(host.document(), live);
    invalid.parameters.pattern = SurfaceStructurePattern::HeightMap {
        width: 12,
        height: 12,
        samples: vec![255; 144],
        repeat: [2, 2],
    };
    let error = dispatch(
        &host,
        &call,
        "set_surface_structure",
        json!({"expected_revision":1,"request":invalid}),
    )
    .unwrap_err();
    assert!(error.message.contains("192"), "{error}");
    assert_eq!(host.document(), live);

    request.parameters.pattern = SurfaceStructurePattern::HeightMap {
        width: 2,
        height: 2,
        samples: vec![0, 127, 192, 255],
        repeat: [1, 1],
    };
    let edited = run(
        "set_surface_structure",
        json!({"expected_revision":1,"request":request}),
    );
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["feature"], created["feature"]);
    assert_eq!(edited["part"], created["part"]);
    let inspected = run(
        "inspect_surface_structure",
        json!({"feature":request.feature}),
    );
    assert_eq!(inspected["request"], json!(request));
    let stale = dispatch(
        &host,
        &call,
        "capture_surface_structure_faces",
        json!({"expected_revision":1,"part":stock.body,"producer":stock.feature,"faces":[top]}),
    )
    .unwrap_err();
    assert_eq!(stale.code, "stale_revision");
}
