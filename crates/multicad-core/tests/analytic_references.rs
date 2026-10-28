use multicad_core::{document::EntityId, references::*};
use std::collections::BTreeSet;

#[test]
fn invalid_or_mismatched_exact_geometry_never_enters_a_reference_snapshot() {
    let key = TopologyKey {
        kind: TopologyKind::Edge,
        ordinal: 0,
    };
    let mut snapshot = TopologySnapshot {
        owner: EntityId::new(),
        revision: 0,
        elements: vec![TopologyElement {
            key,
            origins: BTreeSet::new(),
            boundary: BTreeSet::new(),
            frames: vec![],
            geometry: Some(AnalyticGeometry::Line {
                a_mm: [0.; 3],
                b_mm: [0.; 3],
            }),
        }],
    };
    assert!(snapshot.validate().is_err());
    snapshot.elements[0].geometry = Some(AnalyticGeometry::Point {
        position_mm: [1., 2., 3.],
    });
    assert!(snapshot.validate().unwrap_err().contains("kind"));
    snapshot.elements[0].geometry = None;
    snapshot.validate().unwrap();
    let old = serde_json::to_value(&snapshot).unwrap();
    assert!(old["elements"][0].get("geometry").is_none());
    let reopened: TopologySnapshot = serde_json::from_value(old).unwrap();
    assert_eq!(reopened, snapshot);
}

#[test]
fn rigid_occurrence_descriptor_maps_points_and_directions_separately() {
    let frame = ReferenceFrame {
        origin_mm: [1., 2., 3.],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let geometry = AnalyticGeometry::CircularArc {
        frame,
        radius_mm: 4.,
        start_rad: 0.5,
        sweep_rad: 1.,
    };
    let rotate = |[x, y, z]: [f64; 3]| [-y, x, z];
    let moved = geometry.transformed(
        |p| {
            let [x, y, z] = rotate(p);
            [x + 10., y + 20., z + 30.]
        },
        rotate,
    );
    moved.validate().unwrap();
    assert_eq!(
        moved,
        AnalyticGeometry::CircularArc {
            frame: ReferenceFrame {
                origin_mm: [8., 21., 33.],
                u: [0., 1., 0.],
                v: [-1., 0., 0.]
            },
            radius_mm: 4.,
            start_rad: 0.5,
            sweep_rad: 1.
        }
    );
}
