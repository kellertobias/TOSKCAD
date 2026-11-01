//! Deterministic generic source definitions, not manufacturer models.
use super::*;
use crate::{
    commands::{Actor, Channel, CommandError, Request, Session},
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, Point,
        source::Source,
        text::{SketchText, TextAlignment, TextAnchor, TextFont, TextStyle},
    },
    document::{Mutation, Placement},
    model::{self, AssemblyRequest, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest},
    parameters::BoundExpression,
    sketch::Datum,
    symbols::{self, SymbolAuthorRequest, SymbolDefinition, SymbolKind, SymbolPlan},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
struct Builder {
    session: Session,
    base: u32,
    next: u32,
    members: Vec<EntityId>,
    parts: Vec<EntityId>,
    size: [EntityId; 3],
    kind: Kind,
}
impl Builder {
    fn id(&self, role: u32) -> EntityId {
        EntityId::parse(&format!(
            "cad20300-0000-4000-8000-{:012x}",
            self.base + role
        ))
        .expect("fixed equipment identity")
    }
    fn fresh(&mut self) -> EntityId {
        let id = self.id(self.next);
        self.next += 1;
        id
    }
    fn apply(&mut self, edits: Vec<Mutation>) -> Result<(), CommandError> {
        self.session.apply(Request {
            expected_revision: self.session.revision(),
            actor: actor(),
            label: "Author generic equipment".into(),
            edits,
        })?;
        Ok(())
    }
    fn dimension(&self, text: &str) -> Result<Dimension, DocumentError> {
        Ok(Dimension::Expression {
            expression: BoundExpression::bind(
                self.session.document(),
                &format!("={text}"),
                "mm",
                None,
            )
            .map_err(|e| invalid(e.to_string()))?,
        })
    }
    fn new(kind: Kind) -> Result<Self, CommandError> {
        let index = Kind::ALL
            .iter()
            .position(|k| *k == kind)
            .expect("catalog kind") as u32
            + 1;
        let base = index * 10000;
        let name = kind.preset().name;
        let mut wire =
            serde_json::to_value(Document::new(name)?).map_err(|e| invalid(e.to_string()))?;
        wire["id"] = serde_json::to_value(EntityId::parse(&format!(
            "cad20300-0000-4000-8000-{:012x}",
            base + 1
        ))?)
        .expect("identity");
        let document = serde_json::from_value(wire).map_err(|e| invalid(e.to_string()))?;
        let mut this = Self {
            session: Session::new(document)?,
            base,
            next: 100,
            members: vec![],
            parts: vec![],
            size: [EntityId::nil(); 3],
            kind,
        };
        for (index, (name, value)) in ["width", "depth", "height"]
            .into_iter()
            .zip(kind.preset().size_mm)
            .enumerate()
        {
            let mut entity = Entity::new(
                name,
                EntityData::Parameter {
                    expression: value.to_string(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            );
            entity.id = this.id(2 + index as u32);
            this.size[index] = entity.id;
            this.apply(vec![Mutation::Insert(entity)])?;
        }
        Ok(this)
    }
    fn z(&self, fraction: &str) -> String {
        let base = match self.kind.preset().mount {
            MountClass::WallCenter => "-$height / 2",
            MountClass::CeilingTop | MountClass::CounterTop => "-$height",
            _ => "0 mm",
        };
        format!("({base}) + $height * ({fraction})")
    }
    fn front(&self) -> &'static str {
        if matches!(
            self.kind.preset().mount,
            MountClass::WallCenter | MountClass::WallBottom
        ) {
            "-$depth"
        } else {
            "-$depth / 2"
        }
    }
    fn primitive(
        &mut self,
        name: &str,
        parameters: PrimitiveParameters,
    ) -> Result<(), CommandError> {
        let mut request = PrimitiveRequest::new(name, parameters);
        request.feature = self.fresh();
        request.body = self.fresh();
        self.session.commit(
            model::prepare_primitive(&self.session, self.session.revision(), actor(), &request)?
                .prepared,
        )?;
        let mut instance = Entity::new(
            name,
            EntityData::Instance {
                component: request.body,
                placement: Placement::default(),
                placement_source: None,
            },
        );
        instance.id = self.fresh();
        self.members.push(instance.id);
        self.parts.push(request.body);
        self.apply(vec![Mutation::Insert(instance)])
    }
    fn box_at(
        &mut self,
        name: &str,
        size: [&str; 3],
        origin: [&str; 3],
    ) -> Result<(), CommandError> {
        let size = [
            self.dimension(size[0])?,
            self.dimension(size[1])?,
            self.dimension(size[2])?,
        ];
        let origin = [
            self.dimension(origin[0])?,
            self.dimension(origin[1])?,
            self.dimension(origin[2])?,
        ];
        self.primitive(
            name,
            PrimitiveParameters::Box {
                size,
                frame: PrimitiveFrame {
                    origin,
                    ..Default::default()
                },
            },
        )
    }
    fn box_z(
        &mut self,
        name: &str,
        size: [&str; 3],
        xy: [&str; 2],
        z: &str,
    ) -> Result<(), CommandError> {
        let z = self.z(z);
        self.box_at(name, size, [xy[0], xy[1], &z])
    }
    fn cylinder(
        &mut self,
        name: &str,
        radius: &str,
        height: &str,
        xyz: [&str; 3],
        front: bool,
    ) -> Result<(), CommandError> {
        let frame = PrimitiveFrame {
            origin: [
                self.dimension(xyz[0])?,
                self.dimension(xyz[1])?,
                self.dimension(xyz[2])?,
            ],
            u: [1., 0., 0.],
            v: if front { [0., 0., 1.] } else { [0., 1., 0.] },
        };
        self.primitive(
            name,
            PrimitiveParameters::Cylinder {
                radius: self.dimension(radius)?,
                height: self.dimension(height)?,
                frame,
            },
        )
    }
    fn cylinder_z(
        &mut self,
        name: &str,
        radius: &str,
        height: &str,
        xy: [&str; 2],
        z: &str,
    ) -> Result<(), CommandError> {
        let z = self.z(z);
        self.cylinder(name, radius, height, [xy[0], xy[1], &z], false)
    }
    fn basin(&mut self, name: &str) -> Result<(), CommandError> {
        self.box_z(
            &format!("{name} bottom"),
            ["$width", "$depth", "$height * 0.08"],
            ["-$width / 2", "-$depth / 2"],
            "0",
        )?;
        for (x, label) in [("-$width / 2", "left rim"), ("$width * 0.45", "right rim")] {
            self.box_z(
                label,
                ["$width * 0.05", "$depth", "$height * 0.92"],
                [x, "-$depth / 2"],
                "0.08",
            )?;
        }
        for (y, label) in [("-$depth / 2", "front rim"), ("$depth * 0.42", "back rim")] {
            self.box_z(
                label,
                ["$width * 0.9", "$depth * 0.08", "$height * 0.92"],
                ["-$width * 0.45", y],
                "0.08",
            )?;
        }
        Ok(())
    }
    fn geometry(&mut self) -> Result<(), CommandError> {
        let radius = "$width * $depth / ($width + $depth) * 0.45";
        match self.kind {
            Kind::PowerOutletTypeF
            | Kind::DoublePowerOutletTypeF
            | Kind::TriplePowerOutletTypeF
            | Kind::DataOutletRj45
            | Kind::TvAntennaOutlet
            | Kind::Switch => {
                self.box_z(
                    "Wall faceplate",
                    ["$width", "$depth * 0.3", "$height"],
                    ["-$width / 2", "-$depth * 0.3"],
                    "0",
                )?;
                let n = match self.kind {
                    Kind::DoublePowerOutletTypeF => 2,
                    Kind::TriplePowerOutletTypeF => 3,
                    _ => 1,
                };
                for i in 0..n {
                    let x = format!("$width * {}", (i as f64 + 0.5) / n as f64 - 0.5);
                    let z = self.z("0.5");
                    let r = format!("$width * $height / ($width + $height) * {}", 0.6 / n as f64);
                    if self.kind == Kind::Switch {
                        self.box_z(
                            "Switch rocker",
                            ["$width * 0.7", "$depth * 0.7", "$height * 0.7"],
                            ["-$width * 0.35", "-$depth"],
                            "0.15",
                        )?;
                    } else {
                        self.cylinder(
                            "Socket / port face",
                            &r,
                            "$depth * 0.7",
                            [&x, "-$depth * 0.3", &z],
                            true,
                        )?;
                    }
                }
            }
            Kind::CeilingLuminaire | Kind::LampHolderE27 => {
                self.box_z(
                    "Ceiling mounting plate",
                    ["$width", "$depth", "$height * 0.12"],
                    ["-$width / 2", "-$depth / 2"],
                    "0.88",
                )?;
                self.cylinder_z(
                    if self.kind == Kind::LampHolderE27 {
                        "Lamp holder body"
                    } else {
                        "Luminaire diffuser"
                    },
                    radius,
                    "$height * 0.88",
                    ["0 mm", "0 mm"],
                    "0",
                )?;
                if self.kind == Kind::LampHolderE27 {
                    self.cylinder_z(
                        "Holder base ring",
                        radius,
                        "$height * 0.1",
                        ["0 mm", "0 mm"],
                        "0",
                    )?;
                }
            }
            Kind::WallLuminaire => {
                self.box_z(
                    "Luminaire wall plate",
                    ["$width", "$depth * 0.2", "$height"],
                    ["-$width / 2", "-$depth * 0.2"],
                    "0",
                )?;
                self.box_z(
                    "Wall light diffuser",
                    ["$width * 0.8", "$depth * 0.8", "$height * 0.75"],
                    ["-$width * 0.4", "-$depth"],
                    "0.125",
                )?;
            }
            Kind::Radiator => {
                self.box_z(
                    "Radiator panel",
                    ["$width", "$depth * 0.6", "$height"],
                    ["-$width / 2", "-$depth * 0.6"],
                    "0",
                )?;
                for i in 0..12 {
                    let x = format!("$width * {}", -0.46 + i as f64 * 0.08);
                    self.box_z(
                        "Radiator fin",
                        ["$width * 0.04", "$depth * 0.4", "$height * 0.94"],
                        [&x, "-$depth"],
                        "0.03",
                    )?;
                }
            }
            Kind::KitchenSink | Kind::Washbasin | Kind::Bathtub => {
                self.basin(if self.kind == Kind::Bathtub {
                    "Tub"
                } else {
                    "Basin"
                })?;
                if self.kind == Kind::KitchenSink {
                    self.box_z(
                        "Draining board",
                        ["$width * 0.35", "$depth * 0.84", "$height * 0.08"],
                        ["$width * 0.1", "-$depth * 0.42"],
                        "0.92",
                    )?;
                }
            }
            Kind::Toilet => {
                self.box_z(
                    "Toilet pedestal",
                    ["$width * 0.55", "$depth * 0.6", "$height * 0.43"],
                    ["-$width * 0.275", "-$depth * 0.4"],
                    "0",
                )?;
                for (x, name) in [
                    ("-$width / 2", "Seat left"),
                    ("$width * 0.38", "Seat right"),
                ] {
                    self.box_z(
                        name,
                        ["$width * 0.12", "$depth * 0.7", "$height * 0.08"],
                        [x, "-$depth / 2"],
                        "0.43",
                    )?;
                }
                for (y, name) in [
                    ("-$depth / 2", "Seat front"),
                    ("$depth * 0.12", "Seat rear"),
                ] {
                    self.box_z(
                        name,
                        ["$width * 0.76", "$depth * 0.08", "$height * 0.08"],
                        ["-$width * 0.38", y],
                        "0.43",
                    )?;
                }
                self.box_z(
                    "Toilet cistern",
                    ["$width * 0.9", "$depth * 0.3", "$height * 0.49"],
                    ["-$width * 0.45", "$depth * 0.2"],
                    "0.51",
                )?;
            }
            Kind::Shower => {
                self.box_z(
                    "Shower tray",
                    ["$width", "$depth", "$height * 0.025"],
                    ["-$width / 2", "-$depth / 2"],
                    "0",
                )?;
                for x in ["-$width / 2", "$width * 0.475"] {
                    for y in ["-$depth / 2", "$depth * 0.475"] {
                        self.box_z(
                            "Shower enclosure post",
                            ["$width * 0.025", "$depth * 0.025", "$height * 0.975"],
                            [x, y],
                            "0.025",
                        )?;
                    }
                }
                self.box_z(
                    "Shower rear rail",
                    ["$width * 0.95", "$depth * 0.025", "$height * 0.025"],
                    ["-$width * 0.475", "$depth * 0.475"],
                    "0.975",
                )?;
            }
            Kind::Cooktop => {
                self.box_z(
                    "Cooktop slab",
                    ["$width", "$depth", "$height * 0.6"],
                    ["-$width / 2", "-$depth / 2"],
                    "0",
                )?;
                self.burners("0.6", "$height * 0.4")?;
            }
            Kind::CookerOven
            | Kind::WashingMachine
            | Kind::Dryer
            | Kind::Refrigerator
            | Kind::Dishwasher => {
                let body_h = if self.kind == Kind::CookerOven {
                    "$height * 0.96"
                } else {
                    "$height"
                };
                self.box_z(
                    "Appliance case",
                    ["$width", "$depth * 0.98", body_h],
                    ["-$width / 2", "-$depth * 0.48"],
                    "0",
                )?;
                self.box_z(
                    "Appliance control strip",
                    ["$width * 0.96", "$depth * 0.02", "$height * 0.08"],
                    ["-$width * 0.48", "-$depth / 2"],
                    "0.9",
                )?;
                match self.kind {
                    Kind::WashingMachine | Kind::Dryer => {
                        let z = self.z("0.5");
                        self.cylinder(
                            "Circular front door",
                            "$width * $height / ($width + $height) * 0.42",
                            "$depth * 0.02",
                            ["0 mm", "-$depth * 0.48", &z],
                            true,
                        )?;
                    }
                    Kind::Refrigerator => {
                        for (z, h) in [("0.03", "$height * 0.28"), ("0.33", "$height * 0.55")] {
                            self.box_z(
                                "Refrigerator door",
                                ["$width * 0.96", "$depth * 0.02", h],
                                ["-$width * 0.48", "-$depth / 2"],
                                z,
                            )?;
                        }
                    }
                    Kind::CookerOven => {
                        self.box_z(
                            "Oven front door",
                            ["$width * 0.88", "$depth * 0.02", "$height * 0.62"],
                            ["-$width * 0.44", "-$depth / 2"],
                            "0.08",
                        )?;
                        self.burners("0.96", "$height * 0.04")?;
                    }
                    _ => self.box_z(
                        "Dishwasher front door",
                        ["$width * 0.96", "$depth * 0.02", "$height * 0.84"],
                        ["-$width * 0.48", "-$depth / 2"],
                        "0.03",
                    )?,
                }
            }
        }
        Ok(())
    }
    fn burners(&mut self, z: &str, height: &str) -> Result<(), CommandError> {
        for x in ["-$width * 0.25", "$width * 0.25"] {
            for y in ["-$depth * 0.25", "$depth * 0.25"] {
                self.cylinder_z(
                    "Cooktop burner",
                    "$width * $depth / ($width + $depth) * 0.2",
                    height,
                    [x, y],
                    z,
                )?;
            }
        }
        Ok(())
    }
    fn plan(&mut self, name: &str, mark: bool) -> Result<EntityId, CommandError> {
        let (original, mut edits) = Source::create(Datum::Xy, name)?;
        let sketch = self.fresh();
        let plane = self.fresh();
        for edit in &mut edits {
            if let Mutation::Insert(entity) = edit {
                if entity.id == original {
                    entity.id = sketch;
                    if let EntityData::Sketch { plane: p, .. } = &mut entity.data {
                        *p = plane;
                    }
                } else {
                    entity.id = plane;
                }
            }
        }
        self.apply(edits)?;
        let mut source = Source::read(self.session.document(), sketch)?.0;
        let origin = self.fresh();
        source.sketch.points.push(Point {
            id: origin,
            position_mm: [0., 0.],
        });
        source.add_constraint(ConstraintKind::Fix { point: origin });
        source.sketch.constraints.last_mut().expect("fix").id = self.fresh();
        if mark {
            self.mark(&mut source, origin)?;
        } else {
            let front = self.front();
            let back = if front == "-$depth" {
                "0 mm"
            } else {
                "$depth / 2"
            };
            self.rectangle(
                &mut source,
                origin,
                ["-$width / 2", front, "$width / 2", back],
            )?;
            match self.kind {
                Kind::KitchenSink | Kind::Washbasin | Kind::Bathtub => {
                    self.rectangle(
                        &mut source,
                        origin,
                        [
                            "-$width * 0.43",
                            "-$depth * 0.39",
                            if self.kind == Kind::KitchenSink {
                                "$width * 0.05"
                            } else {
                                "$width * 0.43"
                            },
                            "$depth * 0.39",
                        ],
                    )?;
                }
                Kind::Toilet => {
                    self.rectangle(
                        &mut source,
                        origin,
                        [
                            "-$width * 0.45",
                            "$depth * 0.2",
                            "$width * 0.45",
                            "$depth / 2",
                        ],
                    )?;
                    self.rectangle(
                        &mut source,
                        origin,
                        [
                            "-$width * 0.33",
                            "-$depth * 0.4",
                            "$width * 0.33",
                            "$depth * 0.1",
                        ],
                    )?;
                }
                Kind::Shower => {
                    self.line(
                        &mut source,
                        origin,
                        ["-$width / 2", "-$depth / 2"],
                        ["$width / 2", "$depth / 2"],
                    )?;
                    self.line(
                        &mut source,
                        origin,
                        ["-$width / 2", "$depth / 2"],
                        ["$width / 2", "-$depth / 2"],
                    )?;
                }
                Kind::Radiator => {
                    for i in 1..8 {
                        let x = format!("$width * {}", -0.5 + i as f64 / 8.);
                        self.line(&mut source, origin, [&x, "-$depth"], [&x, "0 mm"])?;
                    }
                }
                Kind::CookerOven | Kind::Cooktop => {
                    for x in ["-$width * 0.25", "$width * 0.25"] {
                        for y in ["-$depth * 0.25", "$depth * 0.25"] {
                            self.circle(
                                &mut source,
                                origin,
                                [x, y],
                                "$width * $depth / ($width + $depth) * 0.2",
                            )?;
                        }
                    }
                }
                _ => {}
            }
        }
        let edits = source.replace(self.session.document(), sketch)?;
        self.apply(edits)?;
        Ok(sketch)
    }
    fn point(
        &mut self,
        s: &mut Source,
        origin: EntityId,
        xy: [&str; 2],
    ) -> Result<EntityId, DocumentError> {
        let id = self.fresh();
        let values = self.session.parameters();
        let x = self.dimension(xy[0])?;
        let y = self.dimension(xy[1])?;
        s.sketch.points.push(Point {
            id,
            position_mm: [
                x.value_in(Unit::Millimeter, Some(values))
                    .map_err(invalid)?,
                y.value_in(Unit::Millimeter, Some(values))
                    .map_err(invalid)?,
            ],
        });
        for (axis, dimension) in [(Axis::X, x), (Axis::Y, y)] {
            s.add_constraint(ConstraintKind::Distance {
                a: origin,
                b: id,
                axis,
                dimension,
            });
            s.sketch.constraints.last_mut().expect("coordinate").id = self.fresh();
        }
        Ok(id)
    }
    fn line(
        &mut self,
        s: &mut Source,
        origin: EntityId,
        a: [&str; 2],
        b: [&str; 2],
    ) -> Result<(), DocumentError> {
        let a = self.point(s, origin, a)?;
        let b = self.point(s, origin, b)?;
        s.sketch.curves.push(Curve {
            id: self.fresh(),
            construction: false,
            kind: CurveKind::Line { start: a, end: b },
        });
        Ok(())
    }
    fn rectangle(
        &mut self,
        s: &mut Source,
        origin: EntityId,
        r: [&str; 4],
    ) -> Result<(), DocumentError> {
        let mut points = vec![];
        for xy in [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]] {
            points.push(self.point(s, origin, xy)?);
        }
        for i in 0..4 {
            s.sketch.curves.push(Curve {
                id: self.fresh(),
                construction: false,
                kind: CurveKind::Line {
                    start: points[i],
                    end: points[(i + 1) % 4],
                },
            });
        }
        Ok(())
    }
    fn circle(
        &mut self,
        s: &mut Source,
        origin: EntityId,
        xy: [&str; 2],
        radius: &str,
    ) -> Result<(), DocumentError> {
        let center = self.point(s, origin, xy)?;
        let curve = self.fresh();
        let radius = self.dimension(radius)?;
        s.sketch.curves.push(Curve {
            id: curve,
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: radius
                    .value_in(Unit::Millimeter, Some(self.session.parameters()))
                    .map_err(invalid)?,
            },
        });
        s.add_constraint(ConstraintKind::Radius {
            curve,
            dimension: radius,
        });
        s.sketch.constraints.last_mut().expect("radius").id = self.fresh();
        Ok(())
    }
    fn mark(&mut self, s: &mut Source, origin: EntityId) -> Result<(), DocumentError> {
        let code = match self.kind {
            Kind::PowerOutletTypeF => "F",
            Kind::DoublePowerOutletTypeF => "2F",
            Kind::TriplePowerOutletTypeF => "3F",
            Kind::DataOutletRj45 => "RJ",
            Kind::TvAntennaOutlet => "TV",
            Kind::Switch => "S",
            Kind::CeilingLuminaire | Kind::WallLuminaire => "L",
            Kind::LampHolderE27 => "LH",
            Kind::WashingMachine => "WM",
            Kind::Dryer => "DR",
            Kind::Refrigerator => "FR",
            Kind::Dishwasher => "DW",
            _ => "",
        };
        if self.kind.preset().category == Category::ElectricalLighting {
            if matches!(self.kind, Kind::CeilingLuminaire | Kind::WallLuminaire) {
                self.circle(s, origin, ["0 mm", "0 mm"], "0.5 mm")?;
                self.line(s, origin, ["-0.35 mm", "-0.35 mm"], ["0.35 mm", "0.35 mm"])?;
                self.line(s, origin, ["-0.35 mm", "0.35 mm"], ["0.35 mm", "-0.35 mm"])?;
            } else if matches!(
                self.kind,
                Kind::PowerOutletTypeF
                    | Kind::DoublePowerOutletTypeF
                    | Kind::TriplePowerOutletTypeF
                    | Kind::LampHolderE27
            ) {
                self.circle(s, origin, ["0 mm", "0 mm"], "0.5 mm")?;
            } else {
                self.rectangle(s, origin, ["-0.5 mm", "-0.5 mm", "0.5 mm", "0.5 mm"])?;
            }
        }
        if !code.is_empty() && !matches!(self.kind, Kind::CeilingLuminaire | Kind::WallLuminaire) {
            s.sketch.texts.push(SketchText {
                id: self.fresh(),
                style: TextStyle {
                    text: code.into(),
                    font: TextFont::HackRegular,
                    height_mm: if self.kind.preset().category == Category::ElectricalLighting {
                        0.55
                    } else {
                        1.0
                    },
                    position_mm: [0., 0.],
                    anchor: TextAnchor::Center,
                    rotation_deg: 0.,
                    alignment: TextAlignment::Center,
                },
            });
        }
        Ok(())
    }
    fn finish(mut self) -> Result<SymbolDefinition, CommandError> {
        self.geometry()?;
        let mut root = AssemblyRequest::new(
            self.kind.preset().name,
            self.members.clone(),
            self.parts.clone(),
        );
        root.assembly = self.id(10);
        root.metadata = self.id(11);
        self.session.commit(
            model::prepare_assembly(&self.session, self.session.revision(), actor(), &root)?
                .prepared,
        )?;
        let footprint = self.plan("Physical equipment footprint", false)?;
        let has_mark = self.kind.preset().category == Category::ElectricalLighting
            || matches!(
                self.kind,
                Kind::WashingMachine | Kind::Dryer | Kind::Refrigerator | Kind::Dishwasher
            );
        let mark = if has_mark {
            Some(self.plan("Generic equipment plan mark", true)?)
        } else {
            None
        };
        let preset = self.kind.preset();
        let template = Template {
            component: root.assembly,
            kind: self.kind,
            width_parameter: self.size[0],
            depth_parameter: self.size[1],
            height_parameter: self.size[2],
            footprint,
            mark,
            convention: PlanConvention::ToskCadGenericV1,
            power_connector: preset.power_connector,
            mark_paper_mm: mark.map(|_| 3.5),
            mount: preset.mount,
            supports_built_in: preset.supports_built_in,
            supports_standalone: preset.supports_standalone,
        };
        let metadata = self.id(12);
        self.apply(vec![Mutation::Insert(template.entity(metadata)?)])?;
        let mut author = SymbolAuthorRequest::new(
            preset.name,
            root.assembly,
            SymbolKind::Dynamic,
            self.size.to_vec(),
            Some(SymbolPlan { sketch: footprint }),
        );
        author.definition = self.id(13);
        author.symbol = self.id(14);
        self.session.commit(
            symbols::prepare_author(&self.session, self.session.revision(), actor(), &author)?
                .prepared,
        )?;
        symbols::read_definition(self.session.document(), author.definition).map_err(Into::into)
    }
}
pub fn generic_definition(kind: Kind) -> Result<SymbolDefinition, CommandError> {
    Builder::new(kind)?.finish()
}
