//! Editable native scale projects for application review, generated without a UI.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, InstanceMoveRequest,
        NumericTransform, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, ScaleBatchRequest,
        ScaleParameters, ScaleTarget, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
use multicad_model::Compiler;
use std::{
    fs::OpenOptions,
    io::{Cursor, Write},
    path::Path,
};
type Error = Box<dyn std::error::Error>;
type ReviewProjects = (Vec<(&'static str, ProjectFile)>, serde_json::Value);
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
struct Fixture {
    session: Session,
    stock: PrimitiveRequest,
    instance: EntityId,
    other: EntityId,
}
fn expression(session: &Session, text: &str, unit: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(session.document(), text, unit, None)?,
    })
}
fn build() -> Result<Fixture, Error> {
    let mut document = Document::new("Scale review: shared stock and independent occurrences")?;
    for (name, value, unit) in [
        ("width", 60., "mm"),
        ("uniform_factor", 1.5, "ratio"),
        ("factor_x", 2., "ratio"),
        ("factor_y", 1.5, "ratio"),
        ("factor_z", 0.5, "ratio"),
    ] {
        document.insert(Entity::new(
            name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        ))?;
    }
    let mut session = Session::new(document)?;
    let stock = PrimitiveRequest::new(
        "Stock definition, width = $width",
        PrimitiveParameters::Box {
            size: [
                expression(&session, "=$width", "mm")?,
                length(40.),
                length(12.),
            ],
            frame: Default::default(),
        },
    );
    let cylinder = PrimitiveRequest::new(
        "Unchanged round stock",
        PrimitiveParameters::Cylinder {
            radius: length(15.),
            height: length(30.),
            frame: PrimitiveFrame {
                origin: [length(0.), length(120.), length(0.)],
                ..Default::default()
            },
        },
    );
    for part in [&stock, &cylinder] {
        session.commit(
            model::prepare_primitive(&session, session.revision(), actor(), part)?.prepared,
        )?;
    }
    let mut a = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: stock.body,
                producer: stock.feature,
            },
        },
        NumericTransform {
            translation: [length(160.), length(0.), length(0.)],
            angle: model::angle(30.),
            ..Default::default()
        },
    );
    a.name = "Linked instance A, rotate 30 degrees".into();
    let mut b = InstanceCopyRequest::new(
        a.source.clone(),
        NumericTransform {
            translation: [length(160.), length(130.), length(0.)],
            ..Default::default()
        },
    );
    b.name = "Linked instance B, unchanged reference".into();
    session.commit(
        model::prepare_instance_copies(
            &session,
            session.revision(),
            actor(),
            &[a.clone(), b.clone()],
        )?
        .prepared,
    )?;
    Ok(Fixture {
        session,
        stock,
        instance: a.instance,
        other: b.instance,
    })
}
fn checked_commit(session: &mut Session, request: &ScaleBatchRequest) -> Result<(), Error> {
    let candidate = model::prepare_scale_batch(session, session.revision(), actor(), request)?;
    Compiler::default().compile(candidate.prepared.document(), session.revision() + 1)?;
    session.commit(candidate.prepared)?;
    Ok(())
}
fn center(session: &Session, instance: EntityId) -> Result<[Dimension; 3], Error> {
    let model = Compiler::default().compile(session.document(), session.revision())?;
    let bounds = model
        .snapshot()
        .occurrences
        .get(&instance)
        .ok_or("Missing review instance")?
        .bounds;
    Ok(std::array::from_fn(|i| {
        length(bounds.min[i] + (bounds.max[i] - bounds.min[i]) * 0.5)
    }))
}
fn checked_project(session: &Session) -> Result<ProjectFile, Error> {
    Compiler::default().compile(session.document(), session.revision())?;
    let project = ProjectFile::new(session.document().clone())?;
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default())?;
    let opened = project_file::read(
        &mut Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if opened.project.document != project.document {
        return Err("Native review source did not round-trip".into());
    }
    Ok(project)
}
fn projects() -> Result<ReviewProjects, Error> {
    let mut f = build()?;
    let mut files = vec![("01-ready.multicad", checked_project(&f.session)?)];
    let request = ScaleBatchRequest::new(
        f.session.document(),
        vec![ScaleTarget::Instance {
            instance: f.instance,
        }],
        ScaleParameters {
            factors: [
                expression(&f.session, "=$factor_x", "ratio")?,
                expression(&f.session, "=$factor_y", "ratio")?,
                expression(&f.session, "=$factor_z", "ratio")?,
            ],
            pivot: center(&f.session, f.instance)?,
            ..Default::default()
        },
        false,
    )?;
    checked_commit(&mut f.session, &request)?;
    files.push((
        "02-nonuniform-instance.multicad",
        checked_project(&f.session)?,
    ));
    let uniform = expression(&f.session, "=$uniform_factor", "ratio")?;
    let copy = ScaleBatchRequest::new(
        f.session.document(),
        vec![ScaleTarget::Instance {
            instance: f.instance,
        }],
        ScaleParameters {
            factors: std::array::from_fn(|_| uniform.clone()),
            pivot: center(&f.session, f.instance)?,
            ..Default::default()
        },
        true,
    )?;
    let copied = copy.instances()[0];
    checked_commit(&mut f.session, &copy)?;
    let movement = InstanceMoveRequest::new(
        copied,
        NumericTransform {
            translation: [length(0.), length(240.), length(0.)],
            ..Default::default()
        },
    );
    let candidate = model::prepare_instance_moves(
        &f.session,
        f.session.revision(),
        actor(),
        std::slice::from_ref(&movement),
    )?;
    Compiler::default().compile(candidate.prepared.document(), f.session.revision() + 1)?;
    f.session.commit(candidate.prepared)?;
    files.push(("03-scaled-copy.multicad", checked_project(&f.session)?));
    let part = ScaleBatchRequest::new(
        f.session.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: f.stock.body,
                producer: f.stock.feature,
            },
        }],
        ScaleParameters {
            factors: std::array::from_fn(|_| uniform.clone()),
            ..Default::default()
        },
        false,
    )?;
    checked_commit(&mut f.session, &part)?;
    files.push((
        "04-definition-propagation.multicad",
        checked_project(&f.session)?,
    ));
    let manifest = serde_json::json!({"document":f.session.document().id(),"stock":f.stock.body,"primitive":f.stock.feature,"scaled_instance":f.instance,"unchanged_instance":f.other,"scaled_copy":copied,"files":files.iter().map(|(name,_)|*name).collect::<Vec<_>>(),"scope":"Canonical editable source, regenerated with production native geometry; these generated assets are not physical UI or manufacturing acceptance evidence."});
    Ok((files, manifest))
}
fn save(
    root: &Path,
    files: &[(&str, ProjectFile)],
    manifest: &serde_json::Value,
) -> Result<(), Error> {
    let manifest_path = root.join("manifest.json");
    if manifest_path.exists() || files.iter().any(|(name, _)| root.join(name).exists()) {
        return Err("Review output already exists; choose a new directory".into());
    }
    std::fs::create_dir_all(root)?;
    for (name, project) in files {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(name))?;
        project_file::write(project, &mut file, Default::default())?;
        file.sync_all()?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(manifest_path)?;
    file.write_all(&serde_json::to_vec_pretty(manifest)?)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
fn main() -> Result<(), Error> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Usage: scale_review NEW_OUTPUT_DIRECTORY")?;
    let (files, manifest) = projects()?;
    save(Path::new(&root), &files, &manifest)?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_projects_regenerate_real_scaled_occurrences_without_phantom_component_outputs() {
        let (files, manifest) = projects().unwrap();
        let stock: EntityId = serde_json::from_value(manifest["stock"].clone()).unwrap();
        let instance: EntityId =
            serde_json::from_value(manifest["scaled_instance"].clone()).unwrap();
        let other: EntityId =
            serde_json::from_value(manifest["unchanged_instance"].clone()).unwrap();
        let copy: EntityId = serde_json::from_value(manifest["scaled_copy"].clone()).unwrap();
        for (index, (_, project)) in files.iter().enumerate() {
            let model = Compiler::default().compile(&project.document, 0).unwrap();
            assert_eq!(model.snapshot().bodies.len(), 2);
            assert_eq!(
                model.snapshot().occurrences.len(),
                if index < 2 { 2 } else { 3 }
            );
            let expected = if index == 3 {
                145800.
            } else if index > 0 {
                43200.
            } else {
                28800.
            };
            assert!((model.solid(instance).unwrap().volume_mm3().unwrap() - expected).abs() < 1e-5);
            let stock_volume = if index == 3 { 97200. } else { 28800. };
            assert!(
                (model.solid(stock).unwrap().volume_mm3().unwrap() - stock_volume).abs() < 1e-5
            );
            assert!(
                (model.solid(other).unwrap().volume_mm3().unwrap() - stock_volume).abs() < 1e-5
            );
            if index >= 2 {
                let expected = if index == 3 { 492075. } else { 145800. };
                assert!((model.solid(copy).unwrap().volume_mm3().unwrap() - expected).abs() < 1e-5);
            }
        }
    }
    #[test]
    fn existing_review_artifacts_are_never_overwritten() {
        let root = std::env::temp_dir().join(format!("multicad-scale-review-{}", EntityId::new()));
        let (files, manifest) = projects().unwrap();
        save(&root, &files, &manifest).unwrap();
        let before = std::fs::read(root.join("01-ready.multicad")).unwrap();
        assert!(save(&root, &files, &manifest).is_err());
        assert_eq!(
            std::fs::read(root.join("01-ready.multicad")).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
