//! Under-bench front vise: Wood jaws and Common metal hardware in one model.
use super::{Error, actor, checked, dim, point, tag};
use multicad_core::{
    commands::Session,
    document::EntityId,
    materials::{Assignment, prepare_assignment},
    model::{
        self, BodyInput, BodyOutput, BooleanRequest, GrainAxis, MachiningDefinition, MachiningFace,
        MachiningKind, Operation, PanelDefinition, PanelEdges, PanelPlane, PanelRequest,
        PanelSource, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
    },
};
fn primitive(
    s: &mut Session,
    name: &str,
    parameters: PrimitiveParameters,
) -> Result<BodyInput, Error> {
    let request = PrimitiveRequest::new(name, parameters);
    let prepared = model::prepare_primitive(s, s.revision(), actor(), &request)?;
    checked(s, prepared.prepared)?;
    Ok(BodyInput {
        body: request.body,
        producer: request.feature,
    })
}
fn block(
    s: &mut Session,
    name: &str,
    size: [&str; 3],
    origin: [&str; 3],
) -> Result<BodyInput, Error> {
    let parameters = PrimitiveParameters::Box {
        size: point(s, size)?,
        frame: PrimitiveFrame {
            origin: point(s, origin)?,
            ..Default::default()
        },
    };
    primitive(s, name, parameters)
}
fn cylinder(
    s: &mut Session,
    name: &str,
    radius: f64,
    height: &str,
    origin: [&str; 3],
    along_y: bool,
) -> Result<BodyInput, Error> {
    let parameters = PrimitiveParameters::Cylinder {
        radius: length(radius),
        height: dim(s, height)?,
        frame: PrimitiveFrame {
            origin: point(s, origin)?,
            u: if along_y { [0., 0., 1.] } else { [1., 0., 0.] },
            v: if along_y { [1., 0., 0.] } else { [0., 1., 0.] },
        },
    };
    primitive(s, name, parameters)
}
fn boolean(
    s: &mut Session,
    name: &str,
    operation: Operation,
    target: BodyInput,
    tools: Vec<BodyInput>,
) -> Result<BodyInput, Error> {
    let body = target.body;
    let mut request = BooleanRequest::new(operation, target, tools);
    request.name = name.into();
    let prepared = model::prepare_boolean(s, s.revision(), actor(), &request)?;
    checked(s, prepared.prepared)?;
    Ok(BodyInput {
        body,
        producer: request.feature,
    })
}
fn assign(s: &mut Session, parts: &[EntityId], material: EntityId) -> Result<(), Error> {
    let prepared = prepare_assignment(
        s,
        s.revision(),
        actor(),
        parts,
        &Assignment {
            material: Some(material),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )?;
    s.commit(prepared)?;
    Ok(())
}
pub fn build(
    s: &mut Session,
    stock: EntityId,
    steel: EntityId,
    beech: EntityId,
) -> Result<Vec<EntityId>, Error> {
    let mut wood = vec![];
    for (name, y) in [
        ("Vise fixed jaw", "#0"),
        ("Vise moving jaw", "=-40 mm - $vise_opening"),
    ] {
        let request = PanelRequest::new(
            name,
            PanelDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                source: PanelSource::Rectangle {
                    stock,
                    plane: PanelPlane::Xz,
                    origin: point(
                        s,
                        [
                            "=$vise_center_x - $vise_jaw_width / 2",
                            y,
                            "=$frame_height - 100 mm",
                        ],
                    )?,
                    length: dim(s, "=$vise_jaw_width")?,
                    width: length(140.),
                },
                grain: GrainAxis::U,
                edges: PanelEdges::default(),
                oversize: None,
            },
        );
        let prepared = model::prepare_panel(s, s.revision(), actor(), &request)?;
        checked(s, prepared.prepared)?;
        for (label, diameter, xs) in [
            (
                "guide rod bores",
                17.,
                vec![
                    "=$vise_jaw_width / 2 - 60 mm",
                    "=$vise_jaw_width / 2 + 60 mm",
                ],
            ),
            ("lead screw bore", 22., vec!["=$vise_jaw_width / 2"]),
        ] {
            let holes = xs
                .into_iter()
                .map(|x| Ok([dim(s, x)?, length(20.)]))
                .collect::<Result<Vec<_>, Error>>()?;
            let prepared = model::prepare_machining(
                s,
                s.revision(),
                actor(),
                &format!("{name} {label}"),
                &MachiningDefinition {
                    kind: MachiningKind::Drill {
                        holes,
                        diameter: length(diameter),
                        depth: None,
                    },
                    face: MachiningFace::Top,
                    targets: vec![request.body],
                    suppressed: false,
                    generated_by: None,
                },
            )?;
            checked(s, prepared.model.prepared)?;
        }
        wood.push(request.body);
    }
    let mut metal = vec![];
    metal.push(
        block(
            s,
            "Vise underside mounting plate",
            ["#150", "#200", "#16"],
            ["=$vise_center_x - 75 mm", "#100", "=$frame_height - 16 mm"],
        )?
        .body,
    );
    for (name, x) in [
        ("Vise left mounting post", "=$vise_center_x - 75 mm"),
        ("Vise right mounting post", "=$vise_center_x + 55 mm"),
    ] {
        metal.push(
            block(
                s,
                name,
                ["#20", "#20", "#44"],
                [x, "#240", "=$frame_height - 60 mm"],
            )?
            .body,
        );
    }
    let body = block(
        s,
        "Vise guide housing",
        ["#150", "#40", "#40"],
        ["=$vise_center_x - 75 mm", "#230", "=$frame_height - 100 mm"],
    )?;
    let mut holes = vec![];
    for (x, r) in [
        ("=$vise_center_x - 60 mm", 8.5),
        ("=$vise_center_x", 11.),
        ("=$vise_center_x + 60 mm", 8.5),
    ] {
        holes.push(cylinder(
            s,
            "Vise housing bore",
            r,
            "#42",
            [x, "#229", "=$frame_height - 80 mm"],
            true,
        )?);
    }
    metal.push(boolean(s, "Bore vise guide housing", Operation::Cut, body, holes)?.body);
    for (name, x) in [
        ("Vise left guide rod", "=$vise_center_x - 60 mm"),
        ("Vise right guide rod", "=$vise_center_x + 60 mm"),
    ] {
        metal.push(
            cylinder(
                s,
                name,
                8.,
                "#500",
                [x, "=-70 mm - $vise_opening", "=$frame_height - 80 mm"],
                true,
            )?
            .body,
        );
    }
    let screw = cylinder(
        s,
        "Vise lead screw (simplified)",
        10.,
        "#540",
        [
            "=$vise_center_x",
            "=-120 mm - $vise_opening",
            "=$frame_height - 80 mm",
        ],
        true,
    )?;
    let hub = cylinder(
        s,
        "Vise handle hub",
        18.,
        "#30",
        [
            "=$vise_center_x",
            "=-125 mm - $vise_opening",
            "=$frame_height - 80 mm",
        ],
        true,
    )?;
    let screw = boolean(
        s,
        "Join screw and handle hub",
        Operation::Join,
        screw,
        vec![hub],
    )?;
    let bore = cylinder(
        s,
        "Vise handle bore",
        5.5,
        "#200",
        [
            "=$vise_center_x",
            "=-110 mm - $vise_opening",
            "=$frame_height - 180 mm",
        ],
        false,
    )?;
    metal.push(boolean(s, "Bore sliding handle", Operation::Cut, screw, vec![bore])?.body);
    metal.push(
        cylinder(
            s,
            "Vise sliding handle",
            5.,
            "#160",
            [
                "=$vise_center_x",
                "=-110 mm - $vise_opening",
                "=$frame_height - 160 mm",
            ],
            false,
        )?
        .body,
    );
    for (name, z) in [
        ("Vise lower handle knob", "=$frame_height - 178 mm"),
        ("Vise upper handle knob", "=$frame_height"),
    ] {
        wood.push(
            cylinder(
                s,
                name,
                10.,
                "#18",
                ["=$vise_center_x", "=-110 mm - $vise_opening", z],
                false,
            )?
            .body,
        );
    }
    assign(s, &metal, steel)?;
    assign(s, &wood[2..], beech)?;
    let mut parts = wood;
    parts.extend(metal);
    tag(s, &parts, "vise")?;
    Ok(parts)
}
