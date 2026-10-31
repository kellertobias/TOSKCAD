use super::{Page, Projection, source::invalid};
use crate::document::DocumentError;

/// Deterministic paper-space typography contract. Renderers use monospaced text
/// at FONT_MM and these prewrapped lines; they must not ellipsize field values.
pub const FONT_MM: f64 = 2.2;
pub const CHAR_MM: f64 = 1.45;
pub const LINE_MM: f64 = 3.0;
#[derive(Clone, Debug, PartialEq)]
pub struct TextCell {
    pub rect_mm: [f64; 4],
    pub label: String,
    pub lines: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PageLayout {
    pub paper_mm: [f64; 2],
    pub printable_mm: [f64; 4],
    pub title_cells: Vec<TextCell>,
    pub notes: Vec<TextCell>,
    /// Paper rectangle of the application logo inside the title block.
    pub logo_mm: Option<[f64; 4]>,
}
pub fn layout_page(page: &Page, index: usize, total: usize) -> Result<PageLayout, DocumentError> {
    let [w, h] = page.settings.dimensions();
    let m = page.settings.margin_mm;
    if [w, h, m].iter().any(|value| !value.is_finite())
        || !(50.0..=2000.).contains(&w)
        || !(50.0..=2000.).contains(&h)
        || m < 0.
        || 2. * m + 20. > w.min(h)
    {
        return Err(invalid("Invalid printable paper dimensions"));
    }
    let mut result = PageLayout {
        paper_mm: [w, h],
        printable_mm: [m, m, w - m, h - m],
        title_cells: vec![],
        notes: vec![],
        logo_mm: None,
    };
    let mut block_top = h - m;
    if page.settings.title_block {
        let width = (w - 2. * m).min(180.);
        let half = width / 2.;
        let chars = ((half - 4.) / CHAR_MM).floor() as usize;
        if chars < 8 {
            return Err(invalid(
                "The printable area is too narrow for a title block",
            ));
        }
        let projection = match page.settings.projection {
            Projection::FirstAngle => "First angle",
            Projection::ThirdAngle => "Third angle",
        };
        let scale = super::sheet_scale_label(page);
        let fields = [
            ("Title", page.info.title.as_str()),
            ("Project", page.info.project.as_str()),
            ("Author", page.info.author.as_str()),
            ("Checker", page.info.checker.as_str()),
            ("Revision", page.info.revision.as_str()),
            ("Number", page.number.as_str()),
        ];
        let mut rows = vec![];
        for pair in fields.chunks(2) {
            let values = pair
                .iter()
                .map(|(label, value)| {
                    (
                        (*label).to_owned(),
                        wrap(
                            if value.trim().is_empty() {
                                "—"
                            } else {
                                value
                            },
                            chars,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let height = 6.
                + LINE_MM
                    * values
                        .iter()
                        .map(|(_, lines)| lines.len())
                        .max()
                        .unwrap_or(1) as f64;
            rows.push((height, values));
        }
        let footer = format!("Page {index}/{total}   Scale {scale}   {projection}");
        let lines = wrap(&footer, ((width - 4.) / CHAR_MM).floor() as usize);
        let footer_height = 4. + LINE_MM * lines.len() as f64;
        let height = rows.iter().map(|(height, _)| height).sum::<f64>() + footer_height;
        if height > h - 2. * m - 10. {
            return Err(invalid(
                "Title block text does not fit this paper. Shorten the fields or select larger paper; required text will not be clipped",
            ));
        }
        let x = w - m - width;
        let mut y = h - m - height;
        block_top = y;
        for (height, values) in rows {
            for (column, (label, lines)) in values.into_iter().enumerate() {
                let left = x + column as f64 * half;
                result.title_cells.push(TextCell {
                    rect_mm: [left, y, left + half, y + height],
                    label,
                    lines,
                });
            }
            y += height;
        }
        result.title_cells.push(TextCell {
            rect_mm: [x, y, x + width, y + footer_height],
            label: String::new(),
            lines,
        });
    }
    for note in &page.notes {
        let [x, y] = note.position_mm;
        let chars = ((w - m - x - 2.) / CHAR_MM).floor().max(0.) as usize;
        if chars < 2 {
            return Err(invalid("Text note has no printable width"));
        }
        let lines = wrap(&note.text, chars);
        let bottom = y + LINE_MM * lines.len() as f64 + 2.;
        let note_rect = [x, y, w - m, bottom];
        if bottom > h - m
            || result
                .title_cells
                .iter()
                .any(|cell| overlaps(note_rect, cell.rect_mm))
        {
            return Err(invalid(format!(
                "Text note extends beyond available paper or overlaps the title block (block starts at {block_top:.1} mm)"
            )));
        }
        result.notes.push(TextCell {
            rect_mm: note_rect,
            label: String::new(),
            lines,
        });
    }
    Ok(result)
}
fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}
fn wrap(value: &str, width: usize) -> Vec<String> {
    let mut lines = vec![];
    for line in value.split('\n') {
        if line.is_empty() {
            lines.push(String::new());
        } else {
            let mut current = String::new();
            let mut cells = 0;
            for character in line.replace('\t', "    ").chars() {
                // Reserve two monospaced cells for non-ASCII fallback glyphs,
                // including East Asian text and emoji. Combining sequences may
                // wrap earlier, but no required character is dropped.
                let used = if character.is_ascii() { 1 } else { 2 };
                if cells + used > width && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    cells = 0;
                }
                current.push(character);
                cells += used;
            }
            if !current.is_empty() {
                lines.push(current);
            }
        }
    }
    lines
}
