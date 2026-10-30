use super::*;

fn source() -> super::super::source::Source {
    super::super::source::Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    }
}
fn text(s: &str) -> SketchText {
    SketchText {
        id: EntityId::new(),
        style: TextStyle::new(s, 10., [0., 0.]),
    }
}
fn hack(s: &str) -> SketchText {
    let mut t = text(s);
    t.style.font = TextFont::HackRegular;
    t
}
fn bounds(contours: &[TextContour]) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in contours.iter().flat_map(|c| c.poles.iter()) {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    b
}

#[test]
fn operations_add_edit_remove_and_refuse_without_change() {
    let source = source();
    let add = TextOperation::Add {
        style: TextStyle::new("ABO", 5., [1., 2.]),
    };
    let (added, id) = add.apply(&source).unwrap();
    assert_eq!(added.sketch.texts.len(), 1);
    assert_eq!(added.minimum_schema_version(), 2);
    let json = serde_json::to_value(&added).unwrap();
    let back: super::super::source::Source = serde_json::from_value(json).unwrap();
    assert_eq!(back, added, "text round-trips exactly through source JSON");
    let mut style = TextStyle::new("ABD", 6., [1., 2.]);
    let (edited, same) = TextOperation::Edit {
        text: id,
        style: style.clone(),
    }
    .apply(&added)
    .unwrap();
    assert_eq!(same, id);
    assert_eq!(edited.sketch.texts[0].style.text, "ABD");
    style.text.clear();
    assert!(
        TextOperation::Edit { text: id, style }
            .apply(&edited)
            .unwrap_err()
            .to_string()
            .contains("empty")
    );
    let (removed, _) = TextOperation::Remove { text: id }.apply(&edited).unwrap();
    assert!(removed.sketch.texts.is_empty());
    assert!(TextOperation::Remove { text: id }.apply(&removed).is_err());
    // Serialized sketches without text keep their previous JSON shape.
    assert!(
        !serde_json::to_string(&removed.sketch)
            .unwrap()
            .contains("texts")
    );
}

#[test]
fn both_fonts_map_abo_to_exact_closed_degree_two_contours() {
    for font in TextFont::ALL {
        let mut t = text("ABO");
        t.style.font = font;
        let contours = t.contours().unwrap();
        // A: outline + counter, B: outline + two counters, O: outline + counter.
        assert_eq!(contours.len(), 7, "{font:?}");
        for c in &contours {
            assert_eq!(c.basis.degree, 2);
            assert_eq!(c.poles.first(), c.poles.last(), "closed contour");
            c.basis.validate(c.poles.len()).unwrap();
        }
        assert_eq!(contours, t.contours().unwrap(), "deterministic");
        let ids: std::collections::BTreeSet<_> = contours.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 7);
        // The size is the cap height: H spans exactly 0..size.
        let mut h = text("H");
        h.style.font = font;
        let [_, bottom, _, top] = bounds(&h.contours().unwrap());
        assert!(
            (top - 10.).abs() < 1e-9 && bottom.abs() < 1e-9,
            "{font:?} {top} {bottom}"
        );
    }
}

#[test]
fn cantarell_is_proportional_and_hack_is_monospaced() {
    let width = |t: SketchText| {
        let [bl, br, _, _] = t.style.block_corners().unwrap();
        br[0] - bl[0]
    };
    assert!(width(text("iiii")) < width(text("MMMM")) * 0.6);
    assert_eq!(width(hack("iiii")), width(hack("MMMM")));
}

