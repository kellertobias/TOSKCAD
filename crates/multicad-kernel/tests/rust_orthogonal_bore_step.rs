use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    BooleanOperation, CircularPocketTool, GeometryPolicy, PlaneFrame, RigidTransform, Solid,
    rust_kernel::Rational,
};

#[test]
fn independent_native_reader_keeps_both_rims_and_the_blind_floor_in_placed_stock() {
    let policy = GeometryPolicy::default();
    let size = [20, 16, 10];
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|v| Rational::new(v, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let tools = [
        (
            PlaneFrame {
                origin: [-1., 5., 4.],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            1.5,
            22.,
        ),
        (
            PlaneFrame {
                origin: [16., 12., 11.],
                v: [0., -1., 0.],
                ..Default::default()
            },
            2.,
            4.,
        ),
    ]
    .map(|(frame, radius_mm, length)| CircularPocketTool {
        frame,
        center_uv: [0.; 2],
        radius_mm,
        displacement_mm: frame.normal().map(|component| component * length),
        origin: Origin {
            source: EntityId::new(),
            role: "cut".into(),
        },
    });
    let rust = stock
        .rust_drill_circular_machining(&tools, &[true, false], policy)
        .unwrap();
    assert!(rust.supports_step_export());
    let mut native = Solid::box_solid(size.map(|v| v as f64), policy).unwrap();
    for (tool, length) in tools.iter().zip([22., 4.]) {
        let cutter = Solid::cylinder(tool.frame, tool.radius_mm, length, policy).unwrap();
        native = native
            .boolean(&cutter, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap();
    }
    let pose = RigidTransform {
        rotation: [[0.36, -0.8, 0.48], [0.48, 0.6, 0.64], [-0.8, 0., 0.6]],
        translation_mm: [19., -23., 41.],
    };
    let [rust, native] = [rust, native].map(|s| s.transformed(pose).unwrap());
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("drilled.step");
    rust.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, policy).unwrap();
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "{a} != {b}");
    assert_eq!(readback.solid_count().unwrap(), 1);
    for solid in [&rust, &readback] {
        near(solid.volume_mm3().unwrap(), native.volume_mm3().unwrap());
        assert_eq!(solid.faces().unwrap().len(), native.faces().unwrap().len());
        let area = |s: &Solid| s.faces().unwrap().iter().map(|face| face.area_mm2).sum();
        near(area(solid), area(&native));
        let [a, b] = [solid.bounds().unwrap(), native.bounds().unwrap()];
        for (x, y) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            near(x, y);
        }
    }
}
