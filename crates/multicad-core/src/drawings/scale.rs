//! Displayed drawing scales and view fitting. Every view prints its own
//! paper:model scale next to it, and the title block shows one scale only when
//! every view on the page actually uses it. Fitting chooses a scale from the
//! view's evaluated paper extent and the free paper beside the title block.
use super::{DrawingEvaluation, DrawingView, Page, PageLayout, Scale, ViewEvaluation, ViewKind};
use crate::document::EntityId;

/// Height of a view caption on paper.
pub const CAPTION_MM: f64 = 2.5;
/// Monospaced advance per character, relative to text height. Courier is 0.6;
/// the extra margin covers the native monospace fallback.
pub const CHAR_ASPECT: f64 = 0.66;
/// Paper reserved below a fitted view for its caption.
const CAPTION_STRIP_MM: f64 = 8.;
/// Clearance between a fitted view and the border or title block.
const FIT_PAD_MM: f64 = 6.;
/// Title-block text when the page's views do not share one scale.
pub const MIXED_SCALE: &str = "Mixed, see views";

/// `n:d` exactly as stored; no rounding or reduction.
pub fn scale_text(scale: Scale) -> String {
    format!("{}:{}", scale.numerator, scale.denominator)
}
/// Paper millimeters per model millimeter.
pub fn scale_factor(scale: Scale) -> f64 {
    scale.numerator as f64 / scale.denominator as f64
}
/// The scale a title block may claim for the whole page: the page default when
/// it has no views, the views' common scale when they agree (by ratio), and
/// otherwise a notice that each view carries its own scale.
pub fn sheet_scale_label(page: &Page) -> String {
    let mut views = page.views.iter().map(|v| v.scale);
    let Some(first) = views.next() else {
        return scale_text(page.settings.scale);
    };
    let same = |a: Scale, b: Scale| {
        a.numerator as u64 * b.denominator as u64 == b.numerator as u64 * a.denominator as u64
    };
    if views.all(|s| same(s, first)) {
        scale_text(first)
    } else {
        MIXED_SCALE.into()
    }
}
/// A view's evaluated geometry on paper, `[x0, y0, x1, y1]` with y down.
pub fn view_paper_bounds(
    view: &DrawingView,
    evaluation: Option<&DrawingEvaluation>,
) -> Option<[f64; 4]> {
    let Some(ViewEvaluation::Ready { vectors, .. }) = evaluation?.views.get(&view.id) else {
        return None;
    };
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in vectors.iter().flat_map(|v| &v.preview_mm) {
        let [x, y] = view.paper_point(*p);
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    b[0].is_finite().then_some(b)
}
/// One printed view label: `at` is the left end of the baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewCaption {
    pub view: EntityId,
    pub at: [f64; 2],
    pub height_mm: f64,
    pub text: String,
}
pub fn caption_text(view: &DrawingView) -> String {
    format!("{} — Scale {}", view.name.trim(), scale_text(view.scale))
}
/// Union of the title-block cells, if the page has one.
pub fn title_block_rect(layout: &PageLayout) -> Option<[f64; 4]> {
    layout
        .title_cells
        .iter()
        .map(|c| c.rect_mm)
        .chain(layout.logo_mm)
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
}
/// A caption under every view, centered on its evaluated paper extent (or its
/// anchor while the preview is pending). A caption that would leave the
/// printable area or overlap the title block moves above the view and is kept
/// inside the border, so the scale is never silently hidden.
pub fn view_captions(
    page: &Page,
    layout: &PageLayout,
    evaluation: Option<&DrawingEvaluation>,
) -> Vec<ViewCaption> {
    let [px0, py0, px1, py1] = layout.printable_mm;
    let block = title_block_rect(layout);
    page.views
        .iter()
        .map(|view| {
            let text = caption_text(view);
            let width = text.chars().count() as f64 * CAPTION_MM * CHAR_ASPECT;
            let bounds = view_paper_bounds(view, evaluation).unwrap_or({
                let [x, y] = view.position_mm;
                [x, y, x, y]
            });
            let center = (bounds[0] + bounds[2]) * 0.5;
            let x = (center - width * 0.5).clamp(px0 + 1., (px1 - 1. - width).max(px0 + 1.));
            let fits = |baseline: f64| {
                let rect = [x, baseline - CAPTION_MM, x + width, baseline + 0.6];
                rect[1] >= py0
                    && rect[3] <= py1
                    && block.is_none_or(|b| {
                        !(rect[0] < b[2] && rect[2] > b[0] && rect[1] < b[3] && rect[3] > b[1])
                    })
            };
            let below = bounds[3] + 2. + CAPTION_MM;
            let above = bounds[1] - 2.;
            let baseline = if fits(below) {
                below
            } else if fits(above) {
                above
            } else {
                below.clamp(py0 + CAPTION_MM + 1., py1 - 1.)
            };
            ViewCaption {
                view: view.id,
                at: [x, baseline],
                height_mm: CAPTION_MM,
                text,
            }
        })
        .collect()
}

