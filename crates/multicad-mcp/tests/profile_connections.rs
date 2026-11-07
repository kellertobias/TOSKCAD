//! Public dispatch for the canonical connector request and native failure gate.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::{LibraryDefinition, ProfileShape, Stock, StockKind},
    model::{
        self, BodyInput, BodyOutput, MemberDefinition, MemberPath, MemberRequest, OccurrencePath,
        ProfileAttachment, ProfileConnectionKind, ProfileConnectionNut, ProfileConnectionRequest,
        ProfileConnectorAnchor, ProfileGrooveSide, length,
    },
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, catalog, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}

fn fixture() -> (Document, ProfileConnectionRequest) {
    let mut session = Session::new(Document::new("Connector transport").unwrap()).unwrap();
    let actor = Actor::local(Channel::Ui);
    let material = Entity::new(
        "Aluminum",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 165, 170, 255],
            section_hatch: None,
        },
    );
    let stock = EntityId::new();
    let profile = LibraryDefinition::Stock(Stock {
        material: material.id,
        specification: StockKind::Profile {
            designation: "item 0.0.026.03".into(),
            length_mm: 6000.,
            width_mm: 40.,
            height_mm: 40.,
            wall_mm: None,
            shape: ProfileShape::ItemLine8_40x40,
        },
    })
    .entity(stock, "Line 8 section")
    .unwrap();
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor.clone(),
            label: "Profile stock".into(),
            edits: vec![Mutation::Insert(material), Mutation::Insert(profile)],
        })
        .unwrap();
    let members = [0., 40.].map(|y| {
        MemberRequest::new(
            "Profile member",
            MemberDefinition {
                output: BodyOutput {
                    body: EntityId::nil(),
                },
                stock,
                path: MemberPath::Points {
                    start: [0., y, 0.].map(length),
                    end: [100., y, 0.].map(length),
                },
                anchor: Default::default(),
                rotation: None,
                extend_start: None,
                extend_end: None,
            },
        )
    });
    let prepared = model::prepare_members(&session, 1, actor, &members, None).unwrap();
    session.commit(prepared.prepared).unwrap();
    let [first, second] = members.map(|member| {
        let body = member.member.output.body;
        ProfileAttachment {
            member: BodyInput {
                body,
                producer: member.feature,
            },
            occurrence: OccurrencePath {
                root: body,
                members: vec![],
            },
            anchor: ProfileConnectorAnchor::Groove {
                side: ProfileGrooveSide::MaxY,
                station: length(40.),
            },
        }
    });
    let request = ProfileConnectionRequest::new(
        "Joining plate",
        ProfileConnectionKind::JoiningPlate,
        first,
        second,
    );
    (session.document().clone(), request)
}

#[test]
fn profile_connection_catalog_and_refusals_leave_live_source_unchanged() {
    let tools = catalog();
    for (name, read_only) in [
        ("set_profile_connection", false),
        ("inspect_profile_connection", true),
        ("remove_profile_connection", false),
    ] {
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap()
                .read_only,
            read_only
        );
    }
    let (original, request) = fixture();
    let host = Headless::new(Workspace::new(original.clone()).unwrap(), None, false);
    let input = |revision| json!({"expected_revision":revision,"request":request});
    for (call, args, code) in [
        (call(false), input(1), "stale_revision"),
        (call(true), input(0), "cancelled"),
        (
            call(false),
            json!({"expected_revision":0,"request":{"connection":request.connection}}),
            "invalid_request",
        ),
    ] {
        let error = dispatch(&host, &call, "set_profile_connection", args).unwrap_err();
        assert_eq!(error.code, code, "{error}");
        assert_eq!(host.document(), original);
    }
    let readonly = Headless::new(Workspace::new(original.clone()).unwrap(), None, true);
    let error = dispatch(&readonly, &call(false), "set_profile_connection", input(0)).unwrap_err();
    assert_eq!(error.code, "read_only");
    assert_eq!(readonly.document(), original);
}

#[test]
fn profile_connection_native_create_edit_and_remove_share_canonical_ids() {
    let (original, mut request) = fixture();
    let host = Headless::new(Workspace::new(original.clone()).unwrap(), None, false);
    let run =
        |name: &str, arguments: Value| dispatch(&host, &call(false), name, arguments).unwrap();
    let created = run(
        "set_profile_connection",
        json!({"expected_revision":0,"request":request}),
    );
    assert_eq!(created["revision"], 1);
    assert_eq!(created["native_geometry_checked"], true);
    assert_eq!(created["parts"].as_array().unwrap().len(), 5);
    assert_eq!(created["connection"], json!(request.connection));
    let inspect = |request: &ProfileConnectionRequest| {
        let inspected = run(
            "inspect_profile_connection",
            json!({"connection":request.connection}),
        );
        assert_eq!(inspected["article"], "0.0.692.16");
        assert_eq!(inspected["nut_article"], request.nut.article());
        assert_eq!(inspected["request"], json!(request));
    };
    inspect(&request);

    let live = host.document();
    let mut invalid = request.clone();
    invalid.second.anchor = ProfileConnectorAnchor::Groove {
        side: ProfileGrooveSide::MaxY,
        station: length(41.),
    };
    let error = dispatch(
        &host,
        &call(false),
        "set_profile_connection",
        json!({"expected_revision":1,"request":invalid}),
    )
    .unwrap_err();
    assert_eq!(error.code, "model_failed", "{error}");
    assert!(
        error.message.contains("coplanar parallel grooves"),
        "{error}"
    );
    assert_eq!(host.document(), live);

    request.name = "Renamed joining plate".into();
    request.nut = ProfileConnectionNut::Stainless;
    let edited = run(
        "set_profile_connection",
        json!({"expected_revision":1,"request":request}),
    );
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["parts"], created["parts"]);
    inspect(&request);

    let removed = run(
        "remove_profile_connection",
        json!({"expected_revision":2,"connection":request.connection}),
    );
    assert_eq!(removed["revision"], 3);
    assert_eq!(removed["native_geometry_checked"], true);
    assert_eq!(host.document(), original);
}
