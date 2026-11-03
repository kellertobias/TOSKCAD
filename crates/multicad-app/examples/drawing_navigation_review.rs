//! CAD-226 production PDF controller/backend evidence; creates no OS window.
#[path = "../src/exchange.rs"]
mod exchange;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::{self, DrawingView, Page, PageAction, Scale, TextNote, ViewKind, ViewSource},
    exchange::PageRef,
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    project_file::{self, ProjectFile},
};
use multicad_ui::exchange::{DrawingPdfScope, ExportWorkspace};
use std::{
    path::Path,
    time::{Duration, Instant},
};
type Error = Box<dyn std::error::Error>;

pub fn fixture() -> Result<(Session, PageRef), Error> {
    let mut s = Session::new(Document::new("Drawing navigation review")?)?;
    let actor = Actor::local(Channel::Ui);
    let request = PrimitiveRequest::new(
        "Machined plate",
        PrimitiveParameters::Box {
            size: [length(60.), length(40.), length(8.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, s.revision(), actor.clone(), &request)?;
    s.commit(prepared.prepared)?;
    let part = s
        .document()
        .entities()
        .find(|e| matches!(e.data, multicad_core::document::EntityData::Part { .. }))
        .unwrap()
        .id;
    let mut selected = None;
    for (name, sheets) in [
        (
            "Construction",
            vec![("A1", "Plate overview"), ("A2", "Plate manufacturing")],
        ),
        ("Hardware", vec![("H1", "Fastener schedule")]),
    ] {
        let mut book = None;
        for (number, title) in sheets {
            let mut page = Page::new(number);
            page.info.title = title.into();
            page.info.project = "CAD-226 production export".into();
            page.notes.push(TextNote {
                id: EntityId::new(),
                position_mm: [20., 20.],
                text: format!("{number}: {title} - native plate 60 x 40 x 8 mm"),
            });
            page.views.push(DrawingView {
                id: EntityId::new(),
                name: "Top projection".into(),
                source: ViewSource::Model {
                    entities: vec![part],
                },
                frame: multicad_core::references::ReferenceFrame {
                    origin_mm: [0.; 3],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
                kind: ViewKind::Projection,
                position_mm: [90., 90.],
                scale: Scale {
                    numerator: 1,
                    denominator: 1,
                },
                hidden_lines: false,
                tangent_edges: false,
                paper_chord_tolerance_mm: 0.05,
                guide: None,
                equipment_categories: None,
            });
            let page_id = page.id;
            if let Some(id) = book {
                let p =
                    drawings::prepare(&s, s.revision(), actor.clone(), id, PageAction::Add(page))?;
                s.commit(p)?;
            } else {
                let (id, p) =
                    drawings::prepare_create(&s, s.revision(), actor.clone(), name, Some(page))?;
                s.commit(p)?;
                book = Some(id);
                selected.get_or_insert(PageRef {
                    book: id,
                    page: page_id,
                });
            }
        }
    }
    Ok((s, selected.unwrap()))
}
pub fn export_scopes(root: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let (s, page) = fixture()?;
    let source = s.document().clone();
    let revision = s.revision();
    project_file::write(
        &ProjectFile::new(source.clone())?,
        &mut std::fs::File::create(root.join("drawings.tcd"))?,
        Default::default(),
    )?;
    let mut w = ExportWorkspace::default();
    w.set_backend(Box::new(exchange::Backend::new()))?;
    let mut results = vec![];
    for (name, scope, count) in [
        ("sheet", DrawingPdfScope::Sheet, 1),
        ("drawing", DrawingPdfScope::Drawing, 2),
        ("project", DrawingPdfScope::Project, 3),
    ] {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir)?;
        assert!(w.set_directory(dir.to_str().unwrap()).is_none());
        w.open_drawing_pdf(&s, page.clone())?;
        w.set_drawing_pdf_scope(scope)?;
        let started = Instant::now();
        w.start_drawing_pdf(&s, Channel::Ui)?;
        while w.pending() {
            if started.elapsed() > Duration::from_secs(60) {
                return Err("Native PDF timeout".into());
            }
            w.poll(&s);
            std::thread::sleep(Duration::from_millis(2));
        }
        let (notice, error) = w.take_notice().ok_or("Missing PDF result")?;
        if error {
            return Err(notice.into());
        }
        assert!(notice.contains(&format!("{count}-sheet PDF")), "{notice}");
        assert_eq!(s.document(), &source);
        assert_eq!(s.revision(), revision);
        let bundle = std::fs::read_dir(&dir)?
            .map(|e| e.unwrap().path())
            .find(|p| p.is_dir())
            .ok_or("Missing published bundle")?;
        let pdf = std::fs::read_dir(&bundle)?
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "pdf"))
            .ok_or("Missing PDF")?;
        let bytes = std::fs::read(&pdf)?;
        assert!(bytes.starts_with(b"%PDF-"));
        results.push(serde_json::json!({"scope":name,"expected_pages":count,"pdf":pdf,"bytes":bytes.len(),"milliseconds":started.elapsed().as_secs_f64()*1000.,"notice":notice}));
    }
    Ok(
        serde_json::json!({"scope":"Production ExportWorkspace -> native ExportWorker -> existing PDF publisher; no OS-window interaction", "source_unchanged":true,"revision":revision,"results":results}),
    )
}
fn main() -> Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Pass a new output directory")?;
    let report = export_scopes(Path::new(&root))?;
    std::fs::write(
        Path::new(&root).join("native-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_controller_exports_each_scope_without_source_edits() {
        let root = tempfile::tempdir().unwrap();
        super::export_scopes(root.path()).unwrap();
    }
}
