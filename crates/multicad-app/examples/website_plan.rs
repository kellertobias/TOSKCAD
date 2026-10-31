//! Export a checked native drawing page as a website SVG. The workbench
//! preview spaces its weld callouts in a transactional copy; the input file
//! stays unchanged. Usage: website_plan PROJECT.tcd PAGE OUTPUT.svg
use multicad_core::{
    drawings,
    project_file::{self, FileLimits},
};
use multicad_model::{Compiler, DrawingProviders};
use std::{
    error::Error,
    fs::{self, File},
};
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, number, output] = args.as_slice() else {
        return Err("Usage: website_plan PROJECT.tcd PAGE OUTPUT.svg".into());
    };
    let input_path = fs::canonicalize(input)?;
    if fs::canonicalize(output).ok().as_ref() == Some(&input_path) {
        return Err("The SVG output must differ from the project input".into());
    }
    let p = project_file::read(
        &mut File::open(input)?,
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    let mut p = p;
    let mut revision = 0;
    if *number == "WB-102" {
        use multicad_core::commands::{Actor, Channel, Session};
        let book = drawings::books(&p.document)
            .next()
            .ok_or("Drawing book missing")?;
        let mut page = drawings::read(&p.document, book)?
            .pages
            .into_iter()
            .find(|p| p.number == *number)
            .ok_or("Weld drawing page missing")?;
        for left in [true, false] {
            let mut entries: Vec<_> = page
                .annotations
                .iter()
                .enumerate()
                .filter_map(|(i, a)| match &a.kind {
                    drawings::AnnotationKind::Weld { points_mm, .. }
                        if (points_mm[0][0] < 110.) == left =>
                    {
                        Some((i, points_mm[0][1]))
                    }
                    _ => None,
                })
                .collect();
            entries.sort_by(|a, b| a.1.total_cmp(&b.1));
            for (row, (index, _)) in entries.iter().enumerate() {
                if let drawings::AnnotationKind::Weld {
                    points_mm, tail, ..
                } = &mut page.annotations[*index].kind
                {
                    *points_mm = vec![
                        points_mm[0],
                        [if left { 15. } else { 175. }, 28. + row as f64 * 19.],
                    ];
                    *tail = tail.split_whitespace().next().unwrap_or("").to_string();
                }
            }
        }
        for annotation in &mut page.annotations {
            if let drawings::AnnotationKind::Table {
                columns,
                rows,
                widths_mm,
                ..
            } = &mut annotation.kind
                && columns.last().is_some_and(|c| c == "Notes")
            {
                columns.pop();
                widths_mm.pop();
                for row in rows {
                    row.pop();
                }
            }
        }
        let mut session = Session::new(p.document)?;
        let prepared = drawings::prepare(
            &session,
            0,
            Actor::local(Channel::CommandLine),
            book,
            drawings::PageAction::Replace(page),
        )?;
        session.commit(prepared)?;
        revision = session.revision();
        p.document = session.document().clone();
    }
    let m = Compiler::default().compile(&p.document, revision)?;
    for id in drawings::books(&p.document) {
        let b = drawings::read(&p.document, id)?;
        for page in b.pages {
            if page.number == *number {
                let e =
                    m.drawing_page(&p.document, revision, &DrawingProviders::default(), page.id)?;
                let svg = drawings::svg::page_svg(&p.document, revision, id, page.id, &e)?;
                fs::write(output, svg)?;
                println!(
                    "{}: {} evaluated views; wrote {}",
                    page.number,
                    e.views.len(),
                    output
                );
                return Ok(());
            }
        }
    }
    Err("drawing page missing".into())
}
