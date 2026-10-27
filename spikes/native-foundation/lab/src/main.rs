pub use multicad_ui::{camera, icons, navigation};
mod geometry;
mod raster;
mod session;
mod solver;
mod ui;
mod viewport;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => ui::launch().map_err(|e| e.to_string().into()),
        [flag, path] if flag == "--verify" => geometry::verify(std::path::Path::new(path)),
        [flag, path] if flag == "--benchmark-ui" => ui::benchmark(std::path::Path::new(path)),
        _ => Err("Usage: multicad-foundation-lab [--verify OUTPUT_DIRECTORY]".into()),
    }
}
