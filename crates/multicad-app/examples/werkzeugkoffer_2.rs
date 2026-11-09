//! Native plywood and PETG tool case generated from the dimensioned reference.
use multicad_core::{
    assembly_inspection::OccurrencePair,
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    hierarchy,
    materials::{Assignment, TagEdit, prepare_assignment, prepare_library, prepare_tags},
    model::*,
    parameters::BoundExpression,
    project_file,
};
use multicad_model::{Compiler, EvaluationCache, Model};
#[cfg(not(test))]
use std::path::Path;

#[path = "werkzeugkoffer_2/metal.rs"]
mod metal;
#[path = "werkzeugkoffer_2/outputs.rs"]
mod outputs;
#[path = "werkzeugkoffer_2/panels.rs"]
mod panels;
#[path = "werkzeugkoffer_2/printed.rs"]
mod printed;

#[path = "support/model_preview.rs"]
mod model_preview;

pub type Error = Box<dyn std::error::Error>;
pub const NAME: &str = "Toolcase";
const DESCRIPTION: &str = "Birch plywood and PETG tool case with two rail-driven drawers, eight small drawers, and an open rear compartment. Edit the lower drawer divider and runner fits in Project Parameters. Motion contains the closed and extended poses. Project Export contains the plywood cut profiles and print package.";
const LARGE_TRAVEL: f64 = 185.;
const SMALL_TRAVEL: f64 = 94.;

fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}

fn parameter(name: &str, value: &str, description: &str) -> Entity {
    let mut entity = Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    entity.description = description.into();
    entity
}

struct CaseBuilder {
    session: Session,
    cache: EvaluationCache,
    petg: EntityId,
}

