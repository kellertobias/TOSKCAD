//! Scale labels, the standard scale series and the fitting contract, without
//! native geometry. PDF geometry is checked in multicad-model.
use multicad_core::{document::EntityId, drawings::*, references::ReferenceFrame};

fn scale(numerator: u32, denominator: u32) -> Scale {
    Scale {
        numerator,
        denominator,
    }
}
fn view(name: &str, s: Scale) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model {
            entities: vec![EntityId::new()],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [100., 80.],
        scale: s,
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
    }
}

#[test]
fn title_block_claims_one_scale_only_when_every_view_uses_it() {
    let mut page = Page::new("A-1");
    page.settings.scale = scale(1, 1);
    assert_eq!(
        sheet_scale_label(&page),
        "1:1",
        "page default without views"
    );
    page.views.push(view("Plan", scale(1, 23)));
    assert_eq!(
        sheet_scale_label(&page),
        "1:23",
        "never the 1:1 page default"
    );
    page.views.push(view("Copy", scale(2, 46)));
    assert_eq!(sheet_scale_label(&page), "1:23", "equal ratios agree");
    page.views.push(view("Detail", scale(5, 1)));
    assert_eq!(sheet_scale_label(&page), MIXED_SCALE);
    let layout = layout_page(&page, 1, 1).unwrap();
    let title: Vec<_> = layout
        .title_cells
        .iter()
        .flat_map(|c| c.lines.clone())
        .collect::<Vec<_>>();
    assert!(title.iter().any(|l| l.contains(MIXED_SCALE)), "{title:?}");
    assert!(!title.iter().any(|l| l.contains("1:1")), "{title:?}");
    // Captions: one per view, each with its stored scale, none in the title
    // block and all on the printable paper.
    let captions = view_captions(&page, &layout, None);
    let texts: Vec<_> = captions.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Plan — Scale 1:23",
            "Copy — Scale 2:46",
            "Detail — Scale 5:1"
        ]
    );
    let block = title_block_rect(&layout).unwrap();
    let [px0, py0, px1, py1] = layout.printable_mm;
    for c in &captions {
        let w = c.text.chars().count() as f64 * c.height_mm * CHAR_ASPECT;
        let r = [c.at[0], c.at[1] - c.height_mm, c.at[0] + w, c.at[1]];
        assert!(
            r[0] >= px0 && r[1] >= py0 && r[2] <= px1 && r[3] <= py1,
            "{c:?}"
        );
        assert!(
            r[2] <= block[0] || r[0] >= block[2] || r[3] <= block[1] || r[1] >= block[3],
            "{c:?} overlaps the title block"
        );
    }
    // A view anchored just above the title block moves its caption above.
    page.views[0].position_mm = [block[0] + 20., block[1] - 1.];
    let moved = &view_captions(&page, &layout, None)[0];
    assert!(moved.at[1] < block[1] - 1., "{moved:?}");
}

#[test]
fn standard_series_prefers_readable_engineering_scales() {
    let series: Vec<_> = standard_scales().into_iter().map(scale_text).collect();
    for expected in [
        "100:1", "10:1", "5:1", "2:1", "1:1", "1:2", "1:5", "1:10", "1:20", "1:25", "1:50",
        "1:100", "1:250", "1:100000",
    ] {
        assert!(
            series.contains(&expected.to_owned()),
            "{expected}: {series:?}"
        );
    }
    assert!(!series.contains(&"1:23".to_owned()));
    let factors: Vec<_> = standard_scales().into_iter().map(scale_factor).collect();
    assert!(factors.windows(2).all(|w| w[0] > w[1]), "largest first");

    // Small parts are enlarged; large ones reduced to the next standard scale.
    let a = [265., 136.];
    let fit = |e: [f64; 2], m| scale_text(fit_scale(e, a, m).unwrap());
    assert_eq!(fit([16., 16.], FitMode::Standard), "5:1");
    assert_eq!(fit([16., 16.], FitMode::Exact), "8:1");
    assert_eq!(fit([4000., 3000.], FitMode::Standard), "1:25");
    assert_eq!(fit([4000., 3000.], FitMode::Exact), "1:23");
    assert_eq!(fit([200., 100.], FitMode::Standard), "1:1");
    assert_eq!(fit([720., 560.], FitMode::Standard), "1:5");
    assert!(fit_scale([16., 16.], a, FitMode::KeepScale).is_err());
    assert!(fit_scale([0., 0.], a, FitMode::Standard).is_err());
    assert!(fit_scale([1e9, 1e9], a, FitMode::Exact).is_err());
    assert!(fit_scale([10., 10.], [0., 10.], FitMode::Standard).is_err());
    // A line has one finite axis; it still fits.
    assert_eq!(fit([100., 0.], FitMode::Standard), "2:1");
}

#[test]
fn fitting_uses_free_paper_beside_the_title_block_and_keeps_scale_on_request() {
    let page = Page::new("A-1");
    let layout = layout_page(&page, 1, 1).unwrap();
    let block = title_block_rect(&layout).unwrap();
    let v = view("Part", scale(1, 1));
    for mode in FitMode::ALL {
        let placed = fit_view(&v, &layout, [-8., -8.], [8., 8.], mode).unwrap();
        let f = scale_factor(placed.scale);
        let half = 8. * f;
        let r = [
            placed.position_mm[0] - half,
            placed.position_mm[1] - half,
            placed.position_mm[0] + half,
            placed.position_mm[1] + half,
        ];
        let [px0, py0, px1, py1] = layout.printable_mm;
        assert!(
            r[0] > px0 && r[1] > py0 && r[2] < px1 && r[3] < py1,
            "{mode:?} {r:?}"
        );
        assert!(
            r[3] < block[1] || r[2] < block[0],
            "{mode:?} clips the title block: {r:?} {block:?}"
        );
        match mode {
            FitMode::KeepScale => assert_eq!(placed.scale, scale(1, 1)),
            _ => assert!(placed.scale.numerator > 1, "{mode:?} enlarges a small part"),
        }
        assert_eq!(placed.center_mm, [0., 0.]);
    }
    // A detail fits its circular region, not the whole projection.
    let mut detail = v.clone();
    detail.kind = ViewKind::Detail {
        center_mm: [50., 0.],
        radius_mm: 5.,
    };
    let placed = fit_view(&detail, &layout, [-500.; 2], [500.; 2], FitMode::Standard).unwrap();
    assert_eq!(placed.center_mm, [50., 0.]);
    assert!(placed.scale.numerator >= 10);
}
