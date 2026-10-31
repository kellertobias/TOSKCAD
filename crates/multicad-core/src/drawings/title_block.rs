//! The default project title block, lower right inside the printable border.
//!
//! ```text
//! +---------------------------------------+-----------+
//! | Project                               |   logo    |  identity row
//! +---------------------------------------+-----------+
//! | Title (large)                                     |  title row
//! +------------+------------+------------+------------+
//! | Drawn by   | Checked by | Revision   | Drawing no.|
//! +------------+------------+------------+------------+
//! | Scale      | Units      | Projection | Sheet      |
//! +------------+------------+------------+------------+
//! ```
//!
//! Without the logo the project cell spans the full width, so there is no
//! empty branding hole. Values wrap within their cells and rows grow; nothing
//! is ellipsized or clipped. This module is independent of view and
//! room-plan rendering.
use super::{Page, Projection, TextCell, source::invalid};
use crate::document::DocumentError;

/// Label text height in every cell.
pub const LABEL_MM: f64 = 1.8;
/// Ordinary value text height.
pub const VALUE_MM: f64 = 2.5;
/// Drawing title text height.
pub const TITLE_MM: f64 = 3.5;
/// Line pitch relative to text height.
pub const PITCH: f64 = 1.35;
/// Monospaced advance relative to text height (matches the page layout).
const ADVANCE: f64 = super::CHAR_MM / super::FONT_MM;
/// Outer frame and inner cell line widths.
pub const FRAME_MM: f64 = 0.5;
pub const CELL_MM: f64 = 0.25;
const MAX_WIDTH_MM: f64 = 180.;
const PAD_MM: f64 = 1.2;
const IDENTITY_MIN_MM: f64 = 15.;
/// The bundled application logo: an 8-bit RGB, non-interlaced PNG.
pub const LOGO_PNG: &[u8] =
    include_bytes!("../../../../assets/branding/toskcad-logo-titleblock.png");

/// The logo's pixel size, read from its PNG header.
pub fn logo_size() -> [u32; 2] {
    let w = u32::from_be_bytes(LOGO_PNG[16..20].try_into().unwrap_or_default());
    let h = u32::from_be_bytes(LOGO_PNG[20..24].try_into().unwrap_or_default());
    [w, h]
}
/// Concatenated IDAT payload: a zlib stream of PNG-filtered scanlines,
/// directly usable as a PDF FlateDecode image with PNG predictors.
pub fn logo_idat() -> Vec<u8> {
    let mut out = vec![];
    let mut i = 8;
    while i + 8 <= LOGO_PNG.len() {
        let n = u32::from_be_bytes(LOGO_PNG[i..i + 4].try_into().unwrap_or_default()) as usize;
        let kind = &LOGO_PNG[i + 4..i + 8];
        if kind == b"IDAT" {
            out.extend_from_slice(&LOGO_PNG[i + 8..i + 8 + n]);
        }
        i += 12 + n;
    }
    out
}

pub(super) struct TitleBlock {
    pub cells: Vec<TextCell>,
    pub logo: Option<[f64; 4]>,
    pub top_mm: f64,
}

struct Field {
    label: &'static str,
    value: String,
    height: f64,
}
fn field(label: &'static str, value: &str, height: f64) -> Field {
    Field {
        label,
        value: if value.trim().is_empty() {
            "—".into()
        } else {
            value.into()
        },
        height,
    }
}
fn chars(width: f64, height: f64) -> usize {
    ((width - 2. * PAD_MM) / (height * ADVANCE)).floor().max(0.) as usize
}
fn cell_height(lines: usize, height: f64) -> f64 {
    PAD_MM + LABEL_MM + 1.0 + lines.max(1) as f64 * height * PITCH + PAD_MM
}

/// Lay out the title block for page `index` of `total`, bottom-right inside
/// `printable` (`[x0, y0, x1, y1]`).
pub(super) fn title_block(
    page: &Page,
    index: usize,
    total: usize,
    printable: [f64; 4],
) -> Result<TitleBlock, DocumentError> {
    let [px0, py0, px1, py1] = printable;
    let width = (px1 - px0).min(MAX_WIDTH_MM);
    let column = width / 4.;
    if chars(column, VALUE_MM) < 8 {
        return Err(invalid(
            "The printable area is too narrow for a title block",
        ));
    }
    let projection = match page.settings.projection {
        Projection::FirstAngle => "First angle",
        Projection::ThirdAngle => "Third angle",
    };
    let logo = page.settings.app_logo;
    let project_width = if logo { 3. * column } else { width };
    // Rows of (fields, column span of each field).
    let rows: [Vec<(Field, f64)>; 4] = [
        vec![(
            field("Project", &page.info.project, VALUE_MM),
            project_width,
        )],
        vec![(field("Title", &page.info.title, TITLE_MM), width)],
        vec![
            (field("Drawn by", &page.info.author, VALUE_MM), column),
            (field("Checked by", &page.info.checker, VALUE_MM), column),
            (field("Revision", &page.info.revision, VALUE_MM), column),
            (field("Drawing no.", &page.number, VALUE_MM), column),
        ],
        vec![
            (
                field("Scale", &super::sheet_scale_label(page), VALUE_MM),
                column,
            ),
            (field("Units", "mm", VALUE_MM), column),
            (field("Projection", projection, VALUE_MM), column),
            (
                field("Sheet", &format!("{index} / {total}"), VALUE_MM),
                column,
            ),
        ],
    ];
    let wrapped: Vec<Vec<(Field, f64, Vec<String>)>> = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(f, span)| {
                    let lines = super::wrap_text(&f.value, chars(span, f.height));
                    (f, span, lines)
                })
                .collect()
        })
        .collect();
    let heights: Vec<f64> = wrapped
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let h = row
                .iter()
                .map(|(f, _, lines)| cell_height(lines.len(), f.height))
                .fold(0., f64::max);
            if i == 0 && logo {
                h.max(IDENTITY_MIN_MM)
            } else {
                h
            }
        })
        .collect();
    let total_height: f64 = heights.iter().sum();
    if total_height > py1 - py0 - 10. {
        return Err(invalid(
            "Title block text does not fit this paper. Shorten the fields or select larger paper; required text will not be clipped",
        ));
    }
    let x0 = px1 - width;
    let top = py1 - total_height;
    let mut cells = vec![];
    let mut y = top;
    let mut logo_rect = None;
    for ((row, height), r) in wrapped.into_iter().zip(&heights).zip(0..) {
        let mut x = x0;
        for (f, span, lines) in row {
            cells.push(TextCell {
                rect_mm: [x, y, x + span, y + height],
                label: f.label.into(),
                lines,
                height_mm: f.height,
            });
            x += span;
        }
        if r == 0 && logo {
            // Fit the logo with preserved proportions inside the last column.
            let [pw, ph] = logo_size().map(f64::from);
            let (bw, bh) = (column - 2. * PAD_MM, height - 2. * PAD_MM);
            let s = (bw / pw).min(bh / ph);
            let (lw, lh) = (pw * s, ph * s);
            let cx = x + column * 0.5;
            let cy = y + height * 0.5;
            logo_rect = Some([cx - lw * 0.5, cy - lh * 0.5, cx + lw * 0.5, cy + lh * 0.5]);
        }
        y += height;
    }
    // The title row's cell order puts the title first, then the project, so
    // consumers can find both without knowing the geometry.
    cells.swap(0, 1);
    Ok(TitleBlock {
        cells,
        logo: logo_rect,
        top_mm: top,
    })
}