/// CAD-142 placement, kept verbatim to prove migrated files are unchanged.
fn legacy_contour_poles(t: &SketchText) -> Vec<Vec<[f64; 2]>> {
    let style = &t.style;
    let font = Font::load(style.font).unwrap();
    let scale = style.height_mm / font.cap_height;
    let total: f64 = style
        .text
        .chars()
        .map(|c| font.advance(c).unwrap())
        .collect::<Vec<_>>()
        .into_iter()
        .sum();
    let start = match style.alignment {
        TextAlignment::Left => 0.,
        TextAlignment::Center => -total / 2.,
        TextAlignment::Right => -total,
    };
    let (sin, cos) = exact_sin_cos(style.rotation_deg);
    let place = |x: f64, y: f64| {
        let (x, y) = (x * scale, y * scale);
        [
            style.position_mm[0] + x * cos - y * sin,
            style.position_mm[1] + x * sin + y * cos,
        ]
    };
    let mut pen_x = start;
    let mut out = vec![];
    for c in style.text.chars() {
        for segments in font.glyph(c).unwrap() {
            let mut poles = vec![place(pen_x + segments[0].start[0], segments[0].start[1])];
            for s in &segments {
                poles.push(place(pen_x + s.control[0], s.control[1]));
                poles.push(place(pen_x + s.end[0], s.end[1]));
            }
            out.push(poles);
        }
        pen_x += font.advance(c).unwrap();
    }
    out
}

#[test]
fn cad142_storage_migrates_to_the_bottom_anchor_with_identical_outlines() {
    for (alignment, anchor) in [
        ("left", TextAnchor::BottomLeft),
        ("center", TextAnchor::BottomCenter),
        ("right", TextAnchor::BottomRight),
    ] {
        for rotation in [0., 90., 33.] {
            let id = EntityId::new();
            let json = serde_json::json!({
                "id": id,
                "style": { "text": "ABO", "font": "hack_regular", "height_mm": 7.5,
                           "origin_mm": [3.25, -1.5], "rotation_deg": rotation,
                           "alignment": alignment }
            });
            let t: SketchText = serde_json::from_value(json).unwrap();
            assert_eq!(t.style.anchor, anchor);
            assert_eq!(t.style.position_mm, [3.25, -1.5]);
            assert_eq!(t.style.font, TextFont::HackRegular);
            let poles: Vec<_> = t.contours().unwrap().into_iter().map(|c| c.poles).collect();
            assert_eq!(poles, legacy_contour_poles(&t), "{alignment} {rotation}");
            // Written back in the anchor form, which reads back exactly.
            let written = serde_json::to_value(&t).unwrap();
            assert!(written["style"].get("origin_mm").is_none());
            assert_eq!(written["style"]["anchor"], serde_json::json!(anchor));
            let back: SketchText = serde_json::from_value(written).unwrap();
            assert_eq!(back, t);
        }
    }
    // Mixed or missing placement fields are refused instead of guessed.
    for style in [
        serde_json::json!({"text":"A","font":"hack_regular","height_mm":1.0,"origin_mm":[0.0,0.0],
            "anchor":"center","rotation_deg":0.0,"alignment":"left"}),
        serde_json::json!({"text":"A","font":"hack_regular","height_mm":1.0,
            "position_mm":[0.0,0.0],"rotation_deg":0.0,"alignment":"left"}),
        serde_json::json!({"text":"A","font":"hack_regular","height_mm":1.0,
            "rotation_deg":0.0,"alignment":"left"}),
    ] {
        assert!(serde_json::from_value::<TextStyle>(style).is_err());
    }
}

