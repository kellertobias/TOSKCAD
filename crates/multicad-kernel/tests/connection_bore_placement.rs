use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    CircularPocketTool, GeometryErrorKind, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform,
    Solid,
};
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn vector(pose: RigidTransform, p: [f64; 3]) -> [f64; 3] {
    pose.rotation
        .map(|row| row.into_iter().zip(p).map(|(a, b)| a * b).sum())
}
fn point(pose: RigidTransform, p: [f64; 3]) -> [f64; 3] {
    let q = vector(pose, p);
    std::array::from_fn(|i| q[i] + pose.translation_mm[i])
}
fn tool(pose: RigidTransform, roll: f64, tilt: f64) -> CircularPocketTool {
    let (s, c) = roll.sin_cos();
    let (st, ct) = tilt.sin_cos();
    let frame = PlaneFrame {
        origin: point(pose, [10., 10., 4.10001]),
        u: vector(pose, [c * ct, s * ct, st]),
        v: vector(pose, [s, -c, 0.]),
    };
    CircularPocketTool {
        frame,
        center_uv: [0.; 2],
        radius_mm: 1.7,
        displacement_mm: frame.normal().map(|v| v * 4.10002),
        origin: Origin {
            source: EntityId::new(),
            role: "receiving.round".into(),
        },
    }
}
#[test]
fn full_round_tools_preserve_actual_bores_after_proper_and_far_placement() {
    let policy = GeometryPolicy::default();
    for translation_mm in [[1000., -2000., 3000.], [1e6, -2e6, 3e6]] {
        let pose = RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm,
        };
        let stock = Solid::box_solid([20., 20., 4.1], policy)
            .unwrap()
            .transformed(pose)
            .unwrap();
        let owner = EntityId::new();
        let before = stock.topology_snapshot(owner, 0).unwrap();
        for roll in [0., 0.37, 1.4] {
            let (cut, mouths) = stock
                .rust_connection_through_bore(&tool(pose, roll, 0.), 1., policy)
                .unwrap();
            for (actual, local) in mouths.into_iter().zip([[10., 10., 4.1], [10., 10., 0.]]) {
                for (a, b) in actual.into_iter().zip(point(pose, local)) {
                    near(a, b);
                }
            }
            let volume = 1640. - std::f64::consts::PI * 1.7_f64.powi(2) * 4.1;
            near(cut.volume_mm3().unwrap(), volume);
            assert_eq!(cut.bounds().unwrap(), stock.bounds().unwrap());
            let mesh = cut
                .manufacturing_mesh_with_receipt(MeshOptions {
                    linear_deflection_mm: 0.001,
                    angular_deflection_rad: 0.05,
                    maximum_triangles: 10000,
                })
                .unwrap();
            assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("posed-bore.step");
            cut.write_step(&path).unwrap();
            let loaded = Solid::read_step(&path, policy).unwrap();
            assert!((loaded.volume_mm3().unwrap() - volume).abs() < 1e-5);
            assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
        }
        let failure = stock
            .rust_connection_through_bore(&tool(pose, 0.37, 1e-4), 1., policy)
            .err()
            .unwrap();
        assert_eq!(failure.kind, GeometryErrorKind::Unsupported);
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    }
}
