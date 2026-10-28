use multicad_core::{
    document::{Document, EntityId},
    model::ModelEvaluationInfo,
    project_file::ContentHash,
    references::{
        Diagnostic, GeometryIntent, Origin, ReferenceAddress, ReferenceReport, Resolution,
        TopologyKind, inspect,
    },
};
use multicad_ui::viewport::{Scene, SceneBackend, SceneFeed, SceneReply, SceneStamp};
use std::{collections::BTreeSet, sync::Arc};

fn stamp(document: &Document) -> SceneStamp {
    SceneStamp {
        document: document.id(),
        revision: 7,
        generation: 1,
    }
}
fn scene(document: &Document, stamp: SceneStamp) -> Scene {
    Scene::new(stamp, vec![], vec![])
        .unwrap()
        .with_model_evaluation(Arc::new(ModelEvaluationInfo {
            source_hash: ContentHash::document(document).unwrap(),
            consumed_bodies: BTreeSet::new(),
            cut_targets: vec![],
        }))
        .unwrap()
}
fn report(document: &Document) -> ReferenceReport {
    let mut report = inspect(document, 7, &Default::default());
    let source = EntityId::new();
    let intent = GeometryIntent {
        id: EntityId::new(),
        kind: TopologyKind::Edge,
        origins: BTreeSet::from([Origin {
            source,
            role: "fixture.edge".into(),
        }]),
        boundary: None,
        attachment: None,
    };
    report.entries.push(Diagnostic {
        address: ReferenceAddress {
            owner: EntityId::new(),
            reference: intent.id,
        },
        source,
        role: "source".into(),
        intent,
        resolution: Resolution::Invalid {
            message: "Exact finite intersection requires repair".into(),
        },
        affected: vec![],
        dependency_hash: None,
    });
    report
}

#[test]
fn native_reference_reports_require_matching_source_revision_and_preserve_hidden_results() {
    let document = Document::new("Native report metadata").unwrap();
    let source = ContentHash::document(&document).unwrap();
    let expected = report(&document);
    assert!(
        Scene::new(stamp(&document), vec![], vec![])
            .unwrap()
            .with_native_reference_report(source, expected.clone())
            .is_err()
    );
    assert!(
        scene(&document, stamp(&document))
            .with_native_reference_report(ContentHash::of(b"other source"), expected.clone())
            .is_err()
    );
    let mut stale = expected.clone();
    stale.revision -= 1;
    assert!(
        scene(&document, stamp(&document))
            .with_native_reference_report(source, stale)
            .is_err()
    );
    let scene = scene(&document, stamp(&document))
        .with_native_reference_report(source, expected.clone())
        .unwrap();
    assert_eq!(scene.native_reference_report(source), Some(&expected));
    assert!(
        scene
            .native_reference_report(ContentHash::of(b"another document"))
            .is_none()
    );
    let hidden = scene.visible_subset(&BTreeSet::new()).unwrap();
    assert_eq!(hidden.native_reference_report(source), Some(&expected));
    let hidden = Arc::try_unwrap(hidden).ok().unwrap();
    assert!(
        hidden
            .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                source_hash: ContentHash::of(b"foreign replacement"),
                consumed_bodies: BTreeSet::new(),
                cut_targets: vec![],
            }))
            .is_err()
    );
}

#[test]
fn scene_feed_keeps_native_diagnostics_instead_of_reconstructing_a_clean_display_report() {
    struct Backend(Option<SceneReply>);
    impl SceneBackend for Backend {
        fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
            let source = ContentHash::document(document)?;
            self.0 = Some(SceneReply {
                stamp,
                result: Ok(Arc::new(
                    scene(document, stamp)
                        .with_native_reference_report(source, report(document))?,
                )),
            });
            Ok(())
        }
        fn poll(&mut self) -> Option<SceneReply> {
            self.0.take()
        }
    }
    let document = Document::new("Finite reference diagnostic").unwrap();
    assert!(inspect(&document, 7, &Default::default()).all_resolved());
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(None)));
    feed.tick(&document, 7);
    let current = feed.references().unwrap();
    assert_eq!(current.report.entries.len(), 1);
    assert!(!current.report.all_resolved());
    assert!(!feed.is_current(document.id(), 7));
    assert!(feed.visible().is_none());
    assert!(feed.error().is_some());
}
