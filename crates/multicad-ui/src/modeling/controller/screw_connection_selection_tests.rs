//! Current native preview receipts authorize only source-neutral Inspector focus.
use super::*;
use crate::modeling::native_fixture::screw_connection as fixture;
use crate::{
    modeling::{ScrewConnectionCommand, native_fixture, screw_connection::ScrewConnectionPanel},
    viewport::{Ray, SceneScope},
};
use multicad_core::{
    document::Placement,
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
#[test]
fn native_library_leaf_display_selection_preserves_candidate_apply_and_source() {
    let (document, mut request, _) = fixture::fixture();
    let model::ScrewConnectionHardwareSource::Generated { part } = &request.hardware[1].source
    else {
        unreachable!()
    };
    let mut author = Session::new(Document::new("Selection Library author").unwrap()).unwrap();
    let q = model::prepare_mechanical_part(&author, 0, Actor::local(Channel::Ui), part).unwrap();
    author.commit(q.prepared).unwrap();
    let definition =
        SymbolAuthorRequest::new("Real6H nut", part.body, SymbolKind::Static, vec![], None);
    let q = symbols::prepare_author(
        &author,
        author.revision(),
        Actor::local(Channel::Ui),
        &definition,
    )
    .unwrap();
    author.commit(q.prepared).unwrap();
    let definition = symbols::read_definition(author.document(), definition.definition).unwrap();
    let placement = SymbolPlaceRequest::from_definition(
        EntityId::new(),
        &definition,
        "Pinned nut",
        BTreeMap::new(),
        Placement::default(),
    )
    .unwrap();
    request.hardware[1].instance = placement.instance;
    request.hardware[1].source = model::ScrewConnectionHardwareSource::Library {
        definition: Box::new(definition),
        placement: Box::new(placement.clone()),
    };
    let session = Session::new(document.clone()).unwrap();
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
        .begin_form(
            &session,
            &Default::default(),
            Channel::Ui,
            ModelForm::ScrewConnection(Box::new(
                ScrewConnectionPanel::from_request(&document, request.clone(), false).unwrap(),
            )),
        )
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(180);
    while controller.pending() {
        controller.poll(&session, &Default::default());
        assert!(Instant::now() < deadline, "{:?}", controller.error());
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    let original = controller
        .candidate
        .as_ref()
        .unwrap()
        .prepared
        .document()
        .clone();
    let stamp = controller.stamp();
    let source = controller
        .display_scene(&session, &Default::default())
        .unwrap();
    let receipt = source.assembly_evaluation().unwrap();
    let offsets = receipt
        .leaves
        .iter()
        .map(|leaf| {
            (
                leaf.path.clone(),
                if leaf.path.root == placement.instance {
                    [25., 0., 0.]
                } else {
                    [0.; 3]
                },
            )
        })
        .collect();
    let presented = source
        .presentation_leaf_offsets(&offsets, &BTreeMap::new())
        .unwrap();
    let repeated = source
        .presentation_leaf_offsets(&offsets, &BTreeMap::new())
        .unwrap();
    assert_eq!(presented.scope(), &SceneScope::Presentation);
    assert_eq!(
        presented.display_leaf_paths(),
        repeated.display_leaf_paths(),
        "Display highlights have stable IDs acrossframes"
    );
    let leaf = receipt
        .leaves
        .iter()
        .find(|leaf| leaf.path.root == placement.instance)
        .unwrap();
    assert!(
        leaf.path.members.contains(&placement.member),
        "Use the actual mapped private Library leaf path"
    );
    let center = leaf.pose.translation_mm;
    let ray = Ray::new(
        [center[0] + 25., center[1] + 2., center[2] + 20.],
        [0., 0., -1.],
    )
    .unwrap();
    let hit = presented
        .pick_display_leaf(ray, None)
        .expect("Actual visible exploded nut");
    assert_eq!(hit.path, leaf.path);
    assert_ne!(hit.display_instance, placement.instance);
    assert!(
        presented.pick_face(ray).is_none(),
        "A display hit cannot become a semantic face reference"
    );
    assert!(controller.focus_connection_leaf(&session, &Default::default(), &presented, &hit.path));
    assert!(controller.connection_leaf_focused(&leaf.path));
    assert_eq!(controller.stamp(), stamp);
    assert_eq!(
        controller.candidate.as_ref().unwrap().prepared.document(),
        &original
    );
    let ModelForm::ScrewConnection(panel) = controller.form.as_ref().unwrap() else {
        unreachable!()
    };
    let ScrewConnectionCommand::Set(current) = panel.request(session.document(), true).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(*current, request);
    assert!(controller.can_apply(&session, &Default::default()));
    assert_eq!(session.document(), &document);
    assert_eq!(session.revision(), 0);
    assert!(
        !controller.focus_connection_leaf(&session, &Default::default(), &source, &leaf.path),
        "Only the bound presentation receipt qualifies"
    );
    controller.invalidate();
    assert!(
        !controller.focus_connection_leaf(&session, &Default::default(), &presented, &leaf.path),
        "A stale candidate cannot focus a current draft"
    );
    assert!(!controller.can_apply(&session, &Default::default()));
}