impl CaseBuilder {
    fn new() -> Result<Self, Error> {
        let mut session = Session::new(Document::new(NAME)?)?;
        let params = [
            parameter(
                "drawer_disk_cell",
                "130",
                "Width of the lower drawer's cutting-disc compartment",
            ),
            parameter(
                "small_runner_play",
                "0.4",
                "Clearance at a small drawer runner",
            ),
            parameter(
                "dovetail_fit",
                "0.15",
                "Clearance around each removable rack strip",
            ),
        ];
        let prepared = session.prepare(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Set tool-case parameters".into(),
            edits: params.into_iter().map(Mutation::Insert).collect(),
        })?;
        session.commit(prepared)?;
        let petg = Entity::new(
            "PETG",
            EntityData::Material {
                density_kg_m3: Some(1270.),
                color_rgba: [64, 100, 141, 255],
                section_hatch: None,
            },
        );
        let id = petg.id;
        let prepared = prepare_library(&session, session.revision(), actor(), &[petg])?;
        session.commit(prepared)?;
        Ok(Self {
            session,
            cache: EvaluationCache::default(),
            petg: id,
        })
    }

    fn checked(&mut self, prepared: Prepared) -> Result<(), Error> {
        let compiler = Compiler {
            omit_display: true,
            ..Default::default()
        };
        let roots: Vec<_> = prepared
            .document()
            .entities()
            .filter(|entity| {
                matches!(
                    entity.data,
                    EntityData::Feature { .. }
                        | EntityData::Part { .. }
                        | EntityData::Instance { .. }
                        | EntityData::Assembly { .. }
                ) && self.session.document().get(entity.id) != Some(*entity)
            })
            .map(|entity| entity.id)
            .collect();
        for roots in roots.chunks(128) {
            compiler
                .compile_sources_cached(
                    &mut self.cache,
                    prepared.document(),
                    self.session.revision(),
                    roots,
                    None,
                )
                .map_err(|error| format!("{}: {error}", prepared.label()))?;
        }
        self.session.commit(prepared)?;
        Ok(())
    }

    fn bound(&self, expression: &str) -> Result<Dimension, Error> {
        Ok(Dimension::Expression {
            expression: BoundExpression::bind(self.session.document(), expression, "mm", None)?,
        })
    }

    fn primitive(
        &mut self,
        name: &str,
        parameters: PrimitiveParameters,
    ) -> Result<BodyInput, Error> {
        let request = PrimitiveRequest::new(name, parameters);
        let operation =
            prepare_primitive(&self.session, self.session.revision(), actor(), &request)?;
        self.checked(operation.prepared)?;
        Ok(BodyInput {
            body: request.body,
            producer: request.feature,
        })
    }

    fn box_part(
        &mut self,
        name: &str,
        origin: [f64; 3],
        size: [f64; 3],
    ) -> Result<EntityId, Error> {
        Ok(self
            .primitive(
                name,
                PrimitiveParameters::Box {
                    frame: PrimitiveFrame {
                        origin: origin.map(length),
                        ..Default::default()
                    },
                    size: size.map(length),
                },
            )?
            .body)
    }

    fn manufacture(
        &mut self,
        body: EntityId,
        class: &str,
        material: Option<EntityId>,
    ) -> Result<(), Error> {
        if let Some(material) = material {
            let prepared = prepare_assignment(
                &self.session,
                self.session.revision(),
                actor(),
                &[body],
                &Assignment {
                    material: Some(material),
                    stock: None,
                    appearance: None,
                    hatch: None,
                },
            )?;
            self.session.commit(prepared)?;
        }
        let prepared = prepare_tags(
            &self.session,
            self.session.revision(),
            actor(),
            &[body],
            &TagEdit::Add([class.to_owned()].into()),
        )?;
        self.session.commit(prepared)?;
        Ok(())
    }

    fn cut(&mut self, target: BodyInput, tools: Vec<BodyInput>) -> Result<EntityId, Error> {
        let request = BooleanRequest::new(Operation::Cut, target, tools);
        let prepared = prepare_boolean(&self.session, self.session.revision(), actor(), &request)?;
        self.checked(prepared.prepared)?;
        Ok(request.feature)
    }

    fn place(&mut self, part: EntityId) -> Result<EntityId, Error> {
        let name = self
            .session
            .document()
            .get(part)
            .ok_or("Missing part to place")?
            .name
            .clone();
        let mut request = ComponentPlacementRequest::new(part, NumericTransform::default());
        request.name = name;
        let prepared =
            prepare_place_component(&self.session, self.session.revision(), actor(), &request)?;
        self.checked(prepared.prepared)?;
        Ok(request.instance)
    }

    fn joint(
        &mut self,
        part: EntityId,
        parent: Option<EntityId>,
        travel: Option<f64>,
    ) -> Result<EntityId, Error> {
        let mut parameters = JointParameters {
            parent,
            ..Default::default()
        };
        if let Some(maximum) = travel {
            let frame = PrimitiveFrame {
                u: [1., 0., 0.],
                v: [0., 0., 1.],
                ..Default::default()
            };
            parameters.kind = JointKind::Slider;
            parameters.moving = JointAnchor::Frame {
                frame: Box::new(frame.clone()),
            };
            parameters.target = JointAnchor::Frame {
                frame: Box::new(frame),
            };
            parameters.limits.translation[2] = Some(JointLimit {
                min: length(0.),
                max: length(maximum),
            });
        } else {
            parameters.kind = JointKind::Fixed;
        }
        let request = JointRequest::new(part, parameters, vec![]);
        let prepared = prepare_joint(&self.session, self.session.revision(), actor(), &request)?;
        self.checked(prepared.prepared)?;
        Ok(request.driver)
    }

    fn pose(
        &mut self,
        name: &str,
        joints: &[EntityId],
        position: impl Fn(usize) -> f64,
    ) -> Result<EntityId, Error> {
        let values = joints
            .iter()
            .enumerate()
            .map(|(i, &joint)| JointPoseValue {
                joint,
                values: JointValues {
                    translation: [length(0.), length(0.), length(position(i))],
                    ..Default::default()
                },
            })
            .collect();
        let request = NamedPoseRequest::new(name, values);
        let prepared =
            prepare_named_pose(&self.session, self.session.revision(), actor(), &request)?;
        self.checked(prepared.prepared)?;
        Ok(request.pose)
    }

    fn group(&mut self, name: &str, entries: Vec<EntityId>, visible: bool) -> Result<(), Error> {
        self.container(hierarchy::ContainerKind::Group, name, entries, visible)
    }

    fn container(
        &mut self,
        kind: hierarchy::ContainerKind,
        name: &str,
        entries: Vec<EntityId>,
        visible: bool,
    ) -> Result<(), Error> {
        let prepare = |session: &Session, action| {
            hierarchy::prepare(session, session.revision(), actor(), action)
        };
        let created = prepare(
            &self.session,
            hierarchy::Action::Create {
                kind,
                name: name.into(),
                parent: None,
            },
        )?;
        let id = created.created.ok_or("Missing model group")?;
        self.session.commit(created.prepared)?;
        let moved = prepare(
            &self.session,
            hierarchy::Action::Move {
                entries,
                parent: Some(id),
                before: None,
            },
        )?;
        self.session.commit(moved.prepared)?;
        if !visible {
            let hidden = prepare(&self.session, hierarchy::Action::SetVisible { id, visible })?;
            self.session.commit(hidden.prepared)?;
        }
        Ok(())
    }

    fn assemble(
        &mut self,
        name: &str,
        members: &[(EntityId, EntityId)],
    ) -> Result<EntityId, Error> {
        let (instances, parts) = members.iter().copied().unzip();
        let request = AssemblyRequest::new(name, instances, parts);
        let prepared = prepare_assembly(&self.session, self.session.revision(), actor(), &request)?;
        self.checked(prepared.prepared)?;
        self.place(request.assembly)
    }

    fn study(
        &mut self,
        name: &str,
        joint: EntityId,
        drawer: OccurrencePath,
        targets: Vec<OccurrencePath>,
        travel: f64,
        step: f64,
    ) -> Result<EntityId, Error> {
        let id = EntityId::new();
        let study = SavedMotionStudy {
            pairs: targets
                .into_iter()
                .map(|target| OccurrencePair {
                    a: drawer.clone(),
                    b: target,
                })
                .collect(),
            joint,
            coordinate: JointCoordinate::Z,
            start: length(0.),
            end: length(travel),
            step: length(step),
            contact_tolerance_mm: 0.0005,
            required_clearance_mm: 0.,
        }
        .entity(id, name)?;
        let prepared = self.session.prepare(Request {
            expected_revision: self.session.revision(),
            actor: actor(),
            label: "Save drawer travel check".into(),
            edits: vec![Mutation::Insert(study)],
        })?;
        self.session.commit(prepared)?;
        Ok(id)
    }
}

