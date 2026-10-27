use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, EntityId, Mutation},
    sketch::{self, Datum, Geometry, Sketch},
};
fn apply(s: &mut Session, label: &str, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: Actor::local(Channel::CommandLine),
        label: label.into(),
        edits,
    })
    .unwrap();
}
#[test]
fn datum_geometry_roundtrip_undo_and_invalid_inputs_preserve_source() {
    for datum in Datum::ALL {
        let mut s = Session::new(Document::new("Sketch test").unwrap()).unwrap();
        let (id, edits) = sketch::create(datum, "Outline").unwrap();
        apply(&mut s, "New sketch", edits);
        let gid = EntityId::new();
        let geometry = Geometry::Rectangle {
            id: gid,
            a: [0., 0.],
            b: [80., 60.],
        };
        let edits = sketch::append(s.document(), id, geometry.clone()).unwrap();
        apply(&mut s, "Rectangle", edits);
        let wire = serde_json::to_vec(s.document()).unwrap();
        let reopened: Document = serde_json::from_slice(&wire).unwrap();
        let (source, frame, _) = Sketch::read(&reopened, id).unwrap();
        assert_eq!(source.geometry, vec![geometry.clone()]);
        assert_eq!(frame.coordinates(frame.point([80., 60.])), [80., 60.]);
        let before = s.document().clone();
        assert!(
            sketch::append(
                &before,
                id,
                Geometry::Line {
                    id: EntityId::new(),
                    a: [1., 1.],
                    b: [1., 1.]
                }
            )
            .is_err()
        );
        // Bypassing convenience constructors still hits canonical validation.
        let edits = sketch::append(s.document(), id, geometry).unwrap();
        assert!(
            s.apply(Request {
                expected_revision: s.revision(),
                actor: Actor::local(Channel::Mcp),
                label: "duplicate".into(),
                edits
            })
            .is_err()
        );
        assert_eq!(s.document(), &before);
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert!(
            Sketch::read(s.document(), id)
                .unwrap()
                .0
                .geometry
                .is_empty()
        );
        s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert_eq!(
            Sketch::read(s.document(), id).unwrap().0.geometry[0].id(),
            gid
        );
    }
}
#[test]
fn translating_shared_plane_detaches_only_selected_sketch_and_is_undoable() {
    let mut s = Session::new(Document::new("Shared datum").unwrap()).unwrap();
    let (id, edits) = sketch::create(Datum::Xz, "First").unwrap();
    apply(&mut s, "First", edits);
    let mut other = s.document().get(id).unwrap().clone();
    other.id = EntityId::new();
    other.name = "Second".into();
    let second = other.id;
    apply(&mut s, "Second", vec![Mutation::Insert(other)]);
    let edits =
        sketch::translate(s.document(), id, Datum::Xz.frame().vector([10., 20., 30.])).unwrap();
    apply(&mut s, "Move", edits);
    assert_eq!(
        Sketch::read(s.document(), id).unwrap().1.origin,
        [10., -30., 20.]
    );
    assert_eq!(
        Sketch::read(s.document(), second).unwrap().1.origin,
        [0.; 3]
    );
    s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(Sketch::read(s.document(), id).unwrap().1.origin, [0.; 3]);
    let mut invalid = s.document().get(id).unwrap().clone();
    if let EntityData::Sketch { definition, .. } = &mut invalid.data {
        definition.data["u"] = serde_json::json!([0., 0., 0.]);
    }
    assert!(s.document().edited(&[Mutation::Replace(invalid)]).is_err());
}
#[test]
fn missing_plane_remains_loadable_for_repair_but_cannot_be_evaluated() {
    let (id, mut edits) = sketch::create(Datum::Xy, "Broken reference").unwrap();
    edits.remove(0);
    let doc = Document::new("Repair").unwrap().edited(&edits).unwrap();
    assert_eq!(doc.reference_diagnostics().len(), 1);
    assert!(Sketch::read(&doc, id).is_err());
    let loaded: Document = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(doc, loaded);
}
