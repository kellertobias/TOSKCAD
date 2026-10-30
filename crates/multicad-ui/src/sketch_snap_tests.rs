use super::*;
use multicad_core::constraint_sketch::{Curve, Point};

/// Screen pixels per sketch millimeter in these tests.
const SCALE: f32 = 4.;

fn to_screen(p: [f64; 2]) -> Option<[f32; 2]> {
    Some([p[0] as f32 * SCALE, -(p[1] as f32) * SCALE])
}
fn query(pointer_mm: [f64; 2]) -> Query<'static> {
    Query {
        pointer_mm,
        pointer_px: to_screen(pointer_mm).unwrap(),
        to_screen: &to_screen,
    }
}
/// Pointer offset by whole screen pixels from a sketch position.
fn query_px(at: [f64; 2], dx: f32, dy: f32) -> Query<'static> {
    let mut pointer = to_screen(at).unwrap();
    pointer[0] += dx;
    pointer[1] += dy;
    Query {
        pointer_mm: [(pointer[0] / SCALE) as f64, (-pointer[1] / SCALE) as f64],
        pointer_px: pointer,
        to_screen: &to_screen,
    }
}

#[derive(Default)]
struct Builder {
    sketch: Sketch,
}
impl Builder {
    fn point(&mut self, p: [f64; 2]) -> EntityId {
        let id = EntityId::new();
        self.sketch.points.push(Point { id, position_mm: p });
        id
    }
    fn curve(&mut self, kind: CurveKind) -> EntityId {
        let id = EntityId::new();
        self.sketch.curves.push(Curve {
            id,
            construction: false,
            kind,
        });
        id
    }
    fn line(&mut self, start: EntityId, end: EntityId) -> EntityId {
        self.curve(CurveKind::Line { start, end })
    }
    fn polygon(&mut self, vertices: &[[f64; 2]]) -> (Vec<EntityId>, Vec<EntityId>) {
        let points: Vec<_> = vertices.iter().map(|v| self.point(*v)).collect();
        let lines = (0..points.len())
            .map(|k| self.line(points[k], points[(k + 1) % points.len()]))
            .collect();
        (points, lines)
    }
    fn geometry(&self) -> SnapGeometry {
        SnapGeometry::from_sketch(&self.sketch)
    }
}

/// Hovers at `pointer` at each listed time and returns the last outcome.
fn hover(assist: &mut SnapAssist, g: &SnapGeometry, q: &Query, times: &[f64]) -> Outcome {
    let mut out = Outcome::default();
    for t in times {
        out = assist.update(g, q, *t);
    }
    out
}

#[test]
fn dwell_acquires_only_after_the_documented_duration() {
    let mut b = Builder::default();
    let a = b.point([10., 10.]);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    let q = query_px([10., 10.], 2., -1.);
    let out = hover(&mut assist, &g, &q, &[0., 0.2, DWELL_SECONDS - 0.01]);
    assert!(out.event.is_none());
    assert!(assist.references().is_empty());
    let remaining = out.repaint_after.unwrap();
    assert!((remaining - 0.01).abs() < 1e-9);
    let out = assist.update(&g, &q, DWELL_SECONDS);
    assert_eq!(out.event, Some(DwellEvent::Acquired(0)));
    let r = &assist.references()[0];
    assert_eq!(r.position, [10., 10.], "cross sits at the exact point");
    assert_eq!(r.source, TargetKey::Point(a));
    // Continuing to hover does not toggle it off again.
    assert!(assist.update(&g, &q, 3.).event.is_none());
    assert_eq!(assist.references().len(), 1);
}

#[test]
fn briefly_passing_over_or_drifting_does_not_acquire() {
    let mut b = Builder::default();
    b.point([0., 0.]);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    assist.update(&g, &query_px([0., 0.], 0., 0.), 0.);
    assist.update(&g, &query([40., 40.]), 0.3);
    let out = assist.update(&g, &query_px([0., 0.], 0., 0.), 0.6);
    assert!(out.event.is_none(), "timer restarts after leaving");
    // Drift beyond the stability tolerance restarts the timer as well.
    assist.update(&g, &query([40., 40.]), 1.9);
    assist.update(&g, &query_px([0., 0.], -3., 0.), 2.);
    assist.update(&g, &query_px([0., 0.], 3., 0.), 2.3);
    let out = assist.update(&g, &query_px([0., 0.], 3., 0.), 2.6);
    assert!(
        out.event.is_none(),
        "6 px drift restarted the dwell at 2.3 s"
    );
    assert!(assist.references().is_empty());
    let out = assist.update(&g, &query_px([0., 0.], 2., 0.), 2.3 + DWELL_SECONDS);
    assert_eq!(out.event, Some(DwellEvent::Acquired(0)));
}