fn auxiliary(case: &mut CaseBuilder) -> Result<Vec<EntityId>, Error> {
    let mut bodies = Vec::new();
    for z in [35.5, 84.5] {
        for x in [8., 335.] {
            let name = format!("Slide spacer {}", bodies.len() + 1);
            let spacer = case.primitive(
                &name,
                PrimitiveParameters::Box {
                    frame: PrimitiveFrame {
                        origin: [x, 24., z - 22.55].map(length),
                        ..Default::default()
                    },
                    size: [2., 200., 45.1].map(length),
                },
            )?;
            let mut holes = Vec::new();
            for y in [30.25, 76.6, 171.5] {
                holes.push(case.primitive(
                    "Spacer clearance hole",
                    PrimitiveParameters::Cylinder {
                        radius: length(2.25),
                        height: length(4.),
                        frame: PrimitiveFrame {
                            origin: [x - 1., y, z].map(length),
                            u: [0., 1., 0.],
                            v: [0., 0., 1.],
                        },
                    },
                )?);
            }
            let id = spacer.body;
            case.cut(spacer, holes)?;
            case.manufacture(id, "printed", Some(case.petg))?;
            bodies.push(id);
        }
    }
    for (i, (x, z)) in [35.5, 84.5]
        .into_iter()
        .flat_map(|z| [(82.5, z), (262.5, z)])
        .enumerate()
    {
        let cylinder = |origin: [f64; 3], r, depth| PrimitiveParameters::Cylinder {
            radius: length(r),
            height: length(depth),
            frame: PrimitiveFrame {
                origin: origin.map(length),
                u: [0., 0., 1.],
                v: [1., 0., 0.],
            },
        };
        let stop = case.primitive(
            &format!("Rear magnet stop {}", i + 1),
            cylinder([x, 224., z], 9., 3.),
        )?;
        let pocket = case.primitive("Rear magnet seat", cylinder([x, 223.9, z], 5.1, 3.1))?;
        let id = stop.body;
        // The stock is only as deep as the magnet. An open recess leaves a ring
        // bonded to the back panel, which closes the back of the seat.
        case.cut(stop, vec![pocket])?;
        case.manufacture(id, "printed", Some(case.petg))?;
        bodies.push(id);
    }
    Ok(bodies)
}

