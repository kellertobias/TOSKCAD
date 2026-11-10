//! Export a checked native drawing page as a website SVG. The workbench
//! drawings use the canonical page layout; the input file stays unchanged. Usage: website_plan PROJECT.tcd PAGE OUTPUT.svg
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
    let revision = 0;
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
