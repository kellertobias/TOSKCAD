use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Placement},
    drawings::*,
    plants::{self, PlantKind},
    references::ReferenceFrame,
};
use multicad_model::{Compiler, DrawingProviders};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn placed(kind: PlantKind) -> (Session, plants::PlantPlaceRequest) {
    let mut s = Session::new(Document::new("Plant plan").unwrap()).unwrap();
    let (height, spread) = if kind == PlantKind::Tree {
        (6000., 4000.)
    } else {
        (800., 1000.)
    };
    let request = plants::PlantPlaceRequest::new(
        &s,
        plants::generic_definition(kind).unwrap(),
        "Plant".into(),
        height,
        spread,
        Placement {
            translation_mm: [5000., 6000., 200.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        },
    )
    .unwrap();
    s.commit(plants::prepare_place(&s, 0, actor(), &request).unwrap())
        .unwrap();
    (s, request)
}
fn view(instance: multicad_core::document::EntityId, denominator: u32) -> DrawingView {
    DrawingView {
        id: multicad_core::document::EntityId::new(),
        name: "Planting plan".into(),
        source: ViewSource::Model {
            entities: vec![instance],
        },
        frame: ReferenceFrame {
            origin_mm: [0., 0., 1100.],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [100., 100.],
        scale: Scale {
            numerator: 1,
            denominator,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
    }
}
#[test]
fn exact_linked_footprints_use_canopy_spread_not_cut_trunk_or_faceted_solids() {
    for kind in [PlantKind::Tree, PlantKind::Shrub] {
        let (s, request) = placed(kind);
        let model = Compiler::default()
            .compile(s.document(), s.revision())
            .unwrap();
        let instance = request.placement.instance;
        let leaves: Vec<_> = model
            .snapshot()
            .leaf_occurrences
            .values()
            .filter(|l| l.path.root == instance)
            .collect();
        assert_eq!(leaves.len(), if kind == PlantKind::Tree { 2 } else { 1 });
        let spread = if kind == PlantKind::Tree {
            4000.
        } else {
            1000.
        };
        let height = if kind == PlantKind::Tree { 6000. } else { 800. };
        let bounds = model.snapshot().occurrences[&instance].bounds;
        for (actual, expected) in bounds
            .min
            .into_iter()
            .zip([5000. - spread / 2., 6000. - spread / 2., 200.])
            .chain(bounds.max.into_iter().zip([
                5000. + spread / 2.,
                6000. + spread / 2.,
                200. + height,
            ]))
        {
            assert!(
                (actual - expected).abs() < 1e-5,
                "native envelope {actual} vs {expected}"
            );
        }
        for denominator in [50, 100] {
            let v = view(instance, denominator);
            let result = model
                .drawing_view(s.document(), s.revision(), &v, &DrawingProviders::default())
                .unwrap();
            let ViewEvaluation::Ready { vectors, .. } = result else {
                panic!("not ready")
            };
            assert_eq!(vectors.len(), if kind == PlantKind::Tree { 2 } else { 1 });
            let expected = if kind == PlantKind::Tree {
                4000.
            } else {
                1000.
            };
            let outer=vectors.iter().find(|v|matches!(v.curve,VectorCurve::Ellipse{major_mm,..} if (major_mm*2.-expected).abs()<1e-7)).unwrap();
            let VectorCurve::Ellipse {
                center,
                major_mm,
                minor_mm,
                ..
            } = outer.curve
            else {
                unreachable!()
            };
            assert_eq!(center, [5000., 6000.]);
            assert!((major_mm - minor_mm).abs() < 1e-8);
            assert_eq!(outer.role, VectorRole::Visible);
            assert_eq!(
                major_mm * 2. / denominator as f64,
                expected / denominator as f64
            );
        }
    }
}
#[test]
fn dimension_edits_visibility_and_history_share_the_original_pin() {
    let (mut s, request) = placed(PlantKind::Tree);
    let pin = multicad_core::symbols::read_instance(s.document(), request.placement.record)
        .unwrap()
        .pin;
    let mut settings = request.settings.clone();
    settings.plan_visible = false;
    s.commit(
        plants::prepare_edit(
            &s,
            s.revision(),
            actor(),
            request.placement.record,
            5000.,
            3000.,
            settings,
        )
        .unwrap(),
    )
    .unwrap();
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let v = view(request.placement.instance, 100);
    assert!(
        model
            .drawing_view(s.document(), s.revision(), &v, &DrawingProviders::default())
            .is_err()
    );
    let mut section = v.clone();
    section.kind = ViewKind::Section {
        offset_mm: 0.,
        range: SectionRange::Unlimited,
        window: SectionWindow::default(),
    };
    let ViewEvaluation::Ready { vectors, .. } = model
        .drawing_view(
            s.document(),
            s.revision(),
            &section,
            &DrawingProviders::default(),
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(!vectors.is_empty());
    assert!(vectors.iter().any(|v| v.role == VectorRole::Section));
    let mut elevation = v.clone();
    elevation.frame = ReferenceFrame {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
    };
    assert!(
        model
            .drawing_view(
                s.document(),
                s.revision(),
                &elevation,
                &DrawingProviders::default()
            )
            .is_ok()
    );
    s.undo(s.revision(), actor()).unwrap();
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    assert!(
        model
            .drawing_view(s.document(), s.revision(), &v, &DrawingProviders::default())
            .is_ok()
    );
    assert_eq!(
        multicad_core::symbols::read_instance(s.document(), request.placement.record)
            .unwrap()
            .pin,
        pin
    );
}
#[test]
fn resized_and_moved_plant_updates_solved_plan_label_and_native_height_together() {
    let (mut s, placed) = placed(PlantKind::Tree);
    let record = placed.placement.record;
    let request = plants::PlantEditRequest {
        record,
        height_mm: 7000.,
        spread_mm: 3000.,
        name: "T1 moved".into(),
        settings: plants::PlantInstance {
            symbol_instance: record,
            size_basis: plants::SizeBasis::Current,
            label: Some("T1".into()),
            plan_visible: true,
        },
        move_by: plants::planar_move(
            s.document(),
            s.parameters(),
            record,
            [7000., 8000., 300.],
            45.,
        )
        .unwrap(),
    };
    s.commit(plants::prepare_edit_request(&s, s.revision(), actor(), &request).unwrap())
        .unwrap();
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let ViewEvaluation::Ready {
        vectors, labels, ..
    } = model
        .drawing_view(
            s.document(),
            s.revision(),
            &view(placed.placement.instance, 100),
            &DrawingProviders::default(),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(vectors.len(), 2);
    assert!(vectors.iter().any(|v|matches!(v.curve,VectorCurve::Ellipse{center,major_mm,..} if center==[7000.,8000.]&&(major_mm-1500.).abs()<1e-7)));
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].text, "T1");
    assert_eq!(labels[0].at_mm, [7000., 8000.]);
    let bounds = model.snapshot().occurrences[&placed.placement.instance].bounds;
    assert!((bounds.min[2] - 300.).abs() < 1e-5);
    assert!((bounds.max[2] - 7300.).abs() < 1e-5);
}