struct Magnets {
    large: [[EntityId; 2]; 2],
    small: [EntityId; 8],
    fixed: Vec<EntityId>,
}

fn magnets(case: &mut CaseBuilder) -> Result<Magnets, Error> {
    let material = Entity::new(
        "Neodymium magnets",
        EntityData::Material {
            density_kg_m3: Some(7500.),
            color_rgba: [151, 154, 158, 255],
            section_hatch: None,
        },
    );
    let material_id = material.id;
    let prepared = prepare_library(&case.session, case.session.revision(), actor(), &[material])?;
    case.session.commit(prepared)?;
    let mut create = |name: String, center: [f64; 3], diameter: f64, depth: f64| {
        let part = case.primitive(
            &name,
            PrimitiveParameters::Cylinder {
                radius: length(diameter / 2.),
                height: length(depth),
                frame: PrimitiveFrame {
                    origin: center.map(length),
                    u: [0., 0., 1.],
                    v: [1., 0., 0.],
                },
            },
        )?;
        case.manufacture(part.body, "purchased", Some(material_id))?;
        Ok::<_, Error>(part.body)
    };
    let mut large = [[EntityId::nil(); 2]; 2];
    let mut small = [EntityId::nil(); 8];
    let mut fixed = Vec::with_capacity(12);
    for (row, z) in [35.5, 84.5].into_iter().enumerate() {
        for (col, x) in [82.5, 262.5].into_iter().enumerate() {
            let seat = format!("{}-{}", row + 1, col + 1);
            let back = printed::BIG_DEPTH;
            large[row][col] = create(
                format!("Ø10 drawer magnet {seat}"),
                [x, back - 3., z],
                10.,
                3.,
            )?;
            fixed.push(create(
                format!("Ø10 case magnet {seat}"),
                [x, back, z],
                10.,
                3.,
            )?);
        }
    }
    for (i, (drawer, rack)) in printed::small_magnet_seats().into_iter().enumerate() {
        small[i] = create(format!("Ø5 drawer magnet {}", i + 1), drawer, 5., 3.1)?;
        fixed.push(create(format!("Ø5 rack magnet {}", i + 1), rack, 5., 3.1)?);
    }
    Ok(Magnets {
        large,
        small,
        fixed,
    })
}

pub struct Scenario {
    pub session: Session,
    pub panels: [EntityId; 7],
    pub large_drawers: [EntityId; 2],
    pub small_drawers: [EntityId; 8],
    pub strips: Vec<EntityId>,
    pub coupons: Vec<EntityId>,
    pub rack: EntityId,
    pub poses: Vec<EntityId>,
    pub export_sets: Vec<EntityId>,
    pub print_plates: Vec<EntityId>,
    pub studies: Vec<EntityId>,
}