fn acquire(assist: &mut SnapAssist, g: &SnapGeometry, at: [f64; 2], start: f64) {
    let q = query_px(at, 1., 1.);
    let out = hover(assist, g, &q, &[start, start + DWELL_SECONDS]);
    assert!(
        matches!(out.event, Some(DwellEvent::Acquired(_))),
        "{at:?} was not acquired: {out:?}"
    );
}

#[test]
fn two_references_expose_their_exact_midpoint() {
    let mut b = Builder::default();
    b.point([0., 0.]);
    b.point([20., 10.]);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    acquire(&mut assist, &g, [0., 0.], 0.);
    acquire(&mut assist, &g, [20., 10.], 1.);
    assert_eq!(assist.references().len(), 2, "first reference retained");
    let out = assist.update(&g, &query_px([10., 5.], 3., -2.), 3.);
    let target = out.target.unwrap();
    assert_eq!(target.kind, TargetKind::ReferenceMidpoint);
    assert_eq!(target.position, [10., 5.]);
    assert_eq!(
        placement(Some(&target), [0.; 2], Some(10.)),
        ([10., 5.], None)
    );
}

#[test]
fn guides_snap_in_all_four_directions() {
    let mut b = Builder::default();
    b.point([5., 5.]);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    acquire(&mut assist, &g, [5., 5.], 0.);
    for (pointer, axis, expected) in [
        ([35., 5.6], Axis::Horizontal, [35., 5.]),
        ([-25., 4.4], Axis::Horizontal, [-25., 5.]),
        ([5.7, 40.], Axis::Vertical, [5., 40.]),
        ([4.5, -30.], Axis::Vertical, [5., -30.]),
    ] {
        let target = assist.update(&g, &query(pointer), 2.).target.unwrap();
        assert_eq!(target.kind, TargetKind::Guide(axis), "{pointer:?}");
        assert!((target.position[0] - expected[0]).abs() < 1e-6);
        assert!((target.position[1] - expected[1]).abs() < 1e-6);
        assert_eq!(
            target.position[if axis == Axis::Horizontal { 1 } else { 0 }],
            5.
        );
        assert_eq!(
            target.guides,
            vec![GuideLine {
                from: [5., 5.],
                axis
            }]
        );
    }
    // Beyond the guide tolerance (6 px = 1.5 mm here) nothing snaps.
    assert!(assist.update(&g, &query([35., 7.]), 2.).target.is_none());
}

#[test]
fn guides_from_two_references_intersect_and_meet_lines() {
    let mut b = Builder::default();
    b.point([0., 0.]);
    b.point([30., 20.]);
    let s = b.point([50., -10.]);
    let e = b.point([50., 40.]);
    b.line(s, e);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    acquire(&mut assist, &g, [0., 0.], 0.);
    acquire(&mut assist, &g, [30., 20.], 1.);
    let target = assist.update(&g, &query([30.5, 0.4]), 2.).target.unwrap();
    assert_eq!(target.kind, TargetKind::GuideIntersection);
    assert_eq!(target.position, [30., 0.]);
    assert_eq!(target.guides.len(), 2);
    let target = assist.update(&g, &query([49.6, 20.3]), 2.).target.unwrap();
    assert_eq!(target.kind, TargetKind::GuideOnLine);
    assert_eq!(target.position, [50., 20.]);
}

#[test]
fn line_midpoints_points_on_lines_and_competing_targets() {
    let mut b = Builder::default();
    let a = b.point([0., 0.]);
    let c = b.point([40., 0.]);
    let line = b.line(a, c);
    let g = b.geometry();
    let target = resolve(&g, &[], &query_px([20., 0.], 2., 1.), None).unwrap();
    assert_eq!(target.key, TargetKey::LineMidpoint(line));
    assert_eq!(target.position, [20., 0.]);
    let target = resolve(&g, &[], &query([31., 0.8]), None).unwrap();
    assert_eq!(target.kind, TargetKind::OnCurve);
    assert!((target.position[0] - 31.).abs() < 1e-6 && target.position[1] == 0.);
    // A vertex beats the nearer point on its own line and midpoints.
    let target = resolve(&g, &[], &query_px([40., 0.], -6., 0.), None).unwrap();
    assert_eq!(target.key, TargetKey::Point(c));
    assert_eq!(target.anchor, Some(c));
    // Outside every tolerance there is no geometry target; the grid applies.
    assert!(resolve(&g, &[], &query([25., 10.]), None).is_none());
    assert_eq!(placement(None, [24.6, 10.2], Some(10.)), ([20., 10.], None));
    assert_eq!(placement(None, [24.6, 10.2], None), ([24.6, 10.2], None));
}

