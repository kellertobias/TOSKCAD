use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    *,
};

fn stock(frame: PlaneFrame) -> Solid {
    let q = |a, b| Rational::new(a, b).unwrap();
    let id = EntityId::new();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([q(0, 1); 2], q(6, 1)).unwrap(),
            q(0, 1),
            q(21, 5),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: id,
            role: "circle".into(),
        },
        Default::default(),
    )
    .unwrap()
}
fn spec(length_mm: f64) -> HelicalThread {
    HelicalThread {
        major_radius_mm: 3.175,
        pitch_mm: 1.,
        length_mm,
        depth_mm: 0.54,
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    }
}
#[test]
fn inferred_through_stock_retains_the_exact_shared_thread_interval() {
    let frame = PlaneFrame {
        origin: [17., -8., 13.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    };
    let source = stock(frame)
        .rust_cut_internal_thread(spec(4.2), 0.1, EntityId::new(), frame, Default::default())
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.step");
    source.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let (_, decoded) = rust_kernel::step_read::read_import_solid(&text, 1e-7).unwrap();
    assert!(matches!(
        decoded.profile,
        rust_kernel::step_read::ImportProfile::HelicalThread {
            stock_radius_mm: Some(_),
            ..
        }
    ));
    let imported = Solid::import_rust_step(&text, EntityId::new(), Default::default()).unwrap();
    let interval = imported.rust_receiving_thread_intervals().unwrap();
    assert_eq!(interval.stock_length_mm, interval.physical_thread_length_mm);
    assert_eq!(
        source
            .topology_snapshot(EntityId::new(), 0)
            .unwrap()
            .elements
            .len(),
        imported
            .topology_snapshot(EntityId::new(), 0)
            .unwrap()
            .elements
            .len()
    );
    let again = dir.path().join("again.step");
    imported.write_step(&again).unwrap();
    let reopened = Solid::import_rust_step(
        &std::fs::read_to_string(&again).unwrap(),
        EntityId::new(),
        Default::default(),
    )
    .unwrap();
    let next = reopened.rust_receiving_thread_intervals().unwrap();
    assert_eq!(next.stock_length_mm, next.physical_thread_length_mm);
    assert!((source.volume_mm3().unwrap() - reopened.volume_mm3().unwrap()).abs() < 1e-6);
}
#[test]
fn genuinely_short_floor_is_not_reinterpreted_as_through_material() {
    use rust_kernel::{
        helical_receiver::Receiver,
        helical_thread::{Spec, Thread},
    };
    let thread = Thread::new(Spec {
        major_radius_mm: 1.,
        pitch_mm: 0.1,
        length_mm: 1. - 1e-6,
        depth_mm: 0.054,
        crest_width_mm: 0.0125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts: 1,
        left_hand: false,
        root_radius_mm: None,
    })
    .unwrap();
    let receiver = Receiver::new(thread, 1.5, 1.).unwrap();
    let text = rust_kernel::step::write_helical_receiver(
        &receiver,
        [0.; 3],
        [1., 0., 0.],
        [0., 1., 0.],
        1e-7,
    )
    .unwrap();
    let (_, decoded) = rust_kernel::step_read::read_import_solid(&text, 1e-7).unwrap();
    assert!(matches!(
        decoded.profile,
        rust_kernel::step_read::ImportProfile::HelicalReceiver { .. }
    ));
    let error = Solid::import_rust_step(&text, EntityId::new(), Default::default())
        .err()
        .expect("subminimum physical floor must refuse");
    assert_eq!(error.kind, GeometryErrorKind::Input);
    // A refused imported interval does not poison later genuine through imports.
    let through = Solid::rust_helical_thread(
        spec(1.),
        None,
        EntityId::new(),
        PlaneFrame::default(),
        Default::default(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retry.step");
    through.write_step(&path).unwrap();
    assert!(
        Solid::import_rust_step(
            &std::fs::read_to_string(path).unwrap(),
            EntityId::new(),
            Default::default()
        )
        .is_ok()
    );
}
