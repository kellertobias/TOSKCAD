use multicad_core::{
    document::EntityId,
    references::{Origin, TopologyKind},
};
use multicad_kernel::*;
use std::f64::consts::{PI, TAU};
fn p() -> GeometryPolicy {
    Default::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-6, "{a} != {b}");
}
fn circle(radius: f64) -> CurvedProfile {
    CurvedProfile {
        frame: Default::default(),
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: radius,
        }],
        holes: vec![],
    }
}
fn origins(profile: &CurvedProfile) -> Vec<Origin> {
    profile
        .outer
        .iter()
        .chain(profile.holes.iter().flatten())
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect()
}
fn cap(solid: &Solid, id: EntityId, role: &str) -> Face {
    let snapshot = solid.topology_snapshot(id, 0).unwrap();
    let origin = Origin {
        source: id,
        role: role.into(),
    };
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
        .unwrap();
    solid
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| f.index == element.key.ordinal)
        .unwrap()
}
#[test]
fn rational_circle_frustum_cap_and_side_areas_use_adaptive_exact_surface_integration() {
    let profile = circle(2.);
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_vector_tapered_named(
        &profile,
        [0., 0., 10.],
        0.1_f64.atan(),
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    let before = solid.topology_snapshot(id, 0).unwrap();
    let start = cap(&solid, id, "cap.start");
    let end = cap(&solid, id, "cap.end");
    near(start.area_mm2, 4. * PI);
    near(end.area_mm2, 9. * PI);
    for (i, x) in [0., 0., 0.].into_iter().enumerate() {
        near(start.center_mm[i], x);
    }
    near(end.center_mm[2], 10.);
    let side: f64 = solid
        .faces()
        .unwrap()
        .into_iter()
        .filter(|f| f.index != start.index && f.index != end.index)
        .map(|f| f.area_mm2)
        .sum();
    near(side, 5. * PI * 101_f64.sqrt());
    assert_eq!(solid.topology_snapshot(id, 0).unwrap(), before);
}
#[test]
fn annular_rational_caps_and_oblique_centroids_retain_tight_area_accuracy() {
    let mut profile = circle(5.);
    profile.holes = vec![vec![CurvedEdge2d::Circle {
        center: [0., 0.],
        radius_mm: 2.,
    }]];
    profile.frame.origin = [100., 200., 300.];
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_vector_tapered_named(
        &profile,
        [6., 0., 8.],
        0.1_f64.atan(),
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    let start = cap(&solid, id, "cap.start");
    let end = cap(&solid, id, "cap.end");
    near(start.area_mm2, 21. * PI);
    near(end.area_mm2, 35. * PI);
    for (i, x) in [100., 200., 300.].into_iter().enumerate() {
        near(start.center_mm[i], x);
    }
    for (i, x) in [106., 200., 308.].into_iter().enumerate() {
        near(end.center_mm[i], x);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rational-caps.step");
    solid.write_step(&path).unwrap();
    let reopened = Solid::read_step(&path, p()).unwrap();
    let mut planar: Vec<_> = reopened
        .faces()
        .unwrap()
        .into_iter()
        .filter(|f| f.plane.is_some())
        .map(|f| f.area_mm2)
        .collect();
    planar.sort_by(f64::total_cmp);
    assert_eq!(planar.len(), 2);
    near(planar[0], 21. * PI);
    near(planar[1], 35. * PI);
}
#[test]
fn sloped_planar_cap_area_matches_true_surface_not_projected_section() {
    let mut profile = circle(3.);
    profile.holes = vec![vec![CurvedEdge2d::Circle {
        center: [0., 0.],
        radius_mm: 1.,
    }]];
    let length = 1_f64.hypot(0.3);
    let target = PlaneFrame {
        origin: [0., 0., 10.],
        u: [1. / length, 0., 0.3 / length],
        v: [0., 1., 0.],
    };
    let id = EntityId::new();
    let solid = Solid::extrude_curved_profile_to_plane_named(
        &profile,
        [0., 0., 1.],
        target,
        id,
        &origins(&profile),
        p(),
    )
    .unwrap();
    near(cap(&solid, id, "cap.start").area_mm2, 8. * PI);
    let end = cap(&solid, id, "cap.end");
    near(end.area_mm2, 8. * PI * length);
    near(end.center_mm[0], 0.);
    near(end.center_mm[1], 0.);
    near(end.center_mm[2], 10.);
}
#[test]
fn direct_exact_rational_profile_queries_use_the_same_accurate_measurement() {
    let y = 3_f64.sqrt();
    let profile = CurvedProfile {
        frame: Default::default(),
        outer: vec![CurvedEdge2d::BSpline(BSpline2d {
            degree: 2,
            poles: vec![
                [2., 0.],
                [2., 2. * y],
                [-1., y],
                [-4., 0.],
                [-1., -y],
                [2., -2. * y],
                [2., 0.],
            ],
            knots: vec![0., TAU / 3., 2. * TAU / 3., TAU],
            multiplicities: vec![3, 2, 2, 3],
            weights: Some(vec![1., 0.5, 1., 0.5, 1., 0.5, 1.]),
        })],
        holes: vec![],
    };
    near(profile.area_mm2(p()).unwrap(), 4. * PI);
}
