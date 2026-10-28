//! Editable native occurrence references. Model integration tests import these
//! same fixtures; the generator refuses to overwrite any existing directory.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, SplineBasis, source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, AssemblyRequest, ComponentPlacementRequest, CreatedBody, ExtrudeParameters,
        ExtrudeRequest, NumericTransform, OccurrencePath, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest,
    },
    parameters::BoundExpression,
    project_file::{self, ContentHash, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, TopologyKind},
    sketch::Datum,
    sketch_external::*,
};
use multicad_kernel::NativeRegionQueries;
use multicad_model::{Compiler, Model};
use std::{io::Write, path::Path};
pub type Error = Box<dyn std::error::Error>;
pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
pub fn apply(session: &mut Session, label: &str, edits: Vec<Mutation>) -> Result<(), Error> {
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: label.into(),
        edits,
    })?;
    Ok(())
}
pub fn native(session: &Session) -> Result<Model, Error> {
    Ok(Compiler::default().compile(session.document(), session.revision())?)
}
pub fn read_source(session: &Session, sketch: EntityId) -> Result<Source, Error> {
    Ok(Source::read_for_evaluation(session.document(), sketch, session.revision(), None)?.0)
}
pub fn parameter(session: &mut Session, id: EntityId, value: &str) -> Result<(), Error> {
    let mut e = session
        .document()
        .get(id)
        .ok_or("Parameter missing")?
        .clone();
    let EntityData::Parameter { expression, .. } = &mut e.data else {
        return Err("Not a parameter".into());
    };
    *expression = value.into();
    apply(session, "Edit source parameter", vec![Mutation::Replace(e)])
}
fn bound(doc: &Document, text: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(doc, text, "mm", None)?,
    })
}
fn primitive(
    session: &mut Session,
    name: &str,
    parameters: PrimitiveParameters,
) -> Result<CreatedBody, Error> {
    let request = PrimitiveRequest::new(name, parameters);
    let p = model::prepare_primitive(session, session.revision(), actor(), &request)?;
    let output = p.outputs[0].clone();
    session.commit(p.prepared)?;
    Ok(output)
}
pub fn new_sketch(session: &mut Session, datum: Datum, name: &str) -> Result<EntityId, Error> {
    let (id, edits) = Source::create(datum, name)?;
    apply(session, "Create review sketch", edits)?;
    Ok(id)
}
fn rectangle(session: &mut Session, sketch: EntityId, size: [f64; 2]) -> Result<(), Error> {
    let mut s = read_source(session, sketch)?;
    s.add_rectangle([0., 0.], size)?;
    let edits = s.replace(session.document(), sketch)?;
    apply(session, "Draw authored rectangle", edits)
}
pub fn extrude(session: &mut Session, sketch: EntityId, depth: f64) -> Result<CreatedBody, Error> {
    let model = native(session)?;
    let evaluated = multicad_solver::evaluate_document_sketch(
        session.document(),
        session.revision(),
        sketch,
        Some(&model.snapshot().references),
    )?;
    let regions = evaluated.regions_with_queries(&NativeRegionQueries::default())?;
    if regions.regions.len() != 1 {
        return Err(format!(
            "Expected exactly one native region, got {}",
            regions.regions.len()
        )
        .into());
    }
    let p = ExtrudeRequest::new(
        sketch,
        vec![regions.regions[0].intent()],
        ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: model::length(depth),
                },
            },
            ..Default::default()
        },
    );
    let p = model::prepare_extrude(session, session.revision(), actor(), &p)?;
    let output = p.outputs[0].clone();
    Compiler::default().compile(p.prepared.document(), session.revision())?;
    session.commit(p.prepared)?;
    Ok(output)
}
fn place(
    session: &mut Session,
    component: EntityId,
    parameters: NumericTransform,
) -> Result<ComponentPlacementRequest, Error> {
    let request = ComponentPlacementRequest::new(component, parameters);
    let p = model::prepare_place_component(session, session.revision(), actor(), &request)?;
    session.commit(p.prepared)?;
    Ok(request)
}
fn assembly(
    session: &mut Session,
    name: &str,
    members: Vec<EntityId>,
    parts: Vec<EntityId>,
) -> Result<AssemblyRequest, Error> {
    let r = AssemblyRequest::new(name, members, parts);
    let p = model::prepare_assembly(session, session.revision(), actor(), &r)?;
    session.commit(p.prepared)?;
    Ok(r)
}
pub fn capture(
    session: &mut Session,
    query: &ExternalQuery,
    choice: usize,
    behavior: Behavior,
) -> Result<OccurrenceCaptureRequest, Error> {
    let c = Compiler::default().query_external_catalog(
        session.document(),
        session.revision(),
        query,
        None,
    )?;
    let r = OccurrenceCaptureRequest::new(
        &c,
        &c.results
            .get(choice)
            .ok_or("Missing explicit result")?
            .selector,
        behavior,
    )?;
    let prepared = Compiler::default().prepare_external_capture(
        &session.preparation_snapshot(),
        actor(),
        &r,
        None,
    )?;
    session.commit(prepared.edit.preview.ok_or("No checked native candidate")?)?;
    Ok(r)
}
fn line_intent(session: &Session, body: EntityId) -> Result<GeometryIntent, Error> {
    let m = native(session)?;
    let topology = m
        .solid(body)
        .ok_or("Source body missing")?
        .topology_snapshot(body, session.revision())?;
    let key=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm})if a_mm[1].abs()<1e-7&&b_mm[1].abs()<1e-7&&a_mm[2].abs()<1e-7&&b_mm[2].abs()<1e-7&&(a_mm[0]-b_mm[0]).abs()>1.)).ok_or("No named bottom front edge")?.key;
    Ok(GeometryIntent::capture(&topology, key)?)
}
pub struct NestedFixture {
    pub session: Session,
    pub receiver: CreatedBody,
    pub sketch: EntityId,
    pub world_sketch: EntityId,
    pub source: CreatedBody,
    pub offset: EntityId,
    pub member: ComponentPlacementRequest,
    pub nested: ComponentPlacementRequest,
    pub roots: Vec<ComponentPlacementRequest>,
    pub query: ExternalQuery,
}
pub fn seed_nested() -> Result<NestedFixture, Error> {
    let mut doc = Document::new("Two nested cabinet occurrences")?;
    let parameter = Entity::new(
        "front_offset",
        EntityData::Parameter {
            expression: "20".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let offset = parameter.id;
    doc.insert(parameter)?;
    let mut session = Session::new(doc)?;
    let source = primitive(
        &mut session,
        "Reusable cabinet front",
        PrimitiveParameters::Box {
            size: [model::length(80.), model::length(20.), model::length(12.)],
            frame: Default::default(),
        },
    )?;
    let sketch = new_sketch(&mut session, Datum::Xy, "Consumer definition sketch")?;
    rectangle(&mut session, sketch, [120., 80.])?;
    let receiver = extrude(&mut session, sketch, 6.)?;
    let world_sketch = new_sketch(&mut session, Datum::Xy, "World projected fronts")?;
    let x = bound(session.document(), "=$front_offset")?;
    let member = place(
        &mut session,
        source.body,
        NumericTransform {
            translation: [x, model::length(20.), model::length(0.)],
            ..Default::default()
        },
    )?;
    let receiver_member = place(&mut session, receiver.body, Default::default())?;
    let cabinet = assembly(
        &mut session,
        "Cabinet definition",
        vec![member.instance, receiver_member.instance],
        vec![source.body, receiver.body],
    )?;
    let nested = place(
        &mut session,
        cabinet.assembly,
        NumericTransform {
            translation: [model::length(10.), model::length(0.), model::length(0.)],
            ..Default::default()
        },
    )?;
    let wrapper = assembly(
        &mut session,
        "Nested cabinet module",
        vec![nested.instance],
        vec![],
    )?;
    let mut roots = vec![];
    for (angle, translation) in [(30., [300., 500., 100.]), (-20., [700., -200., 50.])] {
        roots.push(place(
            &mut session,
            wrapper.assembly,
            NumericTransform {
                translation: translation.map(model::length),
                angle: model::angle(angle),
                ..Default::default()
            },
        )?);
    }
    let intent = line_intent(&session, source.body)?;
    let query = ExternalQuery {
        reference: EntityId::new(),
        sketch,
        source: OccurrenceSource {
            component: source.body,
            source: source.body,
            path: OccurrencePath {
                root: roots[0].instance,
                members: vec![nested.instance, member.instance],
            },
        },
        sketch_context: Some(OccurrenceContext {
            component: receiver.body,
            path: OccurrencePath {
                root: roots[0].instance,
                members: vec![nested.instance, receiver_member.instance],
            },
        }),
        intent,
        method: QueryMethod::Coplanar,
    };
    native(&session)?;
    Ok(NestedFixture {
        session,
        receiver,
        sketch,
        world_sketch,
        source,
        offset,
        member,
        nested,
        roots,
        query,
    })
}
pub struct CircleFixture {
    pub session: Session,
    pub source: CreatedBody,
    pub replacement: CreatedBody,
    pub sketch: EntityId,
    pub radius: EntityId,
    pub query: ExternalQuery,
    pub request: Option<OccurrenceCaptureRequest>,
    pub output: Option<CreatedBody>,
}
fn circle_intent(session: &Session, body: EntityId) -> Result<GeometryIntent, Error> {
    let m = native(session)?;
    let topology = m
        .solid(body)
        .ok_or("Cylinder source unavailable")?
        .topology_snapshot(body, session.revision())?;
    let key=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{frame,sweep_rad,..})if frame.origin_mm[2].abs()<1e-7&&(sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-7)).ok_or("No current exact cylinder rim")?.key;
    Ok(GeometryIntent::capture(&topology, key)?)
}
pub fn seed_circle() -> Result<CircleFixture, Error> {
    let mut doc = Document::new("Finite circle intersections drive panel span")?;
    let p = Entity::new(
        "probe_radius",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let radius = p.id;
    doc.insert(p)?;
    let mut session = Session::new(doc)?;
    let r = bound(session.document(), "=$probe_radius")?;
    let source = primitive(
        &mut session,
        "Original circular hardware",
        PrimitiveParameters::Cylinder {
            radius: r,
            height: model::length(20.),
            frame: Default::default(),
        },
    )?;
    let replacement = primitive(
        &mut session,
        "Replacement circular hardware",
        PrimitiveParameters::Cylinder {
            radius: model::length(8.),
            height: model::length(20.),
            frame: PrimitiveFrame {
                origin: [model::length(0.), model::length(30.), model::length(0.)],
                ..Default::default()
            },
        },
    )?;
    let sketch = new_sketch(&mut session, Datum::Yz, "Point-driven panel")?;
    rectangle(&mut session, sketch, [10., 20.])?;
    let mut s = read_source(&session, sketch)?;
    s.add_constraint(ConstraintKind::Fix {
        point: s.sketch.points[0].id,
    });
    s.add_constraint(ConstraintKind::Length {
        line: s.sketch.curves[1].id,
        dimension: model::length(20.),
    });
    let edits = s.replace(session.document(), sketch)?;
    apply(&mut session, "Anchor panel origin and height", edits)?;
    let intent = circle_intent(&session, source.body)?;
    let query = ExternalQuery {
        reference: EntityId::new(),
        sketch,
        source: OccurrenceSource {
            component: source.body,
            source: source.body,
            path: OccurrencePath {
                root: source.body,
                members: vec![],
            },
        },
        sketch_context: None,
        intent,
        method: QueryMethod::Intersect,
    };
    Ok(CircleFixture {
        session,
        source,
        replacement,
        sketch,
        radius,
        query,
        request: None,
        output: None,
    })
}
pub fn capture_circle(f: &mut CircleFixture) -> Result<(), Error> {
    let c = Compiler::default().query_external_catalog(
        f.session.document(),
        f.session.revision(),
        &f.query,
        None,
    )?;
    let index = c
        .results
        .iter()
        .position(|c| matches!(c.geometry,ExternalGeometry::Point{position_mm}if position_mm[0]>0.))
        .ok_or("No explicit positive-side crossing")?;
    let request = capture(&mut f.session, &f.query, index, Behavior::Derived)?;
    let mut s = read_source(&f.session, f.sketch)?;
    let b = s
        .sketch
        .curves
        .iter()
        .find_map(|c| match c.kind {
            CurveKind::Line { start, end }
                if s.sketch
                    .points
                    .iter()
                    .find(|p| p.id == start)
                    .is_some_and(|p| p.position_mm == [0., 0.]) =>
            {
                Some(end)
            }
            _ => None,
        })
        .ok_or("Authored panel bottom edge missing")?;
    s.add_constraint(ConstraintKind::Distance {
        a: b,
        b: request.output.points[0],
        axis: Axis::X,
        dimension: model::length(0.),
    });
    let edits = s.replace(f.session.document(), f.sketch)?;
    apply(
        &mut f.session,
        "Link panel width to chosen exact point",
        edits,
    )?;
    f.output = Some(extrude(&mut f.session, f.sketch, 5.)?);
    f.request = Some(request);
    Ok(())
}
pub fn break_circle(f: &mut CircleFixture) -> Result<(), Error> {
    let p = model::prepare_primitive_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.source.feature,
        &PrimitiveParameters::Box {
            size: [model::length(20.), model::length(20.), model::length(20.)],
            frame: Default::default(),
        },
    )?;
    f.session.commit(p.prepared)?;
    if native(&f.session).is_ok() {
        return Err(
            "Expected broken dependent solid after explicit source topology replacement".into(),
        );
    }
    Ok(())
}
pub fn repair_circle(f: &mut CircleFixture) -> Result<(), Error> {
    let previous = f.request.as_ref().ok_or("No saved circle capture")?;
    let mut query = previous.query.clone();
    query.source = OccurrenceSource {
        component: f.replacement.body,
        source: f.replacement.body,
        path: OccurrencePath {
            root: f.replacement.body,
            members: vec![],
        },
    };
    // Query only the valid replacement source. Full dependent geometry is broken.
    let replacement = Compiler::default().compile_sources(
        f.session.document(),
        f.session.revision(),
        &[f.replacement.body],
    )?;
    let topology = replacement
        .solid(f.replacement.body)
        .ok_or("Replacement missing")?
        .topology_snapshot(f.replacement.body, f.session.revision())?;
    let key=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{frame,sweep_rad,..})if frame.origin_mm[2].abs()<1e-7&&(sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-7)).ok_or("Replacement rim missing")?.key;
    query.intent = GeometryIntent::capture(&topology, key)?;
    let c = Compiler::default().query_external_catalog(
        f.session.document(),
        f.session.revision(),
        &query,
        None,
    )?;
    let choice = c
        .results
        .iter()
        .find(|c| matches!(c.geometry,ExternalGeometry::Point{position_mm}if position_mm[0]>30.))
        .ok_or("Replacement positive crossing missing")?;
    let request =
        OccurrenceCaptureRequest::repair(&c, &choice.selector, previous.behavior, previous)?;
    let prepared = Compiler::default().prepare_external_repair(
        &f.session.preparation_snapshot(),
        actor(),
        &request,
        None,
    )?;
    f.session
        .commit(prepared.edit.preview.ok_or("No native repair candidate")?)?;
    f.request = Some(request);
    Ok(())
}
pub struct SectionFixture {
    pub session: Session,
    pub source: CreatedBody,
    pub sketch: EntityId,
    pub query: ExternalQuery,
    pub output: Option<CreatedBody>,
}
pub fn seed_section() -> Result<SectionFixture, Error> {
    let mut session = Session::new(Document::new("Finite curved wall section")?)?;
    let source = primitive(
        &mut session,
        "Curved wall",
        PrimitiveParameters::Cylinder {
            radius: model::length(10.),
            height: model::length(20.),
            frame: PrimitiveFrame {
                origin: [model::length(0.), model::length(0.), model::length(-10.)],
                ..Default::default()
            },
        },
    )?;
    let sketch = new_sketch(&mut session, Datum::Xy, "Mid-height finite curved section")?;
    let m = native(&session)?;
    let topology = m
        .solid(source.body)
        .ok_or("Section source missing")?
        .topology_snapshot(source.body, session.revision())?;
    let key = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.geometry.is_none())
        .ok_or("Native cylindrical face missing")?
        .key;
    let query = ExternalQuery {
        reference: EntityId::new(),
        sketch,
        source: OccurrenceSource {
            component: source.body,
            source: source.body,
            path: OccurrencePath {
                root: source.body,
                members: vec![],
            },
        },
        sketch_context: None,
        intent: GeometryIntent::capture(&topology, key)?,
        method: QueryMethod::Intersect,
    };
    Ok(SectionFixture {
        session,
        source,
        sketch,
        query,
        output: None,
    })
}
pub fn capture_section(f: &mut SectionFixture) -> Result<(), Error> {
    capture(&mut f.session, &f.query, 0, Behavior::Derived)?;
    f.output = Some(extrude(&mut f.session, f.sketch, 5.)?);
    Ok(())
}
pub struct SplineFixture {
    pub session: Session,
    pub source_sketch: EntityId,
    pub source: CreatedBody,
    pub sketch: EntityId,
    pub query: ExternalQuery,
    pub request: Option<OccurrenceCaptureRequest>,
}
pub fn seed_spline() -> Result<SplineFixture, Error> {
    let mut session = Session::new(Document::new("Exact weighted native spline projection")?)?;
    let source_sketch = new_sketch(&mut session, Datum::Xy, "Weighted source profile")?;
    let mut s = read_source(&session, source_sketch)?;
    let poles = [[0., 0.], [10., 15.], [20., 0.]].map(|p| s.add_point(p));
    let curve = EntityId::new();
    let mut basis = SplineBasis::open_uniform(2, 3)?;
    basis.weights = Some(vec![1., 0.5, 1.]);
    s.sketch.curves.push(Curve {
        id: curve,
        construction: false,
        kind: CurveKind::BSpline {
            poles: poles.to_vec(),
            basis,
        },
    });
    s.add_line(poles[2], poles[0]);
    let edits = s.replace(session.document(), source_sketch)?;
    apply(&mut session, "Create exact rational source profile", edits)?;
    let source = extrude(&mut session, source_sketch, 8.)?;
    let sketch = new_sketch(&mut session, Datum::Xy, "Native spline reference")?;
    let m = native(&session)?;
    let topology = m
        .solid(source.body)
        .ok_or("Spline source missing")?
        .topology_snapshot(source.body, session.revision())?;
    let mut selected = None;
    for e in topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Edge && e.origins.iter().any(|o| o.source == curve))
    {
        let query = ExternalQuery {
            reference: EntityId::new(),
            sketch,
            source: OccurrenceSource {
                component: source.body,
                source: source.body,
                path: OccurrencePath {
                    root: source.body,
                    members: vec![],
                },
            },
            sketch_context: None,
            intent: match GeometryIntent::capture(&topology, e.key) {
                Ok(intent) => intent,
                Err(_) => continue,
            },
            method: QueryMethod::Coplanar,
        };
        if Compiler::default()
            .query_external_catalog(session.document(), session.revision(), &query, None)
            .is_ok_and(|c| {
                c.results.len() == 1
                    && matches!(c.results[0].geometry, ExternalGeometry::BSpline { .. })
            })
        {
            selected = Some(query);
            break;
        }
    }
    let query = selected.ok_or("No exact coplanar source spline edge")?;
    Ok(SplineFixture {
        session,
        source_sketch,
        source,
        sketch,
        query,
        request: None,
    })
}
pub fn archive(document: &Document) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(document.clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
}
pub fn reopen(bytes: &[u8]) -> Result<Session, Error> {
    Ok(Session::new(
        project_file::read(
            &mut std::io::Cursor::new(bytes),
            Default::default(),
            &project_file::supported_capabilities(),
        )?
        .project
        .document,
    )?)
}
fn save(
    directory: &Path,
    name: &str,
    session: &Session,
    expected_broken: bool,
    details: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let bytes = archive(session.document())?;
    let reopened = reopen(&bytes)?;
    if reopened.document() != session.document() {
        return Err("Archive changed canonical reference source".into());
    }
    let result = native(&reopened);
    let (native_shapes, error_text) = match result {
        Ok(model) if !expected_broken => {
            if !model
                .snapshot()
                .references
                .for_model(reopened.document())?
                .all_resolved()
            {
                return Err("Saved review project has unresolved native references".into());
            }
            let shapes=model.snapshot().leaf_occurrences.iter().map(|(path,leaf)|{
                let solid=model.leaf_solid(path).ok_or("Exact physical occurrence missing from review model")?;
                Ok(serde_json::json!({"path":path,"part":leaf.component,"feature":leaf.producer,"world_pose":leaf.pose,"volume_mm3":solid.volume_mm3()?,"bounds_mm":{"min":leaf.bounds.min,"max":leaf.bounds.max}}))
            }).collect::<Result<Vec<_>,Error>>()?;
            (shapes, None)
        }
        Err(e) if expected_broken => (vec![], Some(e.to_string())),
        Ok(_) => return Err("Expected broken consumer unexpectedly compiled".into()),
        Err(e) => return Err(e),
    };
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join(name))?;
    Ok(
        serde_json::json!({"file":name,"document":session.document().id(),"source_hash":ContentHash::document(session.document())?,"archive_sha256":ContentHash::of(&bytes).to_string(),"expected_broken":expected_broken,"native_error":error_text,"native_parts":native_shapes,"details":details}),
    )
}
pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let mut files = vec![];
    let mut f = seed_nested()?;
    files.push(save(directory,"01-nested-cabinets-input.multicad",&f.session,false,serde_json::json!({"query":f.query,"world_sketch":f.world_sketch,"offset":f.offset,"expected_consumer_edge_mm":[[20.,20.],[100.,20.]]}))?);
    let local = capture(&mut f.session, &f.query, 0, Behavior::ReferenceOnly)?;
    let mut world = vec![];
    for root in &f.roots {
        let mut query = f.query.clone();
        query.reference = EntityId::new();
        query.sketch = f.world_sketch;
        query.source.path.root = root.instance;
        query.sketch_context = None;
        query.method = QueryMethod::Project;
        world.push(capture(&mut f.session, &query, 0, Behavior::ReferenceOnly)?);
    }
    files.push(save(directory,"02-nested-cabinets-referenced.multicad",&f.session,false,serde_json::json!({"local":local,"world":world,"expected_consumer_edge_mm":[[20.,20.],[100.,20.]]}))?);
    parameter(&mut f.session, f.offset, "35")?;
    files.push(save(
        directory,
        "03-cabinet-placement-updated.multicad",
        &f.session,
        false,
        serde_json::json!({"expected_consumer_edge_mm":[[35.,20.],[115.,20.]],"offset":f.offset}),
    )?);
    let mut s = seed_section()?;
    files.push(save(
        directory,
        "04-curved-wall-section-input.multicad",
        &s.session,
        false,
        serde_json::json!({"query":s.query}),
    )?);
    capture_section(&mut s)?;
    files.push(save(directory,"05-curved-wall-section-solid.multicad",&s.session,false,serde_json::json!({"sketch":s.sketch,"output":s.output.as_ref().map(|o|serde_json::json!({"part":o.body,"feature":o.feature})),"expected_volume_mm3":std::f64::consts::PI*100.*5.}))?);
    let mut c = seed_circle()?;
    files.push(save(directory,"06-finite-circle-choices-input.multicad",&c.session,false,serde_json::json!({"query":c.query,"radius":c.radius,"expected_point_choices_mm":[[-10.,0.],[10.,0.]]}))?);
    capture_circle(&mut c)?;
    files.push(save(
        directory,
        "07-point-driven-panel.multicad",
        &c.session,
        false,
        serde_json::json!({"request":c.request,"output":c.output.as_ref().map(|o|serde_json::json!({"part":o.body,"feature":o.feature})),"expected_volume_mm3":1000.}),
    )?);
    parameter(&mut c.session, c.radius, "14")?;
    files.push(save(
        directory,
        "08-source-radius-updated.multicad",
        &c.session,
        false,
        serde_json::json!({"expected_volume_mm3":1400.}),
    )?);
    break_circle(&mut c)?;
    files.push(save(directory,"09-broken-dependent-panel.multicad",&c.session,true,serde_json::json!({"request":c.request,"replacement_part":c.replacement.body,"expected_repair":"Choose replacement circular hardware, intersect with YZ, explicitly select positive-side point."}))?);
    repair_circle(&mut c)?;
    files.push(save(
        directory,
        "10-explicitly-repaired-panel.multicad",
        &c.session,
        false,
        serde_json::json!({"request":c.request,"output":c.output.as_ref().map(|o|serde_json::json!({"part":o.body,"feature":o.feature})),"expected_volume_mm3":3800.}),
    )?);
    let mut s = seed_spline()?;
    files.push(save(
        directory,
        "11-weighted-spline-input.multicad",
        &s.session,
        false,
        serde_json::json!({"query":s.query}),
    )?);
    s.request = Some(capture(&mut s.session, &s.query, 0, Behavior::Derived)?);
    files.push(save(directory,"12-exact-spline-reference.multicad",&s.session,false,serde_json::json!({"request":s.request,"source_sketch":s.source_sketch,"receiver_sketch":s.sketch}))?);
    let manifest = serde_json::json!({"generator":"occurrence_reference_review","projects":files,"semantics":["Complete canonical member UUID paths distinguish repeated nested Parts.","Consumer geometry uses relative source-to-consumer pose once, cancelling shared placement prefixes.","World projection is explicit; world display pose excludes the local sketch plane frame.","One explicitly selected native point/curve per reference; source count/support/branch changes require repair.","Protected native pole count/family changes require dependent removal and recapture.","Strict full native Apply currently requires all driving references resolved; multiple independently broken references need future batch repair."],"verification":{"native_geometry":true,"archive_reopen":true,"physical_native_ui_acceptance":false,"windows_linux_runtime":false}});
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
        .ok_or("Usage: occurrence_reference_review NEW_DIRECTORY")?;
    let manifest = generate(Path::new(&directory))?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