/// How Fit chooses a view scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FitMode {
    /// The largest standard engineering scale that fits, enlarging small
    /// parts up to 100:1 (ISO 5455 1-2-5 series plus 1:25 multiples).
    #[default]
    Standard,
    /// The tightest whole-number ratio that fits (n:1 or 1:n).
    Exact,
    /// Keep the current scale and only center the view.
    KeepScale,
}
impl FitMode {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Exact, Self::KeepScale];
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard scale",
            Self::Exact => "Exact ratio",
            Self::KeepScale => "Keep current scale",
        }
    }
}
/// Standard drawing scales, largest first: enlargements, full size, then
/// reductions down to 1:100000.
pub fn standard_scales() -> Vec<Scale> {
    let mut out: Vec<Scale> = [100, 50, 20, 10, 5, 2]
        .into_iter()
        .map(|n| Scale {
            numerator: n,
            denominator: 1,
        })
        .collect();
    out.push(Scale {
        numerator: 1,
        denominator: 1,
    });
    let mut decade = 1u32;
    while decade < 100_000 {
        let quarter = (decade >= 10).then_some(decade / 10 * 25);
        for d in [
            Some(2 * decade),
            quarter,
            Some(5 * decade),
            Some(10 * decade),
        ]
        .into_iter()
        .flatten()
        {
            out.push(Scale {
                numerator: 1,
                denominator: d,
            });
        }
        decade *= 10;
    }
    out
}
/// The scale for a paper `extent` (model mm) within `available` paper mm.
pub fn fit_scale(extent: [f64; 2], available: [f64; 2], mode: FitMode) -> Result<Scale, String> {
    if available.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err("Paper has no space for a view".into());
    }
    let ratio = extent
        .iter()
        .zip(available)
        .filter(|(e, _)| e.is_finite() && **e > 1e-9)
        .map(|(e, a)| a / e)
        .fold(f64::INFINITY, f64::min);
    if !ratio.is_finite() {
        return Err("The view has no extent to fit".into());
    }
    match mode {
        FitMode::KeepScale => Err("Keep current scale does not choose a scale".into()),
        FitMode::Exact if ratio >= 1. => Ok(Scale {
            numerator: (ratio + 1e-9).floor().min(1000.) as u32,
            denominator: 1,
        }),
        FitMode::Exact => {
            let d = (1. / ratio - 1e-9).ceil();
            if d > 100_000. {
                return Err("View needs a larger sheet or a manually configured scale".into());
            }
            Ok(Scale {
                numerator: 1,
                denominator: d as u32,
            })
        }
        FitMode::Standard => standard_scales()
            .into_iter()
            .find(|s| scale_factor(*s) <= ratio * (1. + 1e-9))
            .ok_or_else(|| "View needs a larger sheet or a manually configured scale".into()),
    }
}
/// Where a fitted view goes: the free printable regions above and to the left
/// of the title block, padded and with room for the caption.
pub fn fit_regions(layout: &PageLayout) -> Vec<[f64; 4]> {
    let [x0, y0, x1, y1] = layout.printable_mm;
    let regions = match title_block_rect(layout) {
        Some([bx0, by0, _, _]) => vec![[x0, y0, x1, by0], [x0, y0, bx0, y1]],
        None => vec![[x0, y0, x1, y1]],
    };
    regions
        .into_iter()
        .map(|[a, b, c, d]| {
            [
                a + FIT_PAD_MM,
                b + FIT_PAD_MM,
                c - FIT_PAD_MM,
                d - FIT_PAD_MM - CAPTION_STRIP_MM,
            ]
        })
        .filter(|r| r[2] > r[0] && r[3] > r[1])
        .collect()
}
/// A fitted view: its scale, paper anchor, and the model-space offset of the
/// extent center that must move to the view origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitPlacement {
    pub scale: Scale,
    pub position_mm: [f64; 2],
    pub center_mm: [f64; 2],
}
/// Fit a view whose model-space extent (in view u/v) is `min..max`. The region
/// allowing the larger scale wins; the view is centered in it.
pub fn fit_view(
    view: &DrawingView,
    layout: &PageLayout,
    mut min: [f64; 2],
    mut max: [f64; 2],
    mode: FitMode,
) -> Result<FitPlacement, String> {
    if let ViewKind::Detail {
        center_mm,
        radius_mm,
    } = view.kind
    {
        min = [center_mm[0] - radius_mm, center_mm[1] - radius_mm];
        max = [center_mm[0] + radius_mm, center_mm[1] + radius_mm];
    }
    if min.iter().chain(&max).any(|v| !v.is_finite()) {
        return Err("Resolve this view before fitting it".into());
    }
    let extent = [max[0] - min[0], max[1] - min[1]];
    let regions = fit_regions(layout);
    let mut best: Option<(Scale, [f64; 4])> = None;
    for region in &regions {
        let available = [region[2] - region[0], region[3] - region[1]];
        let scale = match mode {
            FitMode::KeepScale => {
                let s = view.scale;
                let f = scale_factor(s);
                if extent[0] * f > available[0] + 1e-9 || extent[1] * f > available[1] + 1e-9 {
                    continue;
                }
                s
            }
            _ => match fit_scale(extent, available, mode) {
                Ok(s) => s,
                Err(_) => continue,
            },
        };
        if best.is_none_or(|(b, _)| scale_factor(scale) > scale_factor(b)) {
            best = Some((scale, *region));
        }
    }
    let (scale, region) = match (best, mode) {
        (Some(best), _) => best,
        // Keeping the scale never fails: center it in the largest region.
        (None, FitMode::KeepScale) => (
            view.scale,
            regions
                .iter()
                .copied()
                .max_by(|a, b| {
                    ((a[2] - a[0]) * (a[3] - a[1])).total_cmp(&((b[2] - b[0]) * (b[3] - b[1])))
                })
                .ok_or("Paper has no space for a view")?,
        ),
        (None, _) => {
            return Err("View needs a larger sheet or a manually configured scale".into());
        }
    };
    Ok(FitPlacement {
        scale,
        position_mm: [(region[0] + region[2]) * 0.5, (region[1] + region[3]) * 0.5],
        center_mm: [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5],
    })
}