fn organize_drawer_sources(session: &mut Session, assemblies: &[EntityId]) -> Result<(), Error> {
    for &assembly in assemblies {
        let request = AssemblyRequest::existing(session.document(), assembly)?;
        let sources = request
            .members
            .iter()
            .map(
                |id| match session.document().get(*id).map(|entity| &entity.data) {
                    Some(EntityData::Instance { component, .. }) => Ok(*component),
                    _ => Err("Drawer component member is missing its placement"),
                },
            )
            .collect::<Result<Vec<_>, _>>()?;
        // Keep a source beside its sole same-name placement. The normal tree
        // can then show one row with the component's children or part history.
        let prepared = hierarchy::prepare(
            session,
            session.revision(),
            actor(),
            hierarchy::Action::Move {
                entries: sources,
                parent: Some(assembly),
                before: None,
            },
        )?;
        session.commit(prepared.prepared)?;
    }
    Ok(())
}

pub fn build() -> Result<Scenario, Error> {
    let mut case = CaseBuilder::new()?;
    let panels = panels::build(&mut case).map_err(|error| format!("Plywood panels: {error}"))?;
    let plywood_export = outputs::configure_plywood(&mut case, panels)?;
    eprintln!("Checked seven plywood panels");
    let printed =
        printed::build(&mut case).map_err(|error| format!("Printed components: {error}"))?;
    eprintln!("Checked printed components");
    let metal = metal::build(&mut case).map_err(|error| format!("Metal components: {error}"))?;
    eprintln!("Checked trim and slide hardware");
    let auxiliary = auxiliary(&mut case).map_err(|error| format!("Spacers and stops: {error}"))?;
    let magnets = magnets(&mut case).map_err(|error| format!("Drawer magnets: {error}"))?;

    let rack = case.place(printed.rack)?;
    let mut joints = Vec::new();
    let mut moving_drawers = Vec::new();
    let mut fixed_rails = Vec::new();
    let mut large_assemblies = Vec::new();
    for i in 0..2 {
        let drawer = case.place(printed.large[i])?;
        moving_drawers.push(drawer);
        let mut members = vec![(drawer, printed.large[i])];
        let [left_part, right_part] = metal.fixed[i];
        let left = case.place(left_part)?;
        let right = case.place(right_part)?;
        members.extend([(left, left_part), (right, right_part)]);
        fixed_rails.push([left, right]);
        joints.push(case.joint(drawer, Some(left), Some(LARGE_TRAVEL))?);
        for part in metal.moving[i]
            .into_iter()
            .chain(metal.moving_hardware[i].iter().copied())
            .chain(magnets.large[i])
        {
            let member = case.place(part)?;
            members.push((member, part));
            case.joint(member, Some(drawer), None)?;
        }
        let assembly = case.assemble(["Lower drawer", "Upper drawer box"][i], &members)?;
        large_assemblies.push((assembly, members));
    }
    let mut small_assemblies = Vec::new();
    for (i, (part, magnet)) in printed.small.into_iter().zip(magnets.small).enumerate() {
        let drawer = case.place(part)?;
        moving_drawers.push(drawer);
        // The fixed rack and the small drawer belong to different component
        // definitions. The slider follows the drawer component's local frame.
        joints.push(case.joint(drawer, None, Some(SMALL_TRAVEL))?);
        let magnet_instance = case.place(magnet)?;
        case.joint(magnet_instance, Some(drawer), None)?;
        let members = [(drawer, part), (magnet_instance, magnet)];
        let assembly = case.assemble(&format!("Small drawer {}", i + 1), &members)?;
        small_assemblies.push((assembly, members));
    }
    let (lower, lower_members) = &large_assemblies[0];
    let (upper_box, upper_box_members) = &large_assemblies[1];
    let mut upper_members = vec![(*upper_box, None), (rack, Some(printed.rack))];
    upper_members.extend(small_assemblies.iter().map(|(id, _)| (*id, None)));
    let upper = {
        let request = AssemblyRequest::new(
            "Upper drawer",
            upper_members.iter().map(|(id, _)| *id).collect(),
            upper_members.iter().filter_map(|(_, part)| *part).collect(),
        );
        let prepared = prepare_assembly(&case.session, case.session.revision(), actor(), &request)?;
        case.checked(prepared.prepared)?;
        case.place(request.assembly)?
    };
    let definitions = large_assemblies
        .iter()
        .map(|(id, _)| *id)
        .chain(small_assemblies.iter().map(|(id, _)| *id))
        .chain([upper])
        .map(
            |placement| match case.session.document().get(placement).unwrap().data {
                EntityData::Instance { component, .. } => component,
                _ => unreachable!("drawer assembly placement"),
            },
        )
        .collect::<Vec<_>>();
    organize_drawer_sources(&mut case.session, &definitions)?;
    let mut poses = vec![case.pose("All drawers closed", &joints, |_| 0.)?];
    for (index, label) in ["Lower drawer extended", "Upper drawer extended"]
        .into_iter()
        .enumerate()
    {
        poses.push(case.pose(
            label,
            &joints,
            |i| if i == index { LARGE_TRAVEL } else { 0. },
        )?);
    }
    poses.push(case.pose("Small drawer fit check", &joints, |i| {
        if i == 2 { SMALL_TRAVEL } else { 0. }
    })?);
    let member = |id| {
        if lower_members.iter().any(|(member, _)| *member == id) {
            OccurrencePath {
                root: *lower,
                members: vec![id],
            }
        } else if id == rack {
            OccurrencePath {
                root: upper,
                members: vec![id],
            }
        } else if upper_box_members.iter().any(|(member, _)| *member == id) {
            OccurrencePath {
                root: upper,
                members: vec![*upper_box, id],
            }
        } else {
            let (parent, _) = small_assemblies
                .iter()
                .find(|(_, members)| members.iter().any(|(member, _)| *member == id))
                .expect("placed small drawer member");
            OccurrencePath {
                root: upper,
                members: vec![*parent, id],
            }
        }
    };
    let fixed = |id| OccurrencePath {
        root: id,
        members: vec![],
    };
    let mut studies = Vec::new();
    for (i, name) in ["Lower drawer clearance", "Upper drawer clearance"]
        .into_iter()
        .enumerate()
    {
        let mut targets: Vec<_> = panels
            .into_iter()
            .chain(metal.trim.iter().copied())
            .map(fixed)
            .collect();
        targets.extend([
            member(moving_drawers[1 - i]),
            member(fixed_rails[i][0]),
            member(fixed_rails[i][1]),
        ]);
        studies.push(case.study(
            name,
            joints[i],
            member(moving_drawers[i]),
            targets,
            LARGE_TRAVEL,
            37.,
        )?);
    }
    studies.push(case.study(
        "Small drawer and dovetail clearance",
        joints[2],
        member(moving_drawers[2]),
        vec![
            member(rack),
            fixed(printed.strips[0]),
            fixed(printed.strips[1]),
        ],
        SMALL_TRAVEL,
        23.5,
    )?);
    case.container(
        hierarchy::ContainerKind::Component,
        "Toolcase wood assembly",
        panels.to_vec(),
        true,
    )?;
    case.group("Aluminium edge trim", metal.trim, true)?;
    case.group("Rack strips", printed.strips.clone(), true)?;
    case.group("Spacers and magnet stops", auxiliary.clone(), true)?;
    case.group("Stationary drawer magnets", magnets.fixed, true)?;
    case.group("Print-only fit coupons", printed.coupons.clone(), false)?;
    let (paperwork, print_plates) = outputs::configure(&mut case, &printed, &auxiliary)?;
    eprintln!("Checking assembled geometry and source references");

    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_cached(
        &mut case.cache,
        case.session.document(),
        case.session.revision(),
        None,
    )?;
    if !model
        .snapshot()
        .references
        .for_model(case.session.document())?
        .all_resolved()
    {
        return Err("The completed tool case has unresolved native references".into());
    }
    eprintln!("Rendering the example card");
    set_preview(&mut case.session, &model, &printed.coupons)?;
    Ok(Scenario {
        session: case.session,
        panels,
        large_drawers: printed.large,
        small_drawers: printed.small,
        strips: printed.strips,
        coupons: printed.coupons,
        rack: printed.rack,
        poses,
        export_sets: vec![plywood_export, paperwork],
        print_plates,
        studies,
    })
}