#[test]
fn a_nearby_grid_point_never_displaces_a_geometry_target() {
    let mut b = Builder::default();
    let a = b.point([0., 1.3]);
    let c = b.point([40., 1.3]);
    b.line(a, c);
    let g = b.geometry();
    // The pointer sits on the grid point (10, 0) but within 6 px of the line.
    let q = query([10., 0.1]);
    let target = resolve(&g, &[], &q, None).unwrap();
    for grid in [Some(10.), None] {
        let (p, anchor) = placement(Some(&target), q.pointer_mm, grid);
        assert!((p[0] - 10.).abs() < 1e-6);
        assert_eq!(p[1], 1.3);
        assert_eq!(anchor, None);
    }
}

#[test]
fn hysteresis_keeps_the_previous_target_until_the_release_distance() {
    let mut b = Builder::default();
    let a = b.point([0., 0.]);
    let g = b.geometry();
    let key = TargetKey::Point(a);
    let q = query_px([0., 0.], 10., 0.);
    assert!(resolve(&g, &[], &q, None).is_none());
    assert_eq!(resolve(&g, &[], &q, Some(&key)).unwrap().key, key);
    let q = query_px([0., 0.], POINT_SNAP_PX * RELEASE_FACTOR + 0.5, 0.);
    assert!(resolve(&g, &[], &q, Some(&key)).is_none());
}

#[test]
fn centers_are_defined_only_for_circles_arcs_rectangles_and_regular_polygons() {
    let mut b = Builder::default();
    b.polygon(&[[0., 0.], [40., 0.], [40., 20.], [0., 20.]]);
    let hexagon: Vec<[f64; 2]> = (0..6)
        .map(|k| {
            let a = k as f64 * TAU / 6.;
            [100. + 10. * a.cos(), 10. * a.sin()]
        })
        .collect();
    b.polygon(&hexagon);
    // Irregular quadrilateral and scalene triangle: no center target.
    b.polygon(&[[200., 0.], [240., 0.], [230., 20.], [205., 25.]]);
    b.polygon(&[[300., 0.], [340., 0.], [310., 30.]]);
    let center = b.point([0., 100.]);
    b.curve(CurveKind::Circle {
        center,
        radius_mm: 5.,
    });
    let g = b.geometry();
    let mut centers: Vec<_> = g.shape_centers().collect();
    centers.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    assert_eq!(centers.len(), 2, "{centers:?}");
    assert_eq!(centers[0], ([20., 10.], ShapeKind::Rectangle));
    assert!((centers[1].0[0] - 100.).abs() < 1e-9 && centers[1].0[1].abs() < 1e-9);
    assert_eq!(centers[1].1, ShapeKind::RegularPolygon(6));
    let target = resolve(&g, &[], &query_px([20., 10.], 2., 2.), None).unwrap();
    assert_eq!(target.kind, TargetKind::ShapeCenter(ShapeKind::Rectangle));
    assert_eq!(target.position, [20., 10.]);
    let target = resolve(&g, &[], &query_px([0., 100.], 1., 0.), None).unwrap();
    assert_eq!(target.kind, TargetKind::Center);
    assert_eq!(target.anchor, Some(center));
    assert!(resolve(&g, &[], &query([220., 12.]), None).is_none());
}

#[test]
fn rectangle_with_an_attached_line_keeps_its_center() {
    let mut b = Builder::default();
    let (points, _) = b.polygon(&[[0., 0.], [10., 0.], [10., 10.], [0., 10.]]);
    let tail = b.point([30., 30.]);
    b.line(points[2], tail);
    let centers: Vec<_> = b.geometry().shape_centers().collect();
    assert_eq!(centers, vec![([5., 5.], ShapeKind::Rectangle)]);
}

#[test]
fn arcs_snap_only_within_their_sweep() {
    let mut b = Builder::default();
    let center = b.point([0., 0.]);
    let start = b.point([10., 0.]);
    let end = b.point([0., 10.]);
    b.curve(CurveKind::Arc {
        center,
        start,
        end,
        clockwise: false,
    });
    let g = b.geometry();
    let s = std::f64::consts::FRAC_1_SQRT_2 * 10.;
    let target = resolve(&g, &[], &query([s + 0.5, s + 0.5]), None).unwrap();
    assert_eq!(target.kind, TargetKind::OnCurve);
    assert!(((target.position[0]).hypot(target.position[1]) - 10.).abs() < 1e-9);
    assert!(resolve(&g, &[], &query([-s - 0.5, -s - 0.5]), None).is_none());
}