#[test]
fn every_anchor_sits_exactly_on_the_position_and_moves_the_block_around_it() {
    for rotation in [0., 90., -180., 270.] {
        let mut boxes = vec![];
        for anchor in TextAnchor::ALL {
            let mut t = hack("Hi\nHH");
            t.style.position_mm = [12.5, -4.25];
            t.style.rotation_deg = rotation;
            t.style.anchor = anchor;
            let [bl, br, tr, tl] = t.style.block_corners().unwrap();
            let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
            let expected = match anchor {
                TextAnchor::TopLeft => tl,
                TextAnchor::TopCenter => mid(tl, tr),
                TextAnchor::TopRight => tr,
                TextAnchor::LeftCenter => mid(bl, tl),
                TextAnchor::Center => mid(bl, tr),
                TextAnchor::RightCenter => mid(br, tr),
                TextAnchor::BottomLeft => bl,
                TextAnchor::BottomCenter => mid(bl, br),
                TextAnchor::BottomRight => br,
            };
            for (axis, position) in [12.5, -4.25].into_iter().enumerate() {
                assert!(
                    (expected[axis] - position).abs() < 1e-12,
                    "{anchor:?} {rotation}: {expected:?}"
                );
            }
            assert_eq!(t.anchor_mm(), [12.5, -4.25]);
            boxes.push([bl, br, tr, tl]);
        }
        // Same box size for every anchor; only its offset differs.
        let size = |b: &[[f64; 2]; 4]| {
            (
                (b[1][0] - b[0][0]).hypot(b[1][1] - b[0][1]),
                (b[3][0] - b[0][0]).hypot(b[3][1] - b[0][1]),
            )
        };
        for b in &boxes {
            let (w, h) = size(b);
            let (w0, h0) = size(&boxes[0]);
            assert!((w - w0).abs() < 1e-9 && (h - h0).abs() < 1e-9);
        }
    }
    // Exact, unrotated: top-left anchor puts the cap line of the first line
    // on the anchor; bottom-right puts the last baseline's right end there.
    let mut t = hack("HH\nH");
    t.style.position_mm = [10., 20.];
    t.style.anchor = TextAnchor::TopLeft;
    let [bl, br, tr, tl] = t.style.block_corners().unwrap();
    assert_eq!(tl, [10., 20.]);
    assert_eq!(tr[1], 20.);
    assert_eq!(bl[0], 10.);
    let first_h = &t.contours().unwrap()[0];
    let top = first_h.poles.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
    assert!((top - 20.).abs() < 1e-12, "cap line on the anchor: {top}");
    t.style.anchor = TextAnchor::BottomRight;
    let moved = t.style.block_corners().unwrap();
    assert_eq!(moved[1], [10., 20.]);
    let shift = [10. - br[0], 20. - br[1]];
    for (a, b) in [bl, br, tr, tl].iter().zip(moved) {
        assert!((a[0] + shift[0] - b[0]).abs() < 1e-12 && (a[1] + shift[1] - b[1]).abs() < 1e-12);
    }
}

#[test]
fn multiline_alignment_places_each_line_inside_the_block() {
    // Hack is monospaced, so line 2 ("H") is one advance shorter than "HH".
    let base = hack("HH\nH");
    let poles = |alignment| {
        let mut t = base.clone();
        t.style.alignment = alignment;
        t.contours().unwrap()
    };
    let left = poles(TextAlignment::Left);
    let center = poles(TextAlignment::Center);
    let right = poles(TextAlignment::Right);
    assert_eq!(left.len(), 3);
    let x0 = |c: &TextContour| c.poles.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
    let advance = x0(&left[1]) - x0(&left[0]);
    assert!(advance > 0.);
    // First line is the widest line and does not move.
    for set in [&center, &right] {
        assert_eq!(set[0].poles, left[0].poles);
        assert_eq!(set[1].poles, left[1].poles);
    }
    assert!((x0(&left[2]) - x0(&left[0])).abs() < 1e-12);
    assert!((x0(&center[2]) - x0(&left[0]) - advance / 2.).abs() < 1e-12);
    assert!((x0(&right[2]) - x0(&left[1])).abs() < 1e-12);
    // The bottom-left anchor is the last baseline; the first line sits one
    // line pitch higher. Identities do not depend on alignment.
    let y0 = |c: &TextContour| c.poles.iter().map(|p| p[1]).fold(f64::MAX, f64::min);
    assert!(y0(&left[2]).abs() < 1e-12);
    assert!(y0(&left[0]) > 10.);
    assert_eq!(
        left.iter().map(|c| c.id).collect::<Vec<_>>(),
        right.iter().map(|c| c.id).collect::<Vec<_>>()
    );
}

