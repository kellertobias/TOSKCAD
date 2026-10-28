//! Editable source fixture shared with the genuine native spline trim regression.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, SplineBasis, source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, ExtrudeRequest},
    parameters::BoundExpression,
    project_file::{self, ContentHash, ProjectFile},
    sketch::Datum,
    sketch_regions::ExactCurve,
    sketch_trim::{Endpoint, ExtendSpline, TrimCurve, TrimPolicy, TrimRequest},
};
use multicad_kernel::{GeometryPolicy, NativeRegionQueries};
use multicad_model::Compiler;
use std::{collections::BTreeSet, io::Write, path::Path};
pub type Error = Box<dyn std::error::Error>;
#[derive(Clone, Copy, Debug)]
pub enum BoundaryKind {
    Line,
    Arc,
    Circle,
    Spline,
}
pub struct Fixture {
    pub session: Session,
    pub sketch: EntityId,
    pub target: EntityId,
    pub boundary: EntityId,
    pub cut_parameter: EntityId,
}
pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
pub fn source(f: &Fixture) -> Result<Source, Error> {
    Ok(Source::read_for_evaluation(f.session.document(), f.sketch, f.session.revision(), None)?.0)
}
pub fn apply(f: &mut Fixture, label: &str, edits: Vec<Mutation>) -> Result<(), Error> {
    f.session.apply(Request {
        expected_revision: f.session.revision(),
        actor: actor(),
        label: label.into(),
        edits,
    })?;
    Ok(())
}
pub fn line(s: &mut Source, a: [f64; 2], b: [f64; 2], construction: bool) -> EntityId {
    let a = s.add_point(a);
    let b = s.add_point(b);
    let id = s.add_line(a, b);
    s.sketch.curves.last_mut().expect("line").construction = construction;
    id
}
fn add_spline(
    s: &mut Source,
    p: &[[f64; 2]],
    weights: Option<Vec<f64>>,
    construction: bool,
) -> EntityId {
    let poles = p.iter().map(|p| s.add_point(*p)).collect();
    let mut basis = SplineBasis::open_uniform(p.len() as u32 - 1, p.len()).expect("fixture basis");
    basis.weights = weights;
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction,
        kind: CurveKind::BSpline { poles, basis },
    });
    id
}
pub fn seed(rational: bool, boundary: BoundaryKind) -> Result<Fixture, Error> {
    let (sketch, mut edits) = Source::create(Datum::Xy, "Spline trim source")?;
    let parameter = Entity::new(
        "cut_x",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let cut_parameter = parameter.id;
    edits.push(Mutation::Insert(parameter));
    let doc = Document::new(if rational {
        "Rational spline exact edits"
    } else {
        "Polynomial spline exact edits"
    })?
    .edited(&edits)?;
    let mut s = Source::read(&doc, sketch)?.0;
    let target = if rational {
        add_spline(
            &mut s,
            &[[0., 0.], [10., 10.], [20., 0.]],
            Some(vec![1., 0.5, 1.]),
            false,
        )
    } else {
        add_spline(
            &mut s,
            &[[0., 0.], [5., 10.], [15., 10.], [20., 0.]],
            None,
            false,
        )
    };
    let start = s.add_point([0., 0.]);
    s.add_constraint(ConstraintKind::Fix { point: start });
    let boundary = match boundary {
        BoundaryKind::Line => {
            let id = line(&mut s, [10., -20.], [10., 20.], true);
            let CurveKind::Line { start: a, .. } = s.sketch.curves.last().expect("line").kind
            else {
                unreachable!()
            };
            s.add_constraint(ConstraintKind::Vertical { line: id });
            s.add_constraint(ConstraintKind::Distance {
                a: start,
                b: a,
                axis: Axis::X,
                dimension: Dimension::Expression {
                    expression: BoundExpression::bind(&doc, "=$cut_x", "mm", None)?,
                },
            });
            id
        }
        BoundaryKind::Circle => {
            let center = s.add_point([10., 0.]);
            let id = EntityId::new();
            s.sketch.curves.push(Curve {
                id,
                construction: true,
                kind: CurveKind::Circle {
                    center,
                    radius_mm: 9.,
                },
            });
            id
        }
        BoundaryKind::Arc => {
            let center = s.add_point([10., 0.]);
            let a = s.add_point([19., 0.]);
            let b = s.add_point([1., 0.]);
            let id = EntityId::new();
            s.sketch.curves.push(Curve {
                id,
                construction: true,
                kind: CurveKind::Arc {
                    center,
                    start: a,
                    end: b,
                    clockwise: false,
                },
            });
            id
        }
        BoundaryKind::Spline => add_spline(
            &mut s,
            &[[10., -20.], [10., -5.], [10., 5.], [10., 20.]],
            None,
            true,
        ),
    };
    let doc = doc.edited(&s.replace(&doc, sketch)?)?;
    Ok(Fixture {
        session: Session::new(doc)?,
        sketch,
        target,
        boundary,
        cut_parameter,
    })
}
pub fn current_curve(f: &Fixture) -> Result<ExactCurve, Error> {
    let evaluated = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        f.sketch,
        None,
    )?;
    let solved = evaluated.solved().ok_or("Unsolved fixture")?;
    let c = solved
        .curves
        .iter()
        .find(|c| c.id == f.target)
        .ok_or("Missing fixture target")?;
    let CurveKind::BSpline { poles, basis } = &c.kind else {
        return Err("Target no longer spline".into());
    };
    Ok(ExactCurve::BSpline {
        poles: poles
            .iter()
            .map(|id| {
                solved
                    .points
                    .iter()
                    .find(|p| p.id == *id)
                    .expect("pole")
                    .position_mm
            })
            .collect(),
        basis: basis.clone(),
    })
}
pub fn point_at(f: &Fixture, t: f64) -> Result<[f64; 2], Error> {
    let native = NativeRegionQueries::curve(&current_curve(f)?);
    let policy = GeometryPolicy::default();
    let [a, b] = native.domain(policy)?;
    Ok(native.evaluate(a + (b - a) * t, policy)?.point)
}
pub fn trim(f: &Fixture, t: f64) -> Result<multicad_solver::TrimEdit, Error> {
    let request = TrimRequest::TrimCurve(TrimCurve {
        target: f.target,
        pick_mm: point_at(f, t)?,
        boundaries: BTreeSet::from([f.boundary]),
    });
    Ok(multicad_solver::prepare_trim_with_queries(
        &f.session,
        f.session.revision(),
        actor(),
        f.sketch,
        None,
        &request,
        TrimPolicy::default(),
        &NativeRegionQueries::default(),
    )?)
}
pub fn continue_request(f: &mut Fixture, endpoint: Endpoint) -> Result<ExtendSpline, Error> {
    let mut s = source(f)?;
    let xs = match endpoint {
        Endpoint::Start => [-5., -10.],
        Endpoint::End => [25., 30.],
    };
    let boundaries = xs
        .into_iter()
        .map(|x| line(&mut s, [x, -100.], [x, 100.], true))
        .collect();
    let edits = s.replace(f.session.document(), f.sketch)?;
    apply(f, "Add explicit continuation boundaries", edits)?;
    Ok(ExtendSpline {
        target: f.target,
        endpoint,
        boundaries,
        max_length_mm: 40.,
        contact_index: None,
    })
}
pub fn archive(doc: &Document) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(doc.clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
}
pub fn reopen(bytes: &[u8]) -> Result<Session, Error> {
    let doc = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project
    .document;
    Ok(Session::new(doc)?)
}
pub fn close_and_extrude(f: &mut Fixture) -> Result<(EntityId, EntityId, f64), Error> {
    let mut s = source(f)?;
    let CurveKind::BSpline { poles, .. } = &s
        .sketch
        .curves
        .iter()
        .find(|c| c.id == f.target)
        .ok_or("Missing target")?
        .kind
    else {
        return Err("No spline".into());
    };
    let a = poles[0];
    let b = *poles.last().ok_or("No poles")?;
    s.add_line(b, a);
    let edits = s.replace(f.session.document(), f.sketch)?;
    apply(f, "Close retained spline region", edits)?;
    let evaluated = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        f.sketch,
        None,
    )?;
    let regions = evaluated.regions_with_queries(&NativeRegionQueries::default())?;
    if regions.regions.len() != 1 || !regions.diagnostics.is_empty() {
        return Err(format!(
            "Expected one exact closed region: {:?}",
            regions.diagnostics
        )
        .into());
    }
    let area = regions.regions[0].area_mm2;
    let request = ExtrudeRequest::new(
        f.sketch,
        vec![regions.regions[0].intent()],
        Default::default(),
    );
    let prepared = model::prepare_extrude(&f.session, f.session.revision(), actor(), &request)?;
    let output = prepared.outputs[0].clone();
    Compiler::default().compile(prepared.prepared.document(), f.session.revision())?;
    f.session.commit(prepared.prepared)?;
    Ok((output.body, output.feature, area))
}
fn save(directory: &Path, name: &str, f: &Fixture) -> Result<serde_json::Value, Error> {
    let bytes = archive(f.session.document())?;
    let reopened = reopen(&bytes)?;
    if reopened.document() != f.session.document() {
        return Err("Native source roundtrip mismatch".into());
    }
    let model = Compiler::default().compile(reopened.document(), 0)?;
    let metrics:Vec<_>=model.snapshot().bodies.iter().map(|(body,producer)|serde_json::json!({"body":body,"producer":producer,"volume_mm3":model.snapshot().shapes[producer].volume_mm3})).collect();
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join(name))?;
    Ok(
        serde_json::json!({"file":name,"document":f.session.document().id(),"sketch":f.sketch,"target":f.target,"boundary":f.boundary,"cut_x":f.cut_parameter,"source_hash":ContentHash::document(f.session.document())?,"archive_sha256":ContentHash::of(&bytes).to_string(),"native_solids":metrics}),
    )
}
pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let mut files = vec![];
    for rational in [false, true] {
        let label = if rational { "rational" } else { "polynomial" };
        let mut f = seed(rational, BoundaryKind::Line)?;
        files.push(save(directory, &format!("{label}-input.multicad"), &f)?);
        let edit = trim(&f, 0.9)?;
        f.session.commit(
            edit.edit
                .preview
                .ok_or("Native trim has no valid solved candidate")?,
        )?;
        files.push(save(directory, &format!("{label}-trimmed.multicad"), &f)?);
        close_and_extrude(&mut f)?;
        files.push(save(
            directory,
            &format!("{label}-closed-solid.multicad"),
            &f,
        )?);
    }
    for endpoint in [Endpoint::Start, Endpoint::End] {
        let label = if endpoint == Endpoint::Start {
            "start"
        } else {
            "end"
        };
        let mut f = seed(true, BoundaryKind::Line)?;
        let mut r = continue_request(&mut f, endpoint)?;
        files.push(save(
            directory,
            &format!("{label}-continuation-input.multicad"),
            &f,
        )?);
        let contacts = multicad_solver::extension_contacts_with_queries(
            &f.session,
            f.session.revision(),
            f.sketch,
            None,
            &r,
            Default::default(),
            &NativeRegionQueries::default(),
        )?;
        if contacts.len() != 2 {
            return Err("Expected two explicit native continuation choices".into());
        }
        r.contact_index = Some(if endpoint == Endpoint::Start { 0 } else { 1 });
        let edit = multicad_solver::prepare_trim_with_queries(
            &f.session,
            f.session.revision(),
            actor(),
            f.sketch,
            None,
            &TrimRequest::ExtendSpline(r),
            Default::default(),
            &NativeRegionQueries::default(),
        )?;
        f.session
            .commit(edit.edit.preview.ok_or("No valid continuation candidate")?)?;
        files.push(save(directory, &format!("{label}-continued.multicad"), &f)?);
    }
    let manifest = serde_json::json!({"generator":"spline_trim_review","projects":files,"policy":{"continuation":"finite straight geometric tangent joined as one exact clamped B-spline; original curve retained","maximum_length_mm":1000000,"maximum_contacts":256,"ambiguous_contacts":"explicit current choice required","constraints":"changed control-pole/curve constraints reject rather than detach","unsupported":["closed spline seam-aware trim","unbounded analytic extension against spline boundary"]},"verification":{"native_geometry":true,"archive_reopen":true,"physical_native_ui_acceptance":false,"windows_linux_runtime":false}});
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join("manifest.json"))?;
    Ok(manifest)
}
#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("Usage: spline_trim_review NEW_DIRECTORY")?;
    let manifest = generate(Path::new(&directory))?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