fn set_preview(session: &mut Session, model: &Model, coupons: &[EntityId]) -> Result<(), Error> {
    let solids = model
        .snapshot()
        .leaf_occurrences
        .iter()
        .filter(|(_, leaf)| !coupons.contains(&leaf.component))
        .map(|(path, leaf)| {
            let solid = model.leaf_solid(path).ok_or("Part has no physical solid")?;
            Ok::<_, Error>((leaf.component, solid))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let image = model_preview::render_preview(session.document(), solids)?;
    let info = project_file::info::FileInfo {
        name: NAME.into(),
        description: DESCRIPTION.into(),
        preview: Some(multicad_ui::preview_capture::preview_from(&image)?),
        capture_on_save: true,
    };
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Set example information".into(),
        edits: project_file::info::edits(session.document(), &info)?,
    })?;
    session.commit(prepared)?;
    Ok(())
}

#[cfg(not(test))]
#[path = "../src/files.rs"]
#[allow(dead_code)]
mod files;

#[cfg(not(test))]
fn save(path: &Path, scenario: &Scenario) -> Result<(), Error> {
    use multicad_ui::{application::DocumentStorage, storage::SaveMode};
    let file = project_file::ProjectFile::new(scenario.session.document().clone())?;
    files::ProjectStore::default()
        .save_project(path, &file, SaveMode::Create)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let mut args = std::env::args_os().skip(1);
    let usage = "Usage: werkzeugkoffer_2 NEW_FILE.tcd [EXPORT_DIRECTORY] | --motion EXISTING_FILE.tcd REPORT.json | --exports EXISTING_FILE.tcd EXPORT_DIRECTORY";
    let path = args.next().ok_or(usage)?;
    if path == "--motion" || path == "--exports" {
        use multicad_ui::application::DocumentStorage;
        let project = args.next().ok_or(usage)?;
        let destination = args.next().ok_or(usage)?;
        let loaded = files::ProjectStore::default()
            .open_project(Path::new(&project))
            .map_err(|error| error.to_string())?;
        let session = Session::new(loaded.file.document)?;
        if path == "--motion" {
            return multicad_model::with_evaluation_cache(|| {
                outputs::verify_motion(&session, Path::new(&destination))
            });
        }
        let sets: Vec<_> = multicad_core::exchange::sets(session.document()).collect();
        let plates: Vec<_> = multicad_core::exchange::print_plates(session.document()).collect();
        return multicad_model::with_evaluation_cache(|| {
            outputs::verify_and_export(&session, &sets, &plates, Path::new(&destination))
        });
    }
    let path = Path::new(&path);
    if !multicad_core::project_path::is_current_project_path(path) || path.exists() {
        return Err("Choose a new .tcd file".into());
    }
    let scenario = build()?;
    save(path, &scenario)?;
    eprintln!("Saved {}", path.display());
    if let Some(directory) = args.next() {
        multicad_model::with_evaluation_cache(|| {
            outputs::verify_and_export(
                &scenario.session,
                &scenario.export_sets,
                &scenario.print_plates,
                Path::new(&directory),
            )
        })?;
    }
    Ok(())
}
