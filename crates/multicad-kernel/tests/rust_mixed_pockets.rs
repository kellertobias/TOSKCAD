use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CircularPocketTool, CurvedEdge2d, CurvedProfile, GeometryPolicy, MeshOptions,
    PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Rational, display::DisplayMesh, manufacturing::certify},
};
use std::f64::consts::PI;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

#[test]
fn blind_tools_preserve_grooves_and_openings_on_multiple_stock_faces() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let feature = EntityId::new();
    let rust_box = |id: EntityId, size: [i64; 3], origin| {
        Solid::rust_box(
            id.uuid_bits(),
            size.map(|extent| Rational::new(extent, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let native_box = |size, translation_mm| {
        Solid::box_solid(size, policy)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm,
                ..Default::default()
            })
            .unwrap()
    };
    let rust = rust_box(owner, [20, 16, 12], [0.; 3])
        .rust_subtract_orthogonal_tools(
            &[rust_box(EntityId::new(), [3, 18, 5], [5., -1., 9.])],
            policy,
        )
        .unwrap()
        .unwrap();
    let native = native_box([20., 16., 12.], [0.; 3])
        .boolean(
            &native_box([3., 18., 5.], [5., -1., 9.]),
            BooleanOperation::Subtract,
            policy,
        )
        .unwrap()
        .unwrap();
    let tools = [
        CircularPocketTool {
            frame: PlaneFrame {
                origin: [0., 0., 13.],
                ..Default::default()
            },
            center_uv: [13., 5.],
            radius_mm: 2.,
            displacement_mm: [0., 0., -5.],
            origin: Origin {
                source: feature,
                role: "top".into(),
            },
        },
        CircularPocketTool {
            frame: PlaneFrame {
                origin: [-1., 0., 0.],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            center_uv: [10., 6.],
            radius_mm: 1.5,
            displacement_mm: [5., 0., 0.],
            origin: Origin {
                source: feature,
                role: "side".into(),
            },
        },
        CircularPocketTool {
            frame: PlaneFrame {
                origin: [0., 0., 13.],
                ..Default::default()
            },
            center_uv: [6.5, 12.],
            radius_mm: 1.,
            displacement_mm: [0., 0., -7.],
            origin: Origin {
                source: feature,
                role: "groove".into(),
            },
        },
    ];
    let (sin, cos) = 0.53_f64.sin_cos();
    let pose = RigidTransform {
        rotation: [[cos, -sin, 0.], [sin, cos, 0.], [0., 0., 1.]],
        translation_mm: [1_000_000., -600_000., 200.],
    };
    let rotate = |vector: [f64; 3]| {
        pose.rotation
            .map(|row| row.into_iter().zip(vector).map(|(a, b)| a * b).sum::<f64>())
    };
    let rotated_tools = tools.clone().map(|mut tool| {
        let origin = rotate(tool.frame.origin);
        tool.frame = PlaneFrame {
            origin: std::array::from_fn(|i| origin[i] + pose.translation_mm[i]),
            u: rotate(tool.frame.u),
            v: rotate(tool.frame.v),
        };
        tool.displacement_mm = rotate(tool.displacement_mm);
        tool
    });
    let rotated = rust
        .transformed(pose)
        .unwrap()
        .rust_drill_circular_pockets(&rotated_tools, policy)
        .unwrap();
    let first = rust
        .rust_drill_circular_pockets(&tools[..1], policy)
        .unwrap();
    let topology = first.topology_snapshot(owner, 1).unwrap();
    let floor = topology
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face
                && element.origins.iter().any(|origin| {
                    origin.role.ends_with("/top") && origin.role.starts_with("pocket.floor/")
                })
        })
        .unwrap();
    let intent = GeometryIntent::capture(&topology, floor.key).unwrap();
    let rust = first
        .rust_drill_circular_pockets(&tools[1..], policy)
        .unwrap();
    assert!(
        matches!(resolve(&intent, &rust.topology_snapshot(owner, 2).unwrap(), 2),
        Resolution::Resolved { key, .. } if key == floor.key)
    );
    let native = tools.iter().fold(native, |body, tool| {
        let tool_shape = Solid::extrude_curved_profile_vector(
            &CurvedProfile {
                frame: tool.frame,
                outer: vec![CurvedEdge2d::Circle {
                    center: tool.center_uv,
                    radius_mm: tool.radius_mm,
                }],
                holes: vec![],
            },
            tool.displacement_mm,
            policy,
        )
        .unwrap();
        body.boolean(&tool_shape, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap()
    });
    let volume = 20. * 16. * 12. - 3. * 16. * 3. - (16. + 9. + 3.) * PI;
    near(rust.volume_mm3().unwrap(), volume);
    near(native.volume_mm3().unwrap(), volume);
    near(rotated.volume_mm3().unwrap(), volume);
    let [a, b] = [rust.bounds().unwrap(), native.bounds().unwrap()];
    for (actual, expected) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(actual, expected);
    }
    let expected = native.faces().unwrap();
    assert_eq!(rust.faces().unwrap().len(), expected.len());
    for face in rust.faces().unwrap() {
        assert!(
            expected.iter().any(|native| {
                (native.area_mm2 - face.area_mm2).abs() < 2e-6
                    && native
                        .center_mm
                        .into_iter()
                        .zip(face.center_mm)
                        .all(|(a, b)| (a - b).abs() < 2e-6)
            }),
            "{face:?}"
        );
    }
    let floor = &rust.faces().unwrap()[floor.key.ordinal as usize];
    near(floor.area_mm2, 4. * PI);
    let placement = PlaneFrame {
        origin: [13., 5., 8.],
        ..Default::default()
    };
    near(
        rust.planar_face_contours(floor.index, placement)
            .unwrap()
            .area_mm2(),
        4. * PI,
    );
    let receipt = rust
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    let mesh = receipt.mesh;
    assert_eq!(
        certify(&DisplayMesh {
            vertices: mesh.vertices,
            indices: mesh.indices,
            face_indices: mesh.face_indices,
        })
        .unwrap()
        .edge_pairs,
        receipt.edge_pairs
    );

    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("grooves-and-pockets.step");
    rust.write_step(&path).unwrap();
    near(
        Solid::read_step(&path, policy)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        volume,
    );
    let native_path = scratch.path().join("native-grooves-and-pockets.step");
    native.write_step(&native_path).unwrap();
    let rotated_path = scratch.path().join("rotated-grooves-and-pockets.step");
    rotated.write_step(&rotated_path).unwrap();
    for path in [path, native_path, rotated_path] {
        let imported =
            Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy)
                .unwrap();
        near(imported.volume_mm3().unwrap(), volume);
        assert_eq!(imported.faces().unwrap().len(), expected.len());
    }

    let too_wide = CircularPocketTool {
        radius_mm: 2.,
        ..tools[2].clone()
    };
    assert!(
        first
            .rust_drill_circular_pockets(&[too_wide], policy)
            .is_err()
    );
    assert!(
        first
            .rust_drill_circular_pockets(&tools[..1], policy)
            .is_err()
    );
}
