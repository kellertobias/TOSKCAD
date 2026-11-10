use multicad_core::{
    document::EntityId,
    manufacturing::PlanarEdge,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CircularPocketTool, CurvedEdge2d, CurvedProfile, GeometryPolicy, MeshOptions,
    PlaneFrame, RoundedSlotTool, Solid, rust_kernel::Rational,
};
use std::f64::consts::PI;

const AREA: f64 = 16. + 4. * PI;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

fn compare(actual: &Solid, expected: &Solid) {
    near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
    let [a, b] = [actual.bounds().unwrap(), expected.bounds().unwrap()];
    for (actual, expected) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(actual, expected);
    }
    assert_eq!(
        actual.faces().unwrap().len(),
        expected.faces().unwrap().len()
    );
}

fn profile(frame: PlaneFrame) -> CurvedProfile {
    let [a, b, c, d] = [[0., -2.], [4., -2.], [4., 2.], [0., 2.]];
    CurvedProfile {
        frame,
        outer: vec![
            CurvedEdge2d::Line { a, b },
            CurvedEdge2d::Arc {
                center: [4., 0.],
                a: b,
                b: c,
                clockwise: false,
            },
            CurvedEdge2d::Line { a: c, b: d },
            CurvedEdge2d::Arc {
                center: [0., 0.],
                a: d,
                b: a,
                clockwise: false,
            },
        ],
        holes: vec![],
    }
}

fn exact_cap(shape: &Solid, index: u32, frame: PlaneFrame) {
    let cap = shape.planar_face_contours(index, frame).unwrap();
    near(cap.area_mm2(), AREA);
    assert_eq!(cap.loops.len(), 1);
    assert_eq!(cap.loops[0].edges.len(), 4);
    let arcs: Vec<_> = cap.loops[0]
        .edges
        .iter()
        .filter_map(|edge| match edge {
            PlanarEdge::Arc { sweep_rad, .. } => Some(sweep_rad.abs()),
            _ => None,
        })
        .collect();
    assert_eq!(arcs.len(), 2);
    for sweep in arcs {
        near(sweep, PI);
    }
}

fn exchange(shape: &Solid, expected: &Solid, policy: GeometryPolicy) {
    shape
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("rounded.step");
    shape.write_step(&path).unwrap();
    compare(&Solid::read_step(&path, policy).unwrap(), expected);
}

#[test]
fn tilted_tenon_rebuilds_both_sweep_directions_and_exchanges_analytic_faces() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let frame = PlaneFrame {
        origin: [11., -6., 9.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    for height in [5., -5.] {
        let rust = Solid::rust_extrude_rounded_slot(
            frame,
            [[0., 0.], [4., 0.]],
            2.,
            height,
            feature,
            std::array::from_fn(|edge| Origin {
                source: feature,
                role: format!("profile.{edge}"),
            }),
            policy,
        )
        .unwrap();
        let native =
            Solid::extrude_curved_profile_vector(&profile(frame), [height, 0., 0.], policy)
                .unwrap();
        near(rust.volume_mm3().unwrap(), AREA * height.abs());
        compare(&rust, &native);
        let start = rust
            .faces()
            .unwrap()
            .into_iter()
            .find(|face| face.plane.is_some() && (face.center_mm[0] - frame.origin[0]).abs() < 1e-9)
            .unwrap();
        exact_cap(&rust, start.index, frame);
        exchange(&rust, &native, policy);
    }
}

#[test]
fn blind_mortise_keeps_its_floor_reference_when_a_round_hole_is_added() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let cut = EntityId::new();
    let frame = PlaneFrame {
        origin: [5., 6., 13.],
        ..Default::default()
    };
    let slot = RoundedSlotTool {
        frame,
        centers_uv: [[0., 0.], [4., 0.]],
        radius_mm: 2.,
        displacement_mm: [0., 0., -5.],
        origin: Origin {
            source: cut,
            role: "mortise".into(),
        },
    };
    let rust = Solid::rust_box(
        owner.uuid_bits(),
        [20, 16, 12].map(|v| Rational::new(v, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap()
    .rust_drill_rounded_slots(std::slice::from_ref(&slot), policy)
    .unwrap();
    let before = rust.topology_snapshot(owner, 1).unwrap();
    let floor = before
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face
                && element
                    .origins
                    .iter()
                    .any(|origin| origin.role.starts_with("slot.floor/"))
        })
        .unwrap();
    let intent = GeometryIntent::capture(&before, floor.key).unwrap();
    let hole = CircularPocketTool {
        frame,
        center_uv: [9., 4.],
        radius_mm: 1.,
        displacement_mm: [0., 0., -4.],
        origin: Origin {
            source: cut,
            role: "hole".into(),
        },
    };
    let rust = rust
        .rust_drill_circular_pockets(std::slice::from_ref(&hole), policy)
        .unwrap();
    let display = rust.topology_with_display(owner, 2, Some(0.05)).unwrap();
    assert_eq!(display.edges, rust.display_edges(0.05).unwrap());
    assert_eq!(display.vertices, rust.display_vertices().unwrap());
    let exact_only = rust.topology_with_display(owner, 2, None).unwrap();
    assert_eq!(exact_only.topology, display.topology);
    assert!(exact_only.edges.is_empty() && exact_only.vertices.is_empty());
    let Resolution::Resolved { key, .. } = resolve(&intent, &display.topology, 2) else {
        panic!("mortise floor lost its identity after another cut");
    };
    exact_cap(
        &rust,
        key.ordinal,
        PlaneFrame {
            origin: [5., 6., 8.],
            ..frame
        },
    );
    let native = Solid::box_solid([20., 16., 12.], policy).unwrap();
    let cutters = [
        Solid::extrude_curved_profile_vector(&profile(frame), slot.displacement_mm, policy)
            .unwrap(),
        Solid::extrude_curved_profile_vector(
            &CurvedProfile {
                frame,
                outer: vec![CurvedEdge2d::Circle {
                    center: hole.center_uv,
                    radius_mm: hole.radius_mm,
                }],
                holes: vec![],
            },
            hole.displacement_mm,
            policy,
        )
        .unwrap(),
    ];
    let native = cutters.into_iter().fold(native, |body, tool| {
        body.boolean(&tool, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap()
    });
    near(
        rust.volume_mm3().unwrap(),
        20. * 16. * 12. - 4. * AREA - 3. * PI,
    );
    compare(&rust, &native);
    exchange(&rust, &native, policy);

    let overlap = CircularPocketTool {
        center_uv: [2., 0.],
        ..hole
    };
    assert!(
        rust.rust_drill_circular_pockets(&[overlap], policy)
            .is_err()
    );
}
