use super::{Page, source::invalid};
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
    /// Text height of `lines`; labels use `LABEL_MM`.
    pub height_mm: f64,
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
        let block = super::title_block::title_block(page, index, total, result.printable_mm)?;
        block_top = block.top_mm;
        result.title_cells = block.cells;
        result.logo_mm = block.logo;
    }
    for note in &page.notes {
        let [x, y] = note.position_mm;
        let chars = ((w - m - x - 2.) / CHAR_MM).floor().max(0.) as usize;
        if chars < 2 {
            return Err(invalid("Text note has no printable width"));
        }
        let lines = wrap_text(&note.text, chars);
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
            height_mm: FONT_MM,
        });
    }
    Ok(result)
}
fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}
pub(super) fn wrap_text(value: &str, width: usize) -> Vec<String> {
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
