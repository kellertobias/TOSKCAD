//! Real native profile acceptance. This adapter only translates exact derived
//! curves; it does not author production features or manufacture from meshes.
use multicad_core::{
    constraint_sketch::{Curve, CurveKind, Sketch, source::Source},
    document::EntityId,
    references::{Origin, TopologyKind},
    sketch_regions::{self, Boundary, ExactCurve, Region},
};
use multicad_kernel::{BooleanOperation, PlaneFrame, Profile, ProfileEdge, Solid};
use std::f64::consts::PI;

fn source() -> Source {
    Source {
        u: [1., 0., 0.],
        sketch: Sketch::default(),
    }
}
fn circle(s: &mut Source, p: [f64; 2], radius_mm: f64) {
    let center = s.add_point(p);
    s.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::Circle { center, radius_mm },
    });
}
fn arc(s: &mut Source, p: [f64; 2], a: [f64; 2], b: [f64; 2], clockwise: bool) {
    let center = s.add_point(p);
    let start = s.add_point(a);
    let end = s.add_point(b);
    s.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::Arc {
            center,
            start,
            end,
            clockwise,
        },
    });
}
fn line(s: &mut Source, a: [f64; 2], b: [f64; 2]) {
    let a = s.add_point(a);
    let b = s.add_point(b);
    s.add_line(a, b);
}
fn edges(boundary: &Boundary) -> Vec<ProfileEdge> {
    boundary
        .edges
        .iter()
        .map(|e| match e.curve {
            ExactCurve::Line { a, b } => ProfileEdge::Line { a, b },
            ExactCurve::Arc {
                center,
                a,
                b,
                clockwise,
            } => ProfileEdge::Arc {
                center,
                a,
                b,
                clockwise,
            },
            ExactCurve::Circle { center, radius_mm } => ProfileEdge::Circle { center, radius_mm },
        })
        .collect()
}
fn profile(region: &Region, frame: PlaneFrame) -> Profile {
    Profile {
        frame,
        outer: edges(&region.outer),
        holes: region.holes.iter().map(edges).collect(),
    }
}
fn origins(region: &Region, sketch: EntityId) -> Vec<Origin> {
    std::iter::once(&region.outer)
        .chain(&region.holes)
        .flat_map(|b| &b.edges)
        .map(|e| Origin {
            source: sketch,
            role: format!("curve/{}", e.authored),
        })
        .collect()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn one_authored_sketch_makes_independent_exact_profiles_with_holes_and_arc_volume() {
    let mut source = source();
    source.add_rectangle([0., 0.], [100., 60.]).unwrap();
    circle(&mut source, [20., 30.], 5.);
    arc(&mut source, [150., 0.], [160., 0.], [140., 0.], false);
    line(&mut source, [140., 0.], [160., 0.]);
    let saved_source = source.clone();
    let extracted = sketch_regions::extract(&source);
    assert!(
        extracted.diagnostics.is_empty(),
        "{:?}",
        extracted.diagnostics
    );
    assert_eq!(extracted.regions.len(), 3);
    let sketch = EntityId::new();
    let mut volumes = vec![];
    for region in &extracted.regions {
        let height = if region.holes.is_empty() { 7. } else { 18. };
        let feature = EntityId::new();
        let authored = origins(region, sketch);
        let solid = Solid::extrude_profile_named(
            &profile(region, PlaneFrame::default()),
            height,
            feature,
            &authored,
            Default::default(),
        )
        .unwrap();
        near(solid.volume_mm3().unwrap(), region.area_mm2 * height);
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        for origin in authored {
            assert!(
                topology
                    .elements
                    .iter()
                    .any(|element| element.key.kind == TopologyKind::Face
                        && element.origins.contains(&origin)),
                "Authored curve lost its exact side-face provenance: {origin:?}"
            );
        }
        volumes.push(solid.volume_mm3().unwrap());
    }
    volumes.sort_by(f64::total_cmp);
    near(volumes[0], 175. * PI);
    near(volumes[1], 350. * PI);
    near(volumes[2], (6000. - 25. * PI) * 18.);
    assert_eq!(source, saved_source);
}

#[test]
fn every_nested_area_is_selectable_and_fills_outer_volume_without_overlap() {
    let mut s = source();
    circle(&mut s, [0., 0.], 20.);
    circle(&mut s, [0., 0.], 10.);
    circle(&mut s, [0., 0.], 3.);
    let extraction = sketch_regions::extract(&s);
    assert!(extraction.diagnostics.is_empty());
    assert_eq!(extraction.regions.len(), 3);
    let frame = PlaneFrame {
        origin: [25., 50., 75.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let mut sum = 0.;
    let mut solids = vec![];
    for region in &extraction.regions {
        let solid =
            Solid::extrude_profile(&profile(region, frame), 4., Default::default()).unwrap();
        near(solid.volume_mm3().unwrap(), region.area_mm2 * 4.);
        let bounds = solid.bounds().unwrap();
        near(bounds.min[0], 25.);
        near(bounds.max[0], 29.);
        sum += solid.volume_mm3().unwrap();
        solids.push(solid);
    }
    near(sum, 400. * PI * 4.);
    for i in 0..solids.len() {
        for j in i + 1..solids.len() {
            if let Some(overlap) = solids[i]
                .boolean(&solids[j], BooleanOperation::Intersect, Default::default())
                .unwrap()
            {
                near(overlap.volume_mm3().unwrap(), 0.);
            }
        }
    }
}

#[test]
fn clockwise_major_arc_and_reversed_authored_lines_reach_occt_without_sampling() {
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [0., 10.], true);
    line(&mut s, [10., 0.], [0., 10.]);
    let extraction = sketch_regions::extract(&s);
    assert!(extraction.diagnostics.is_empty());
    let region = &extraction.regions[0];
    near(region.area_mm2, 75. * PI + 50.);
    assert!(region.outer.edges.iter().any(|e| e.reversed));
    let solid = Solid::extrude_profile(
        &profile(region, PlaneFrame::default()),
        3.,
        Default::default(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), (75. * PI + 50.) * 3.);
}

#[test]
fn a_valid_profile_can_be_used_exactly_while_unrelated_open_source_remains_editable() {
    let mut s = source();
    s.add_rectangle([0., 0.], [40., 20.]).unwrap();
    line(&mut s, [100., 0.], [120., 5.]);
    let before = s.clone();
    let extraction = sketch_regions::extract(&s);
    assert_eq!(extraction.regions.len(), 1);
    assert!(!extraction.diagnostics.is_empty());
    let solid = Solid::extrude_profile(
        &profile(&extraction.regions[0], PlaneFrame::default()),
        5.,
        Default::default(),
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 4000.);
    assert_eq!(s, before);
}