#[test]
fn line_midpoints_and_centers_can_be_acquired_and_aligned() {
    let mut b = Builder::default();
    b.polygon(&[[0., 0.], [40., 0.], [40., 20.], [0., 20.]]);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    acquire(&mut assist, &g, [20., 10.], 0.);
    assert_eq!(
        assist.references()[0].kind,
        TargetKind::ShapeCenter(ShapeKind::Rectangle)
    );
    acquire(&mut assist, &g, [40., 10.], 1.);
    assert_eq!(assist.references()[1].kind, TargetKind::LineMidpoint);
    let target = assist.update(&g, &query([80., 10.6]), 2.).target.unwrap();
    assert_eq!(target.kind, TargetKind::Guide(Axis::Horizontal));
    assert_eq!(target.position[1], 10.);
    let target = assist.update(&g, &query([20.6, 60.]), 2.).target.unwrap();
    assert_eq!(target.kind, TargetKind::Guide(Axis::Vertical));
    assert_eq!(target.position[0], 20.);
}

#[test]
fn on_curve_references_persist_and_redwelling_releases() {
    let mut b = Builder::default();
    let a = b.point([0., 0.]);
    let c = b.point([40., 0.]);
    b.line(a, c);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    let q = query([13., 0.5]);
    let out = hover(&mut assist, &g, &q, &[0., DWELL_SECONDS, 2., 4.]);
    assert!(out.event.is_none(), "acquired point must not flip off");
    assert_eq!(assist.references().len(), 1);
    assert_eq!(assist.references()[0].position, [13., 0.]);
    assert_eq!(out.target.unwrap().kind, TargetKind::Reference);
    // Leave and dwell again on the reference: it is released.
    assist.update(&g, &query([20., 30.]), 5.);
    let out = hover(&mut assist, &g, &q, &[6., 6. + DWELL_SECONDS]);
    assert_eq!(out.event, Some(DwellEvent::Released(0)));
    assert!(assist.references().is_empty());
}

#[test]
fn a_third_reference_drops_the_oldest() {
    let mut b = Builder::default();
    for x in [0., 30., 60.] {
        b.point([x, x / 3.]);
    }
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    for (k, x) in [0., 30., 60.].into_iter().enumerate() {
        acquire(&mut assist, &g, [x, x / 3.], k as f64);
    }
    let positions: Vec<_> = assist.references().iter().map(|r| r.position).collect();
    assert_eq!(positions, vec![[30., 10.], [60., 20.]]);
}

#[test]
fn references_follow_their_source_and_vanish_with_it() {
    let mut b = Builder::default();
    let a = b.point([0., 0.]);
    let mut assist = SnapAssist::default();
    acquire(&mut assist, &b.geometry(), [0., 0.], 0.);
    b.sketch.points[0].position_mm = [5., 5.];
    assist.revalidate(&b.geometry());
    assert_eq!(assist.references()[0].position, [5., 5.]);
    b.sketch.points.retain(|p| p.id != a);
    assist.revalidate(&b.geometry());
    assert!(assist.references().is_empty());
}

#[test]
fn clearing_and_switching_sketches_remove_transient_aids() {
    let mut b = Builder::default();
    b.point([0., 0.]);
    let g = b.geometry();
    let mut assist = SnapAssist::default();
    let sketch = EntityId::new();
    assist.bind(sketch);
    acquire(&mut assist, &g, [0., 0.], 0.);
    assist.bind(sketch);
    assert_eq!(assist.references().len(), 1);
    assist.bind(EntityId::new());
    assert!(assist.references().is_empty());
    acquire(&mut assist, &g, [0., 0.], 1.);
    assist.clear();
    assert!(assist.references().is_empty() && assist.shown().is_none());
}

#[test]
fn tolerances_are_screen_pixels_and_follow_zoom() {
    let mut b = Builder::default();
    b.point([0., 0.]);
    let g = b.geometry();
    // 1 mm away: 4 px at the test scale snaps; zoomed out 10x it is 0.4 px;
    // zoomed in so 1 mm is 12 px it no longer snaps.
    for (scale, snaps) in [(4f32, true), (0.4, true), (12., false)] {
        let screen = move |p: [f64; 2]| Some([p[0] as f32 * scale, -(p[1] as f32) * scale]);
        let q = Query {
            pointer_mm: [1., 0.],
            pointer_px: [scale, 0.],
            to_screen: &screen,
        };
        assert_eq!(resolve(&g, &[], &q, None).is_some(), snaps, "scale {scale}");
    }
}
