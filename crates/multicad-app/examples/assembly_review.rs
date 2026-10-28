//! Generates editable native drawer, hinge and repeated-subassembly projects.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, Model};
use serde_json::{Value, json};
use std::{
    io::{Cursor, Write},
    path::Path,
};
type Error = Box<dyn std::error::Error>;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn bound(doc: &Document, expression: &str, unit: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(doc, expression, unit, None)?,
    })
}
fn native(s: &Session) -> Result<Model, Error> {
    Ok(Compiler::default().compile(s.document(), s.revision())?)
}
fn commit(s: &mut Session, prepared: AssemblyPrepared) -> Result<(), Error> {
    Compiler::default().compile(prepared.prepared.document(), s.revision())?;
    s.commit(prepared.prepared)?;
    Ok(())
}
struct Fixture {
    session: Session,
    parts: Vec<PrimitiveRequest>,
    members: Vec<ComponentPlacementRequest>,
    assembly: AssemblyRequest,
    roots: Vec<ComponentPlacementRequest>,
    slider: JointRequest,
    hinge: JointRequest,
    poses: Vec<NamedPoseRequest>,
}
fn build() -> Result<Fixture, Error> {
    let mut document = Document::new("Case on rails with hinged display")?;
    for (name, value, unit) in [
        ("drawer_width", 40., "mm"),
        ("base_height", 5., "mm"),
        ("max_travel", 60., "mm"),
        ("hinge_open", 90., "deg"),
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
    let mut parts = vec![];
    for (name, size) in [
        ("Base", [100., 80., 5.]),
        ("Drawer", [40., 30., 10.]),
        ("Hinged display", [30., 4., 25.]),
    ] {
        let mut dims = size.map(length);
        if name == "Drawer" {
            dims[0] = bound(session.document(), "=$drawer_width", "mm")?;
        }
        if name == "Base" {
            dims[2] = bound(session.document(), "=$base_height", "mm")?;
        }
        let part = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: dims,
                frame: Default::default(),
            },
        );
        session
            .commit(prepare_primitive(&session, session.revision(), actor(), &part)?.prepared)?;
        parts.push(part);
    }
    let mut members = vec![];
    for (index, part) in parts.iter().enumerate() {
        let translation = match index {
            0 => [0., 0., 0.],
            1 => [0., 0., 5.],
            _ => [120., 0., 5.],
        };
        let mut member = ComponentPlacementRequest::new(
            part.body,
            NumericTransform {
                translation: translation.map(length),
                ..Default::default()
            },
        );
        member.name = format!(
            "{} member",
            if index == 0 {
                "Base"
            } else if index == 1 {
                "Drawer"
            } else {
                "Display"
            }
        );
        let p = prepare_place_component(&session, session.revision(), actor(), &member)?;
        commit(&mut session, p)?;
        members.push(member);
    }
    let assembly = AssemblyRequest::new(
        "Reusable moving case",
        members.iter().map(|m| m.instance).collect(),
        parts.iter().map(|p| p.body).collect(),
    );
    let local = PrimitiveFrame {
        u: [0., 1., 0.],
        v: [0., 0., 1.],
        ..Default::default()
    };
    let mut target = local.clone();
    target.origin[2] = bound(session.document(), "=$base_height", "mm")?;
    let slider = JointRequest::new(
        members[1].instance,
        JointParameters {
            kind: JointKind::Slider,
            parent: Some(members[0].instance),
            moving: JointAnchor::Frame {
                frame: Box::new(local),
            },
            target: JointAnchor::Frame {
                frame: Box::new(target),
            },
            limits: JointLimits {
                translation: [
                    None,
                    None,
                    Some(JointLimit {
                        min: length(0.),
                        max: bound(session.document(), "=$max_travel", "mm")?,
                    }),
                ],
                rotation: None,
            },
            ..Default::default()
        },
        vec![],
    );
    let hinge = JointRequest::new(
        members[2].instance,
        JointParameters {
            kind: JointKind::Revolute,
            parent: Some(members[0].instance),
            target: JointAnchor::Frame {
                frame: Box::new(PrimitiveFrame {
                    origin: [
                        length(120.),
                        length(0.),
                        bound(session.document(), "=$base_height", "mm")?,
                    ],
                    ..Default::default()
                }),
            },
            limits: JointLimits {
                translation: Default::default(),
                rotation: Some(JointLimit {
                    min: angle(0.),
                    max: bound(session.document(), "=$hinge_open", "deg")?,
                }),
            },
            ..Default::default()
        },
        vec![],
    );
    Ok(Fixture {
        session,
        parts,
        members,
        assembly,
        roots: vec![],
        slider,
        hinge,
        poses: vec![],
    })
}
fn save_new(directory: &Path, name: &str, s: &Session, stage: Value) -> Result<Value, Error> {
    let model = native(s)?;
    if !model.snapshot().references.all_resolved() {
        return Err("Review source has unresolved geometry references".into());
    }
    let receipt = model.assembly_evaluation();
    receipt.validate()?;
    let leaves=model.snapshot().leaf_occurrences.values().map(|leaf|{let solid=model.leaf_solid(&leaf.path).ok_or("Missing exact review leaf")?;Ok(json!({"path":leaf.path,"component":leaf.component,"producer":leaf.producer,"pose":leaf.pose,"bounds_mm":{"min":leaf.bounds.min,"max":leaf.bounds.max},"volume_mm3":solid.volume_mm3()?,"solid_count":solid.solid_count()?}))}).collect::<Result<Vec<_>,Error>>()?;
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    let opened = project_file::read(
        &mut Cursor::new(&bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if opened.project.document != *s.document() {
        return Err("Review file roundtrip changed source".into());
    }
    let restored = Compiler::default().compile(&opened.project.document, 0)?;
    if restored.assembly_evaluation().leaves != receipt.leaves {
        return Err("Reopened leaf identity/pose changed".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join(name))?;
    Ok(
        json!({"file":name,"document":s.document().id(),"source_hash":model.snapshot().stamp.source_hash,"stage":stage,"physical_roots":model.snapshot().occurrences.keys().collect::<Vec<_>>(),"standalone_parts":model.snapshot().bodies.keys().collect::<Vec<_>>(),"leaves":leaves}),
    )
}
fn generate(directory: &Path) -> Result<(), Error> {
    std::fs::create_dir(directory)?;
    let mut f = build()?;
    let mut projects = vec![];
    projects.push(save_new(directory,"01-free-instances-input.multicad",&f.session,json!({"use":"Create relationships on explicit member Instances; original source Parts are also physical in this input.","members":f.members}))?);
    let p = prepare_assembly(&f.session, f.session.revision(), actor(), &f.assembly)?;
    commit(&mut f.session, p)?;
    for (x, name) in [(200., "Right moving case"), (-200., "Left moving case")] {
        let mut root = ComponentPlacementRequest::new(
            f.assembly.assembly,
            NumericTransform {
                translation: [length(x), length(50.), length(0.)],
                ..Default::default()
            },
        );
        root.name = name.into();
        let p = prepare_place_component(&f.session, f.session.revision(), actor(), &root)?;
        commit(&mut f.session, p)?;
        f.roots.push(root);
    }
    projects.push(save_new(directory,"02-reused-assembly-input.multicad",&f.session,json!({"assembly":f.assembly,"roots":f.roots,"use":"Reusable rigid component copies before incoming member relationships"}))?);
    for joint in [&f.slider, &f.hinge] {
        let p = prepare_joint(&f.session, f.session.revision(), actor(), joint)?;
        commit(&mut f.session, p)?;
    }
    for (name, travel, rotation) in [
        ("Closed", "=$max_travel*0", "=$hinge_open*0"),
        ("Half", "=$max_travel/2", "=$hinge_open/2"),
        ("Open", "=$max_travel", "=$hinge_open"),
    ] {
        let pose = NamedPoseRequest::new(
            name,
            vec![
                JointPoseValue {
                    joint: f.slider.driver,
                    values: JointValues {
                        translation: [
                            length(0.),
                            length(0.),
                            bound(f.session.document(), travel, "mm")?,
                        ],
                        ..Default::default()
                    },
                },
                JointPoseValue {
                    joint: f.hinge.driver,
                    values: JointValues {
                        rotation: bound(f.session.document(), rotation, "deg")?,
                        ..Default::default()
                    },
                },
            ],
        );
        let p = prepare_named_pose(&f.session, f.session.revision(), actor(), &pose)?;
        commit(&mut f.session, p)?;
        f.poses.push(pose);
    }
    for (index, pose) in f.poses.iter().enumerate() {
        let p = prepare_apply_pose(&f.session, f.session.revision(), actor(), pose.pose)?;
        commit(&mut f.session, p)?;
        let filename = format!("{:02}-{}.multicad", index + 3, pose.name.to_lowercase());
        let model = native(&f.session)?;
        let path = OccurrencePath {
            root: f.roots[0].instance,
            members: vec![f.slider.owner],
        };
        let expected = 200. + index as f64 * 30.;
        if (model
            .leaf_snapshot(&path)
            .ok_or("Missing drawer leaf")?
            .bounds
            .min[0]
            - expected)
            .abs()
            > 1e-5
        {
            return Err("Named-pose analytic drawer bound failed".into());
        }
        projects.push(save_new(directory,&filename,&f.session,json!({"pose":pose.pose,"name":pose.name,"expected_travel_mm":index*30,"expected_hinge_degrees":index*45,"case_volume_mm3":55000.,"physical_cases":2,"physical_leaves":6}))?);
    }
    let wrapper = AssemblyRequest::new(
        "Two repeated cases",
        f.roots.iter().map(|r| r.instance).collect(),
        f.assembly.definition_parts.clone(),
    );
    let p = prepare_assembly(&f.session, f.session.revision(), actor(), &wrapper)?;
    commit(&mut f.session, p)?;
    let mut placed = ComponentPlacementRequest::new(
        wrapper.assembly,
        NumericTransform {
            translation: [length(1000.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    placed.name = "Nested two-case module".into();
    let p = prepare_place_component(&f.session, f.session.revision(), actor(), &placed)?;
    commit(&mut f.session, p)?;
    let model = native(&f.session)?;
    if model.snapshot().leaf_occurrences.len() != 6
        || model.snapshot().occurrences.len() != 1
        || (model
            .solid(placed.instance)
            .ok_or("Missing nested compound")?
            .volume_mm3()?
            - 110000.)
            .abs()
            > 1e-4
    {
        return Err("Nested review physical counts/volume are inconsistent".into());
    }
    projects.push(save_new(directory,"06-nested-open.multicad",&f.session,json!({"wrapper":wrapper,"root":placed,"expected_total_volume_mm3":110000.,"expected_leaves":6,"use":"Repeated member UUIDs are disambiguated by complete root/member paths"}))?);
    let manifest = json!({"generator":"assembly_review","parts":f.parts,"members":f.members,"assembly":f.assembly,"roots":f.roots,"slider":f.slider,"hinge":f.hinge,"named_poses":f.poses,"parameters":f.session.document().entities().filter(|e|matches!(e.data,EntityData::Parameter{..})).map(|e|json!({"id":e.id,"name":e.name,"source":e.data})).collect::<Vec<_>>(),"assumptions":["All files are editable canonical projects compiled with exact native geometry and reread before saving.","Slider axis is local Z, explicitly oriented to world X by right-handed moving/target frames. Hinge is local Z rotation.","Subassembly copies share their internal joint configuration; their outer placements are independent. Per-copy flexible internal overrides are not implemented.","Definitions and member template Instances are source-only when reusable ownership metadata is present; final physical leaves have complete canonical paths.","No continuous collision or physics claim is made. Native interference and sampled movement queries are separate review actions.","This generator proves native source/geometry consistency; physical UI acceptance and Windows/Linux runtime/performance remain separate."],"projects":projects});
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join("manifest.json"))?;
    println!("{}", directory.display());
    Ok(())
}
fn main() -> Result<(), Error> {
    let dir = std::env::args_os()
        .nth(1)
        .ok_or("Usage: cargo run -p multicad-app --example assembly_review -- NEW_DIRECTORY")?;
    generate(Path::new(&dir))
}
