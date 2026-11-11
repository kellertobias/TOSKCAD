use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, HelicalThread, PlaneFrame, RigidTransform, Solid,
};
use std::collections::BTreeSet;

#[test]
fn nested_compound_preserves_each_thread_curve_and_its_global_topology_ordinal() {
    let policy = GeometryPolicy::default();
    let source = Solid::rust_helical_thread(
        HelicalThread {
            major_radius_mm: 4.,
            pitch_mm: 1.25,
            length_mm: 3.7,
            depth_mm: 0.65,
            crest_width_mm: 0.15,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            starts: 2,
            left_hand: true,
        },
        None,
        EntityId::new(),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let direct = source.display_edges(0.03).unwrap();
    let topology = source.topology_snapshot(EntityId::new(), 0).unwrap();
    assert!(
        topology
            .elements
            .iter()
            .any(|element| element.key.kind == TopologyKind::Edge && element.geometry.is_none())
    );
    let pair = Solid::compound_named(&[&source, &source], EntityId::new(), policy).unwrap();
    let moved = source
        .transformed(RigidTransform {
            translation_mm: [20., 3., -5.],
            ..Default::default()
        })
        .unwrap();
    let owner = EntityId::new();
    let nested = Solid::compound_named(&[&pair, &moved], owner, policy).unwrap();
    let published = nested.topology_with_display(owner, 7, Some(0.03)).unwrap();
    let standalone = nested.display_edges(0.03).unwrap();
    assert_eq!(published.topology.revision, 7);
    assert_eq!(published.edges.len(), 3 * direct.len());
    assert_eq!(standalone.len(), published.edges.len());
    let keys: BTreeSet<_> = published
        .topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .map(|e| e.key.ordinal)
        .collect();
    assert_eq!(keys, published.edges.iter().map(|e| e.index).collect());
    for (index, (actual, separate)) in published.edges.iter().zip(&standalone).enumerate() {
        assert_eq!(actual.index, separate.index);
        assert_eq!(actual.points_mm, separate.points_mm);
        let expected = &direct[index % direct.len()];
        assert_eq!(actual.points_mm.len(), expected.points_mm.len());
        for (actual, expected) in actual.points_mm.iter().zip(&expected.points_mm) {
            let offset = if index < 2 * direct.len() {
                [0.; 3]
            } else {
                [20., 3., -5.]
            };
            for axis in 0..3 {
                assert!((actual[axis] - expected[axis] - offset[axis]).abs() < 1e-12);
            }
        }
    }
    assert_eq!(
        nested.display_edges(0.).unwrap_err().kind,
        GeometryErrorKind::Input
    );
}