#[test]
fn placement_changes_coordinates_but_not_identities() {
    let mut t = text("AB");
    let before = t.contours().unwrap();
    t.style.position_mm = [5., -3.];
    t.style.height_mm = 4.;
    t.style.rotation_deg = 90.;
    t.style.anchor = TextAnchor::Center;
    t.style.alignment = TextAlignment::Right;
    let after = t.contours().unwrap();
    assert_eq!(
        before.iter().map(|c| c.id).collect::<Vec<_>>(),
        after.iter().map(|c| c.id).collect::<Vec<_>>()
    );
    t.style.rotation_deg = 0.;
    t.style.anchor = TextAnchor::BottomRight;
    let [_, _, max_x, _] = bounds(&t.contours().unwrap());
    assert!(max_x <= 5. + 1e-9);
}

#[test]
fn clicking_inside_a_letter_finds_its_text() {
    let mut a = text("H");
    a.style.anchor = TextAnchor::Center;
    let mut b = text("O");
    b.style.position_mm = [50., 0.];
    let [x0, y0, x1, y1] = bounds(&a.contours().unwrap());
    // H's left stem is solid near its left edge at mid height.
    let stem = [x0 + (x1 - x0) * 0.05, (y0 + y1) / 2.];
    assert!(a.contains(stem));
    assert!(!a.contains([x1 + 1., y1 + 1.]));
    assert!(!b.contains(stem));
    let texts = [a.clone(), b.clone()];
    assert_eq!(text_at(&texts, stem), Some(a.id));
    let [ox0, oy0, ox1, oy1] = bounds(&b.contours().unwrap());
    // The ring of O, and its counter, both select it.
    assert_eq!(
        text_at(&texts, [ox0 + (ox1 - ox0) * 0.02, (oy0 + oy1) / 2.]),
        Some(b.id)
    );
    assert_eq!(
        text_at(&texts, [(ox0 + ox1) / 2., (oy0 + oy1) / 2.]),
        Some(b.id)
    );
    assert_eq!(text_at(&texts, [-100., -100.]), None);
    let contour = a.contours().unwrap()[0].id;
    assert!(a.owns_contour(contour));
    assert!(!b.owns_contour(contour));
}

#[test]
fn invalid_text_is_refused_with_actionable_messages() {
    for (s, h, needle) in [
        ("", 5., "empty"),
        ("   ", 5., "empty"),
        ("A\r\nB", 5., "not supported"),
        ("\nAB", 5., "empty line"),
        ("AB\n", 5., "empty line"),
        ("A\tB", 5., "not supported"),
        ("Ж", 5., "not supported"),
        ("AB", 0., "size"),
        ("AB", -1., "size"),
        ("AB", f64::NAN, "size"),
    ] {
        let mut t = text(s);
        t.style.height_mm = h;
        let e = t.validate().unwrap_err().to_string();
        assert!(e.contains(needle), "{s:?}: {e}");
    }
    let many = vec!["I"; MAX_TEXT_LINES + 1].join("\n");
    assert!(
        text(&many)
            .validate()
            .unwrap_err()
            .to_string()
            .contains("lines")
    );
    assert!(
        text("A\n\nB").validate().is_ok(),
        "inner blank lines are fine"
    );
    let long = text(&"O".repeat(40));
    assert!(long.validate().is_ok());
    let e = long.contours().unwrap_err().to_string();
    assert!(e.contains("budget"), "{e}");
}

#[test]
fn every_supported_character_has_exact_closed_contours_in_both_fonts() {
    let all: String = (' '..='~').chain(SUPPORTED_LATIN1.chars()).collect();
    for font in TextFont::ALL {
        for c in all.chars() {
            let mut t = text(&c.to_string());
            t.style.font = font;
            if c == ' ' {
                assert!(t.validate().is_err());
                continue;
            }
            if font.excluded().contains(c) {
                let e = t.validate().unwrap_err().to_string();
                assert!(e.contains("Hack Regular"), "{e}");
                continue;
            }
            let contours = t
                .contours()
                .unwrap_or_else(|e| panic!("{font:?} {c:?}: {e}"));
            assert!(!contours.is_empty(), "{font:?} {c:?}");
        }
    }
}
