//! Native editable symbol fixtures, shared verbatim with the model regressions.
//! Fixture preflight compiles candidates before committing; this does not claim
//! that source-only core preparations independently certify native geometry.
#[allow(dead_code)]
#[path = "parameter_workflow_review.rs"]
pub mod panel;
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::{
        binding::{self, DimensionInput},
        source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::{self, AssemblyRequest, ComponentPlacementRequest, NumericTransform},
    parameters::{BoundExpression, Unit},
    project_file::{self, ContentHash, ProjectFile},
    sketch::Geometry,
    symbols::{
        self, SymbolAuthorRequest, SymbolDefinition, SymbolKind, SymbolPlaceRequest, SymbolPlan,
        SymbolUpdateRequest, SymbolValue,
    },
};
use multicad_model::{Compiler, Model};
use std::{collections::BTreeMap, io::Write, path::Path};
pub type Error = Box<dyn std::error::Error>;
pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
pub struct Library {
    pub panel: panel::Fixture,
    pub author: SymbolAuthorRequest,
    pub members: Vec<EntityId>,
}
pub struct Fixture {
    pub library: Library,
    pub session: Session,
    pub definition: EntityId,
    pub placed: [SymbolPlaceRequest; 2],
    pub plans: BTreeMap<EntityId, EntityId>,
    pub assembly: bool,
}
pub fn checked(session: &mut Session, prepared: Prepared) -> Result<(), Error> {
    let model = Compiler::default().compile(prepared.document(), session.revision())?;
    if !model
        .snapshot()
        .references
        .for_model(prepared.document())?
        .all_resolved()
    {
        return Err("Fixture native references are unresolved".into());
    }
    session.commit(prepared)?;
    Ok(())
}
pub fn library(assembly: bool, kind: SymbolKind) -> Result<Library, Error> {
    let mut panel = panel::seed()?;
    let mut members = vec![];
    let root = if assembly {
        for index in 0..2 {
            let mut parameters = NumericTransform::default();
            if index == 1 {
                parameters.translation[1] =
                    multicad_core::constraint_sketch::Dimension::Expression {
                        expression: BoundExpression::bind(
                            panel.session.document(),
                            "=$height + 20 mm",
                            "mm",
                            None,
                        )?,
                    };
            }
            let request = ComponentPlacementRequest::new(panel.output.body, parameters);
            let prepared = model::prepare_place_component(
                &panel.session,
                panel.session.revision(),
                actor(),
                &request,
            )?;
            checked(&mut panel.session, prepared.prepared)?;
            members.push(request.instance);
        }
        let request = AssemblyRequest::new(
            "Two editable constrained panels",
            members.clone(),
            vec![panel.output.body],
        );
        let prepared =
            model::prepare_assembly(&panel.session, panel.session.revision(), actor(), &request)?;
        checked(&mut panel.session, prepared.prepared)?;
        request.assembly
    } else {
        panel.output.body
    };
    let exposed = if kind == SymbolKind::Dynamic {
        vec![panel.width, panel.height, panel.stock]
    } else {
        vec![]
    };
    let author = SymbolAuthorRequest::new(
        "Constrained panel symbol",
        root,
        kind,
        exposed,
        Some(SymbolPlan {
            sketch: panel.sketch,
        }),
    );
    let prepared =
        symbols::prepare_author(&panel.session, panel.session.revision(), actor(), &author)?;
    checked(&mut panel.session, prepared.prepared)?;
    Ok(Library {
        panel,
        author,
        members,
    })
}
pub fn values(
    library: &Library,
    width: f64,
    height: f64,
    stock: f64,
) -> BTreeMap<EntityId, SymbolValue> {
    BTreeMap::from([
        (
            library.panel.width,
            SymbolValue {
                value: width,
                unit: "mm".into(),
            },
        ),
        (
            library.panel.height,
            SymbolValue {
                value: height,
                unit: "mm".into(),
            },
        ),
        (
            library.panel.stock,
            SymbolValue {
                value: stock,
                unit: "mm".into(),
            },
        ),
    ])
}
pub fn seed(assembly: bool) -> Result<Fixture, Error> {
    let library = library(assembly, SymbolKind::Dynamic)?;
    let definition_source =
        symbols::read_definition(library.panel.session.document(), library.author.definition)?;
    let decoy = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "999".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let mut session = Session::new(
        Document::new("Pinned symbols: independent dimensions")?
            .edited(&[Mutation::Insert(decoy)])?,
    )?;
    let imported =
        symbols::prepare_import(&session, session.revision(), actor(), &definition_source)?;
    let definition = imported.definition;
    checked(&mut session, imported.prepared)?;
    let first = symbols::SymbolPlaceRequest::new(
        session.document(),
        definition,
        "Panel A · 150 × 90",
        {
            let mut v = values(&library, 150., 90., 6.);
            v.insert(
                library.panel.width,
                SymbolValue {
                    value: 15.,
                    unit: "cm".into(),
                },
            );
            v
        },
        Placement {
            translation_mm: [250., 50., 0.],
            ..Default::default()
        },
    )?;
    let second = symbols::SymbolPlaceRequest::new(
        session.document(),
        definition,
        "Panel B · rotated 200 × 100",
        values(&library, 200., 100., 10.),
        Placement {
            translation_mm: [700., 50., 0.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        },
    )?;
    let placed = [first, second];
    let mut plans = BTreeMap::new();
    for request in &placed {
        let prepared = symbols::prepare_place(&session, session.revision(), actor(), request)?;
        plans.extend(prepared.plans);
        checked(&mut session, prepared.prepared)?;
    }
    Ok(Fixture {
        library,
        session,
        definition,
        placed,
        plans,
        assembly,
    })
}
pub fn near(a: f64, b: f64) -> Result<(), Error> {
    if (a - b).abs() > 2e-5 {
        return Err(format!("Native value {a} differs from independent expectation {b}").into());
    }
    Ok(())
}
pub fn verify(f: &Fixture, expected: [[f64; 3]; 2]) -> Result<(), Error> {
    let native = Compiler::default().compile(f.session.document(), f.session.revision())?;
    for (index, request) in f.placed.iter().enumerate() {
        verify_instance(f, &native, index, expected[index])?;
        if symbols::symbol_plan(f.session.document(), request.instance)?
            != Some((request.instance, f.plans[&request.instance]))
        {
            return Err("Plan is not linked to its canonical symbol instance".into());
        }
    }
    let expected_leaves = if f.assembly { 4 } else { 2 };
    if native.snapshot().leaf_occurrences.len() != expected_leaves {
        return Err(
            "Definitions were incorrectly emitted as additional physical symbol copies".into(),
        );
    }
    Ok(())
}
fn verify_instance(
    f: &Fixture,
    native: &Model,
    index: usize,
    expected: [f64; 3],
) -> Result<(), Error> {
    let request = &f.placed[index];
    let instance = symbols::read_instance(f.session.document(), request.record)?;
    if instance.instance != request.instance || instance.ids != request.ids {
        return Err("Compatible symbol source identity changed".into());
    }
    let root = native
        .solid(request.instance)
        .ok_or("Missing exact placed symbol solid")?;
    let count = if f.assembly { 2 } else { 1 };
    if root.solid_count()? != count {
        return Err("Native symbol has the wrong number of actual solids".into());
    }
    near(
        root.volume_mm3()?,
        expected.into_iter().product::<f64>() * f64::from(count),
    )?;
    let extent_y = if f.assembly {
        2. * expected[1] + 20.
    } else {
        expected[1]
    };
    let bounds = root.bounds()?;
    let (want_min, want_max) = if index == 0 {
        (
            [250., 50., 0.],
            [250. + expected[0], 50. + extent_y, expected[2]],
        )
    } else {
        (
            [700. - extent_y, 50., 0.],
            [700., 50. + expected[0], expected[2]],
        )
    };
    for axis in 0..3 {
        near(bounds.min[axis], want_min[axis])?;
        near(bounds.max[axis], want_max[axis])?;
    }
    let mapped_sketch = instance.ids[&f.library.panel.sketch];
    if f.plans[&request.instance] != mapped_sketch {
        return Err("Plan escaped its owned per-instance sketch".into());
    }
    let solved = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        mapped_sketch,
        Some(&native.snapshot().references),
    )?;
    if solved.evaluation.diagnostics.state != multicad_solver::SolveState::FullyConstrained
        || solved.evaluation.diagnostics.degrees_of_freedom != 0
    {
        return Err("Symbol rectangle lost its internal constraints".into());
    }
    near(
        solved.regions()?.regions[0].area_mm2,
        expected[0] * expected[1],
    )?;
    near(
        solved.resolved_rhs[&instance.ids[&f.library.panel.width_dimension]].value,
        expected[0],
    )?;
    near(
        solved.resolved_rhs[&instance.ids[&f.library.panel.height_dimension]].value,
        expected[1],
    )?;
    near(
        f.session
            .parameters()
            .value(instance.ids[&f.library.panel.depth], Unit::Millimeter)?,
        expected[2],
    )?;
    let original = Source::read_for_evaluation(
        f.library.panel.session.document(),
        f.library.panel.sketch,
        f.library.panel.session.revision(),
        None,
    )?
    .0;
    let mapped = Source::read_for_evaluation(
        f.session.document(),
        mapped_sketch,
        f.session.revision(),
        Some(&native.snapshot().references),
    )?
    .0;
    if original.sketch.constraints.len() != mapped.sketch.constraints.len()
        || original.sketch.curves.len() != mapped.sketch.curves.len()
    {
        return Err("Symbol silently duplicated or discarded internal sketch constraints".into());
    }
    let pose = native
        .snapshot()
        .occurrences
        .get(&request.instance)
        .ok_or("No native symbol placement receipt")?
        .pose;
    let leaf = native
        .snapshot()
        .leaf_occurrences
        .values()
        .find(|leaf| {
            leaf.path.root == request.instance
                && (!f.assembly
                    || leaf
                        .path
                        .members
                        .contains(&instance.ids[&f.library.members[0]]))
        })
        .ok_or("No physical leaf for symbol plan comparison")?;
    let mut plan_min = [f64::INFINITY; 3];
    let mut plan_max = [f64::NEG_INFINITY; 3];
    for geometry in solved.geometry()? {
        if let Geometry::Line { a, b, .. } = geometry {
            for point in [a, b] {
                let world = pose.point(solved.frame.point(point));
                for axis in 0..3 {
                    plan_min[axis] = plan_min[axis].min(world[axis]);
                    plan_max[axis] = plan_max[axis].max(world[axis]);
                }
            }
        }
    }
    for axis in 0..2 {
        near(plan_min[axis], leaf.bounds.min[axis])?;
        near(plan_max[axis], leaf.bounds.max[axis])?;
    }
    near(plan_min[2], leaf.bounds.min[2])?;
    near(plan_max[2], leaf.bounds.min[2])?;
    Ok(())
}
pub fn publish_revision_two(library: &mut Library) -> Result<SymbolDefinition, Error> {
    let mut edits = vec![];
    for (id, value) in [
        (library.panel.width, "140"),
        (library.panel.height, "95"),
        (library.panel.stock, "8"),
    ] {
        let mut e = library
            .panel
            .session
            .document()
            .get(id)
            .ok_or("Missing library parameter")?
            .clone();
        let EntityData::Parameter { expression, .. } = &mut e.data else {
            return Err("Wrong library parameter kind".into());
        };
        *expression = value.into();
        edits.push(Mutation::Replace(e));
    }
    let mut source = Source::read_for_evaluation(
        library.panel.session.document(),
        library.panel.sketch,
        library.panel.session.revision(),
        None,
    )?
    .0;
    source = binding::draft_dimension(
        library.panel.session.document(),
        &source,
        library.panel.width_dimension,
        &DimensionInput::Expression {
            text: "=$width - 10 mm".into(),
            unit: "mm".into(),
        },
        false,
    )?;
    edits.extend(source.replace(library.panel.session.document(), library.panel.sketch)?);
    let prepared = library.panel.session.prepare(Request {
        expected_revision: library.panel.session.revision(),
        actor: actor(),
        label: "Author changed constraints and defaults".into(),
        edits,
    })?;
    checked(&mut library.panel.session, prepared)?;
    let mut author = SymbolAuthorRequest::new(
        "Constrained panel symbol",
        library.author.root,
        SymbolKind::Dynamic,
        vec![
            library.panel.width,
            library.panel.stock,
            library.panel.depth,
        ],
        Some(SymbolPlan {
            sketch: library.panel.sketch,
        }),
    );
    author.symbol = library.author.symbol;
    author.revision = "2".into();
    let prepared = symbols::prepare_author(
        &library.panel.session,
        library.panel.session.revision(),
        actor(),
        &author,
    )?;
    let id = prepared.definition;
    checked(&mut library.panel.session, prepared.prepared)?;
    Ok(symbols::read_definition(
        library.panel.session.document(),
        id,
    )?)
}
pub fn update(f: &mut Fixture, replacement: SymbolDefinition) -> Result<(), Error> {
    let mut request = SymbolUpdateRequest::new(f.session.document(), f.definition, replacement)?;
    for values in request.instance_overrides.values_mut() {
        values.remove(&f.library.panel.height);
    }
    let prepared = symbols::prepare_update(&f.session, f.session.revision(), actor(), &request)?;
    for (instance, plan) in &prepared.plans {
        if f.plans.get(instance) != Some(plan) {
            return Err("Compatible definition upgrade changed the linked plan identity".into());
        }
    }
    checked(&mut f.session, prepared.prepared)
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
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if !opened.project.resources.payloads.is_empty() {
        return Err(
            "Review host unexpectedly depends on external or sidecar library resources".into(),
        );
    }
    Ok(Session::new(opened.project.document)?)
}
fn save(
    directory: &Path,
    name: &str,
    f: &Fixture,
    expected: [[f64; 3]; 2],
) -> Result<serde_json::Value, Error> {
    verify(f, expected)?;
    let bytes = archive(f.session.document())?;
    let reopened = reopen(&bytes)?;
    if reopened.document() != f.session.document() {
        return Err("Offline reopen changed pinned symbol source".into());
    }
    let native = Compiler::default().compile(reopened.document(), reopened.revision())?;
    for (i, r) in f.placed.iter().enumerate() {
        near(
            native
                .solid(r.instance)
                .ok_or("Reopened native symbol missing")?
                .volume_mm3()?,
            expected[i].into_iter().product::<f64>() * if f.assembly { 2. } else { 1. },
        )?;
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join(name))?;
    Ok(
        serde_json::json!({"file":name,"definition":f.definition,"instances":f.placed.iter().map(|r|r.instance).collect::<Vec<_>>(),"plans":f.plans.iter().map(|(instance,sketch)|serde_json::json!({"instance":instance,"sketch":sketch})).collect::<Vec<_>>(),"expected_local_dimensions_mm":expected,"solid_count_per_instance":if f.assembly{2}else{1},"source_hash":ContentHash::document(f.session.document())?,"archive_sha256":ContentHash::of(&bytes)}),
    )
}
pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let mut projects = vec![];
    for assembly in [false, true] {
        let mut f = seed(assembly)?;
        let label = if assembly { "assembly" } else { "part" };
        projects.push(save(
            directory,
            &format!("{label}-r1.multicad"),
            &f,
            [[150., 90., 8.], [200., 100., 12.]],
        )?);
        let replacement = publish_revision_two(&mut f.library)?;
        verify(&f, [[150., 90., 8.], [200., 100., 12.]])?;
        update(&mut f, replacement)?;
        projects.push(save(
            directory,
            &format!("{label}-r2.multicad"),
            &f,
            [[140., 95., 8.], [190., 95., 12.]],
        )?);
    }
    let manifest = serde_json::json!({"generator":"symbol_review","projects":projects,"verification":{"real_native_geometry":true,"constrained_sketch":true,"linked_plan_world_pose":true,"offline_reopen_without_library":true,"physical_native_ui_acceptance":false,"windows_linux_runtime":false}});
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join("manifest.json"))?;
    Ok(manifest)
}
#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Usage: cargo run -p multicad-app --example symbol_review -- NEW_DIRECTORY")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&generate(Path::new(&path))?)?
    );
    Ok(())
}
