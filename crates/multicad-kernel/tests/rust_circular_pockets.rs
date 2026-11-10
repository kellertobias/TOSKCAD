use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CircularPocketTool, CurvedEdge2d, CurvedProfile, GeometryPolicy, MeshOptions,
    PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, polygon::PolygonPrism},
};
use std::f64::consts::{PI, TAU};

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

#[test]
fn opposite_blind_drills_keep_exact_floors_rims_references_and_step() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let drill = EntityId::new();
    let frame = PlaneFrame {
        origin: [10., 20., 30.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let corners = [[0, 0], [30, 0], [30, 20], [0, 20]];
    let exact = |n| Rational::new(n, 1).unwrap();
    let origins: Vec<_> = (0..5)
        .map(|i| Origin {
            source: owner,
            role: format!("stock/{i}"),
        })
        .collect();
    let stock = Solid::from_polygon_prism_sweep(
        PolygonPrism::with_circular_holes(
            vec![corners.map(|p| p.map(exact)).to_vec()],
            vec![Circle::new([exact(5); 2], exact(1)).unwrap()],
            exact(12),
        )
        .unwrap(),
        frame,
        true,
        owner,
        &origins,
        &[false; 5],
        policy,
    )
    .unwrap();
    let shape = CurvedProfile {
        frame,
        outer: corners
            .into_iter()
            .zip(corners.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line {
                a: a.map(|i| i as f64),
                b: b.map(|i| i as f64),
            })
            .collect(),
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [5.; 2],
            radius_mm: 1.,
        }]],
    };
    let native =
        Solid::extrude_curved_profile_vector_named(&shape, [-12., 0., 0.], owner, &origins, policy)
            .unwrap();
    let tools: [_; 2] = std::array::from_fn(|i| CircularPocketTool {
        frame: PlaneFrame {
            origin: [[11., 20., 30.], [-3., 20., 30.]][i],
            ..frame
        },
        center_uv: [12., 10.],
        radius_mm: 2.,
        displacement_mm: [[-5., 0., 0.], [4., 0., 0.]][i],
        origin: Origin {
            source: drill,
            role: format!("drill/{i}"),
        },
    });
    let staged = stock
        .rust_drill_circular_pockets(&tools[..1], policy)
        .unwrap();
    let staged_topology = staged.topology_snapshot(owner, 0).unwrap();
    let floor = staged_topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins
                    .iter()
                    .any(|origin| origin.role.starts_with("pocket.floor/"))
        })
        .unwrap();
    let intent = GeometryIntent::capture(&staged_topology, floor.key).unwrap();
    let rust = staged
        .rust_drill_circular_pockets(&tools[1..], policy)
        .unwrap();
    let native = tools.iter().fold(native, |body, tool| {
        let cutter = Solid::extrude_curved_profile_vector(
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
        body.boolean(&cutter, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap()
    });
    let volume = 7200. - 40. * PI;
    near(rust.volume_mm3().unwrap(), volume, 1e-7);
    near(native.volume_mm3().unwrap(), volume, 1e-7);
    let native_faces = native.faces().unwrap();
    assert_eq!(rust.faces().unwrap().len(), native_faces.len());
    for face in rust.faces().unwrap() {
        assert!(
            native_faces.iter().any(|expected| {
                (face.area_mm2 - expected.area_mm2).abs() < 1e-7
                    && face
                        .center_mm
                        .into_iter()
                        .zip(expected.center_mm)
                        .all(|(a, b)| (a - b).abs() < 1e-7)
            }),
            "unexpected face {face:?}"
        );
    }
    let topology = rust.topology_snapshot(owner, 1).unwrap();
    assert!(
        matches!(resolve(&intent, &topology, 1), Resolution::Resolved { key, .. } if key == floor.key)
    );
    let [top, floor] = [0, floor.key.ordinal].map(|face| {
        let element = topology
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == face)
            .unwrap();
        let frame = element.frames[0].frame;
        rust.planar_face_contours(
            face,
            PlaneFrame {
                origin: frame.origin_mm,
                u: frame.u,
                v: frame.v,
            },
        )
        .unwrap()
    });
    assert_eq!(top.loops.len(), 3);
    near(floor.area_mm2(), 4. * PI, 1e-8);
    assert_eq!(
        topology
            .elements
            .iter()
            .filter(|e| matches!(e.geometry,
        Some(AnalyticGeometry::CircularArc { sweep_rad, .. }) if sweep_rad == TAU))
            .count(),
        6
    );
    let receipt = rust
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert!(receipt.edge_pairs > 0 && receipt.vertex_links > 0);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pockets.step");
    rust.write_step(&path).unwrap();
    near(
        Solid::read_step(&path, policy)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        volume,
        1e-7,
    );
    let native_path = dir.path().join("native-pockets.step");
    native.write_step(&native_path).unwrap();
    for path in [path, native_path] {
        let imported =
            Solid::import_rust_step(&std::fs::read_to_string(path).unwrap(), owner, policy)
                .unwrap();
        near(imported.volume_mm3().unwrap(), volume, 1e-7);
        assert_eq!(imported.faces().unwrap().len(), native_faces.len());
    }

    let crossing = CircularPocketTool {
        displacement_mm: [-14., 0., 0.],
        ..tools[0].clone()
    };
    assert!(
        stock
            .rust_drill_circular_pockets(&[crossing], policy)
            .is_err()
    );
    let intersecting = CircularPocketTool {
        displacement_mm: [10., 0., 0.],
        ..tools[1].clone()
    };
    assert!(
        staged
            .rust_drill_circular_pockets(&[intersecting], policy)
            .is_err()
    );
}
