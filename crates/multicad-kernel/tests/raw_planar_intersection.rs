use multicad_core::document::EntityId;
use multicad_kernel::{
    rust_kernel::{
        planar_boolean::{ConvexPiece, CutMaterial, Union},
        step,
    },
    *,
};
const TOL: f64 = 1e-7;
fn block(min: [f64; 3], max: [f64; 3]) -> CutMaterial {
    let piece = ConvexPiece::from_box(min, max, TOL).unwrap();
    let boundary = Union::new(std::slice::from_ref(&piece), TOL).unwrap();
    CutMaterial::certify_existing(&boundary, &[piece], TOL).unwrap()
}
#[test]
fn raw_restored_partition_intersection_matches_native_booleans_and_step_geometry() {
    let [a, b, c, d] = [[0.; 3], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
    let piece = ConvexPiece::new(
        vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]],
        TOL,
    )
    .unwrap();
    let boundary = Union::new(std::slice::from_ref(&piece), TOL).unwrap();
    let tetra = CutMaterial::certify_existing(&boundary, &[piece], TOL).unwrap();
    let first = block([1., 0., 0.], [5., 1., 1.]);
    let second = block([1.5, 0., 0.], [4., 1., 1.]);
    let slot = block([1.5, 0., 0.], [1.75, 1., 1.]);
    let last = block([1.75, 0., 0.], [4., 1., 1.]);
    let common = CutMaterial::intersect_certified(&[&tetra, &first, &second], TOL)
        .unwrap()
        .unwrap();
    let cut = CutMaterial::subtract_certified(&[&common, &slot], TOL)
        .unwrap()
        .unwrap();
    let restored = CutMaterial::union_certified(&[&cut, &slot], TOL).unwrap();
    let raw = Union::intersect(restored.pieces(), last.pieces(), TOL)
        .unwrap()
        .unwrap();
    assert!((raw.volume_mm3() - 1.25).abs() < 1e-8);
    let text = step::write_planar_union(&raw, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let policy = GeometryPolicy::default();
    let read = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
    assert!((read.volume_mm3().unwrap() - 1.25).abs() < 1e-8);
    read.manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    if OCCT_LINKED {
        let folder = tempfile::tempdir().unwrap();
        let native = |name: &str, geometry: &Union| {
            let path = folder.path().join(format!("{name}.step"));
            std::fs::write(
                &path,
                step::write_planar_union(geometry, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap(),
            )
            .unwrap();
            Solid::read_step(&path, policy).unwrap()
        };
        let stock = native("tetra", tetra.boundary());
        let a = native("first", first.boundary());
        let b = native("second", second.boundary());
        let cutter = native("slot", slot.boundary());
        let crop = native("last", last.boundary());
        let common = stock
            .boolean(&a, BooleanOperation::Intersect, policy)
            .unwrap()
            .unwrap()
            .boolean(&b, BooleanOperation::Intersect, policy)
            .unwrap()
            .unwrap();
        let cut = common
            .boolean(&cutter, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap();
        let restored = cut
            .boolean(&cutter, BooleanOperation::Union, policy)
            .unwrap()
            .unwrap();
        let expected = restored
            .boolean(&crop, BooleanOperation::Intersect, policy)
            .unwrap()
            .unwrap();
        let exported = native("raw-result", &raw);
        for solid in [&expected, &exported] {
            assert!((solid.volume_mm3().unwrap() - 1.25).abs() < 1e-7);
            let bounds = solid.bounds().unwrap();
            for (actual, expected) in bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip(raw.bounds()[0].into_iter().chain(raw.bounds()[1]))
            {
                assert!((actual - expected).abs() < 1e-7);
            }
            assert_eq!(solid.solid_count().unwrap(), 1);
        }
    }
}
