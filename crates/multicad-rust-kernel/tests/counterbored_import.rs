use multicad_rust_kernel::{
    Circle, Rational,
    chamfered_bores::{ChamferedBores, Counterbore},
    polygon::PolygonPrism,
    step::Writer,
    step_read::{ImportProfile, read_import_solid},
};

#[test]
fn production_reader_retains_counterbore_topology_and_all_finite_face_keys() {
    let q = |n| Rational::new(n, 1).unwrap();
    let stock = PolygonPrism::with_circular_holes(
        vec![
            vec![[0, 0], [20, 0], [20, 16], [0, 16]]
                .into_iter()
                .map(|p| p.map(q))
                .collect(),
        ],
        vec![Circle::new([q(10), q(8)], q(1)).unwrap()],
        q(6),
    )
    .unwrap();
    for both in [false, true] {
        let mut cuts = vec![Counterbore {
            hole: 0,
            end: false,
            radius_mm: 3.,
            depth_mm: 1.,
        }];
        if both {
            cuts.push(Counterbore {
                hole: 0,
                end: true,
                radius_mm: 2.,
                depth_mm: 1.5,
            });
        }
        let body = ChamferedBores::counterbored(&stock, &cuts, 1e-7, 1e-5).unwrap();
        for [u, v] in [
            [[1., 0., 0.], [0., 1., 0.]],
            [[0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
        ] {
            for reverse in [false, true] {
                let mut writer = Writer::new();
                writer
                    .append_treated_polygon_prism(&stock, &body, [40., -8., 12.], u, v, reverse)
                    .unwrap();
                let bytes = writer.finish().unwrap();
                let (read, solid) = read_import_solid(&bytes, 1e-7).unwrap();
                assert!((read.volume_mm3 - body.volume_mm3()).abs() < 1e-8);
                let ImportProfile::CounterboredPolygon {
                    counterbores,
                    source_faces,
                    ..
                } = solid.profile
                else {
                    panic!("production classification lost seats");
                };
                assert_eq!(counterbores.len(), cuts.len());
                assert_eq!(source_faces.len(), 7 + 2 * cuts.len());
                assert!(source_faces.iter().all(|key| !key.is_empty()));
                let distinct: std::collections::BTreeSet<_> = source_faces.iter().collect();
                assert_eq!(distinct.len(), source_faces.len());
            }
        }
    }
}
