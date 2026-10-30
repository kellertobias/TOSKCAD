//! Sketch text outlines (CAD-142, CAD-149) through the exact native region
//! service: every supported character's glyph contours, in every bundled
//! font, are closed, non-intersecting and nested so that counters are holes.
//! This defines the supported set.
use multicad_core::{
    constraint_sketch::{
        SplineBasis,
        text::{SUPPORTED_LATIN1, SketchText, TextAlignment, TextAnchor, TextFont, TextStyle},
    },
    document::EntityId,
    sketch_regions::{
        ExactCurve, Problem, RegionQueries, extract_evaluated_with_queries_and_interrupt,
    },
};
use multicad_kernel::NativeRegionQueries;

fn text(s: &str) -> SketchText {
    SketchText {
        id: EntityId::new(),
        style: TextStyle::new(s, 10., [0., 0.]),
    }
}

#[test]
fn every_supported_character_extracts_clean_nested_regions() {
    let queries = NativeRegionQueries::default();
    let mut failures = vec![];
    for (font, c) in TextFont::ALL.into_iter().flat_map(|font| {
        ('!'..='~')
            .chain(SUPPORTED_LATIN1.chars())
            .map(move |c| (font, c))
    }) {
        let mut t = text(&c.to_string());
        t.style.font = font;
        if !font.supports(c) {
            assert!(t.validate().is_err(), "{font:?} refuses {c:?}");
            continue;
        }
        let contours = t.contours().unwrap().len();
        let extraction =
            extract_evaluated_with_queries_and_interrupt(&[], &[], &[t], &queries, None).unwrap();
        // Every closed contour bounds exactly one selectable region.
        if !extraction.diagnostics.is_empty() || extraction.regions.len() != contours {
            failures.push(format!(
                "{font:?} {c:?}: {} regions for {contours} contours, {:?}",
                extraction.regions.len(),
                extraction
                    .diagnostics
                    .iter()
                    .map(|d| (d.problem, d.message.clone()))
                    .collect::<Vec<_>>()
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Closed polygon as one C0 degree-2 spline with midpoint controls, the
/// representation text uses for straight glyph spans.
fn polygon(points: &[[f64; 2]]) -> ExactCurve {
    let n = points.len();
    let mut poles = vec![points[0]];
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        poles.push([(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]);
        poles.push(b);
    }
    let mut multiplicities = vec![2; n + 1];
    multiplicities[0] = 3;
    multiplicities[n] = 3;
    ExactCurve::BSpline {
        poles,
        basis: SplineBasis {
            degree: 2,
            knots: (0..=n).map(|k| k as f64).collect(),
            multiplicities,
            weights: None,
        },
    }
}

#[test]
fn c0_spline_self_intersection_ignores_joints_but_finds_real_crossings() {
    let q = NativeRegionQueries::default();
    // Parallel straight spans used to read as overlap in OCCT's single-curve
    // intersector; split C0 pieces are now checked pairwise.
    let h = [
        [0., 0.],
        [1., 0.],
        [1., 4.],
        [3., 4.],
        [3., 0.],
        [4., 0.],
        [4., 10.],
        [3., 10.],
        [3., 5.],
        [1., 5.],
        [1., 10.],
        [0., 10.],
    ];
    let l = [[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 5.], [0., 5.]];
    for shape in [&h[..], &l[..]] {
        let hits = q.self_intersections(&polygon(shape), 1e-7).unwrap();
        assert!(hits.points.is_empty() && !hits.overlap, "{hits:?}");
    }
    let bow_tie = q
        .self_intersections(&polygon(&[[0., 0.], [4., 4.], [4., 0.], [0., 4.]]), 1e-7)
        .unwrap();
    assert!(
        bow_tie
            .points
            .iter()
            .any(|p| (p[0] - 2.).abs() < 1e-6 && (p[1] - 2.).abs() < 1e-6),
        "{bow_tie:?}"
    );
    let retrace = q
        .self_intersections(&polygon(&[[0., 0.], [4., 0.], [4., 2.], [4., 1.]]), 1e-7)
        .unwrap();
    assert!(retrace.overlap || !retrace.points.is_empty(), "{retrace:?}");
}

#[test]
fn multiline_aligned_text_keeps_closed_nested_regions() {
    for font in TextFont::ALL {
        for alignment in [
            TextAlignment::Left,
            TextAlignment::Center,
            TextAlignment::Right,
        ] {
            let mut t = text("AB8\nOg\nÄ");
            t.style.font = font;
            t.style.alignment = alignment;
            t.style.anchor = TextAnchor::Center;
            t.style.rotation_deg = 15.;
            let contours = t.contours().unwrap().len();
            let extraction = extract_evaluated_with_queries_and_interrupt(
                &[],
                &[],
                &[t],
                &NativeRegionQueries::default(),
                None,
            )
            .unwrap();
            assert!(
                extraction.diagnostics.is_empty(),
                "{font:?} {alignment:?}: {:?}",
                extraction.diagnostics
            );
            assert_eq!(extraction.regions.len(), contours, "{font:?} {alignment:?}");
            // A, B, 8, O, g and Ä each enclose at least one counter.
            let letters = extraction
                .regions
                .iter()
                .filter(|r| !r.holes.is_empty())
                .count();
            assert!(letters >= 6, "{font:?} {alignment:?}: {letters}");
        }
    }
}

#[test]
fn abo_counters_are_holes_and_areas_balance() {
    for font in TextFont::ALL {
        abo_counters(font);
    }
}
fn abo_counters(font: TextFont) {
    let mut t = text("ABO");
    t.style.font = font;
    let extraction = extract_evaluated_with_queries_and_interrupt(
        &[],
        &[],
        &[t],
        &NativeRegionQueries::default(),
        None,
    )
    .unwrap();
    assert!(
        extraction.diagnostics.is_empty(),
        "{:?}",
        extraction.diagnostics
    );
    let mut holes: Vec<_> = extraction.regions.iter().map(|r| r.holes.len()).collect();
    holes.sort();
    assert_eq!(holes, vec![0, 0, 0, 0, 1, 1, 2]);
    for letter in extraction.regions.iter().filter(|r| !r.holes.is_empty()) {
        // Each hole is also offered as its own counter disk of equal area.
        for hole in &letter.holes {
            let disk = extraction
                .regions
                .iter()
                .find(|r| r.outer.intent == hole.intent)
                .expect("counter disk");
            assert!(disk.holes.is_empty());
            assert!((disk.area_mm2 - hole.signed_area_mm2.abs()).abs() < 1e-9);
        }
        assert!(letter.area_mm2 < letter.outer.signed_area_mm2.abs());
    }
    // Touching or overlapping texts are diagnosed, never silently merged.
    let mut a = text("O");
    a.style.font = font;
    let mut b = a.clone();
    b.id = EntityId::new();
    b.style.position_mm = [2., 0.];
    let crossing = extract_evaluated_with_queries_and_interrupt(
        &[],
        &[],
        &[a, b],
        &NativeRegionQueries::default(),
        None,
    )
    .unwrap();
    assert!(
        crossing
            .diagnostics
            .iter()
            .any(|d| d.problem == Problem::Intersection)
    );
}
