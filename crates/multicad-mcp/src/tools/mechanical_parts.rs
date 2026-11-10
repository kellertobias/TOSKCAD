//! Native-checked generic hardware geometry; catalog placement remains separate.
use super::{
    NoArgs, Plane, Quantity, Request, ToolResult, commit, compile_without_display, id,
    revision_guard, snapshot,
    threads::{ThreadRecipeArgs, parameters as thread_parameters},
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::Document,
    materials,
    model::{
        self, MechanicalPartParameters, PrimitiveFrame, ScrewDrive, ScrewHead, ThreadParameters,
        ThreadRepresentation, length,
    },
    parameters::EvaluatedParameters,
    sketch::Datum,
    symbols,
};
use multicad_model::GeometryBackend;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MechanicalFrameArgs {
    #[serde(default)]
    pub origin: Option<[Quantity; 3]>,
    #[serde(default)]
    pub plane: Option<Plane>,
    #[serde(default)]
    pub axis_u: Option<[f64; 3]>,
    #[serde(default)]
    pub axis_v: Option<[f64; 3]>,
}
impl MechanicalFrameArgs {
    fn apply(
        &self,
        mut frame: PrimitiveFrame,
        doc: &Document,
        eval: &EvaluatedParameters,
    ) -> Result<PrimitiveFrame, ToolError> {
        if let Some(p) = &self.origin {
            frame.origin = [
                p[0].dimension(doc, eval)?.0,
                p[1].dimension(doc, eval)?.0,
                p[2].dimension(doc, eval)?.0,
            ];
        }
        if self.plane.is_some() && (self.axis_u.is_some() || self.axis_v.is_some()) {
            return Err(ToolError::invalid(
                "Choose a plane or both explicit basis vectors",
            ));
        }
        if let Some(plane) = self.plane {
            let p = Datum::from(plane).frame();
            frame.u = p.u;
            frame.v = p.v;
        }
        match (self.axis_u, self.axis_v) {
            (Some(u), Some(v)) => {
                frame.u = u;
                frame.v = v;
            }
            (None, None) => {}
            _ => {
                return Err(ToolError::invalid(
                    "An explicit basis needs both axis_u and axis_v",
                ));
            }
        }
        frame.validate()?;
        Ok(frame)
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewHeadArgs {
    SocketCap {
        diameter: Option<Quantity>,
        height: Option<Quantity>,
    },
    ExternalHex {
        across_flats: Option<Quantity>,
        height: Option<Quantity>,
    },
    Pan {
        diameter: Option<Quantity>,
        height: Option<Quantity>,
        crown_radius: Option<Quantity>,
        top_land_diameter: Option<Quantity>,
        neck_radius: Option<Quantity>,
    },
    Button {
        diameter: Option<Quantity>,
        height: Option<Quantity>,
        crown_radius: Option<Quantity>,
        top_land_diameter: Option<Quantity>,
        neck_radius: Option<Quantity>,
    },
    Countersunk {
        diameter: Option<Quantity>,
        height: Option<Quantity>,
        included_angle: Option<Quantity>,
        edge_chamfer: Option<Quantity>,
        neck_radius: Option<Quantity>,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewDriveArgs {
    HexSocket {
        across_flats: Option<Quantity>,
        depth: Option<Quantity>,
    },
    ExternalHex,
    Hexalobular {
        major_diameter: Option<Quantity>,
        minor_diameter: Option<Quantity>,
        outer_lobe_radius: Option<Quantity>,
        depth: Option<Quantity>,
        bottom_angle: Option<Quantity>,
    },
    PhillipsH {
        profile: Option<Box<PhillipsHProfileArgs>>,
    },
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhillipsHSizeArgs {
    H0,
    H1,
    H2,
    H3,
    H4,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhillipsInnerArgs {
    Rounded {
        radius: Option<Quantity>,
    },
    ShallowV {
        angle: Option<Quantity>,
        e_width: Option<Quantity>,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhillipsHProfileArgs {
    size: Option<PhillipsHSizeArgs>,
    mouth_diameter: Option<Quantity>,
    mouth_radius: Option<Quantity>,
    b_width: Option<Quantity>,
    g_diameter: Option<Quantity>,
    tilt_angle: Option<Quantity>,
    outer_flank_angle: Option<Quantity>,
    floor_angle: Option<Quantity>,
    outer_corner_angle: Option<Quantity>,
    inner: Option<PhillipsInnerArgs>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MechanicalRecipeArgs {
    WoodScrew {
        length_input: Option<ScrewLengthInputArgs>,
        nominal_diameter: Option<Quantity>,
        head: Option<Box<ScrewHeadArgs>>,
        drive: Option<Box<ScrewDriveArgs>>,
        reference_profile: Option<Box<WoodReferenceArgs>>,
        minimum_tip_inclusive_thread_extent: Option<Quantity>,
        point: Option<WoodPointArgs>,
        representation: Option<WoodRepresentationArgs>,
        #[serde(flatten)]
        frame: MechanicalFrameArgs,
    },
    Washer {
        #[serde(default)]
        bore_diameter: Option<Quantity>,
        #[serde(default)]
        outside_diameter: Option<Quantity>,
        #[serde(default)]
        thickness: Option<Quantity>,
        #[serde(flatten)]
        frame: MechanicalFrameArgs,
    },
    HexNut {
        #[serde(default)]
        across_flats: Option<Quantity>,
        /// Thread length is nut height; its frame locates the whole nut.
        #[serde(default)]
        thread: Box<ThreadRecipeArgs>,
    },
    MachineScrew {
        underhead_length: Option<Quantity>,
        length_input: Option<ScrewLengthInputArgs>,
        shank_diameter: Option<Quantity>,
        head: Option<Box<ScrewHeadArgs>>,
        drive: Option<Box<ScrewDriveArgs>>,
        /// Thread length is the threaded extent; frame is the underhead datum.
        #[serde(default)]
        thread: Box<ThreadRecipeArgs>,
    },
    BearingEnvelope {
        #[serde(default)]
        bore_diameter: Option<Quantity>,
        #[serde(default)]
        outside_diameter: Option<Quantity>,
        #[serde(default)]
        width: Option<Quantity>,
        #[serde(flatten)]
        frame: MechanicalFrameArgs,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WoodReferenceArgs {
    pitch: Option<Quantity>,
    core_diameter: Option<Quantity>,
    included_angle: Option<Quantity>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WoodPointArgs {
    ManufacturerUnspecified,
}
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WoodRepresentationArgs {
    SimplifiedEnvelope,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewLengthInputArgs {
    Underhead { value: Quantity },
    Overall { value: Quantity },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MechanicalPartArgs {
    pub expected_revision: u64,
    /// Existing generator feature ID, not its output Part ID.
    #[serde(default)]
    pub feature: Option<String>,
    /// Required for creation; edits retain the saved name.
    #[serde(default)]
    pub name: Option<String>,
    /// Kind is explicit even on a partial edit. Omitted values retain source.
    pub recipe: MechanicalRecipeArgs,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectMechanicalPartArgs {
    pub feature: String,
}
fn patched_dimension(
    quantity: &Option<Quantity>,
    old: Option<&multicad_core::constraint_sketch::Dimension>,
    field: &str,
    expected: multicad_core::parameters::Unit,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<multicad_core::constraint_sketch::Dimension, ToolError> {
    use multicad_core::{
        constraint_sketch::Dimension,
        parameters::{BoundExpression, Unit},
    };
    let explicit = quantity
        .as_ref()
        .map(|q| {
            let unit = Unit::parse(&q.unit)
                .filter(|u| u.dimension() == expected.dimension())
                .ok_or_else(|| {
                    ToolError::invalid(format!("{field} requires {} units", expected.symbol()))
                })?;
            match (q.value, &q.expression) {
                (Some(value), None) if value.is_finite() => Ok(Dimension::Literal {
                    value,
                    unit: q.unit.clone(),
                }),
                (None, Some(text)) => {
                    let expression = BoundExpression::bind(document, text, unit.symbol(), None)?;
                    expression
                        .evaluate(evaluated)?
                        .in_unit(expected)
                        .ok_or_else(|| {
                            ToolError::invalid(format!("{field} has incompatible expression units"))
                        })?;
                    Ok(Dimension::Expression { expression })
                }
                _ => Err(ToolError::invalid(format!(
                    "{field}: give exactly one finite value or expression"
                ))),
            }
        })
        .transpose()?;
    explicit.or_else(|| old.cloned()).ok_or_else(|| {
        ToolError::invalid(format!(
            "Changing head/drive kind requires explicit {field}"
        ))
    })
}
fn phillips_profile_patch(
    args: &PhillipsHProfileArgs,
    old: Option<&model::PhillipsHParameters>,
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<model::PhillipsHParameters, ToolError> {
    let field = |q, previous, label| {
        patched_dimension(
            q,
            previous,
            label,
            multicad_core::parameters::Unit::Millimeter,
            document,
            evaluated,
        )
    };
    let angle = |q, previous, label| {
        patched_dimension(
            q,
            previous,
            label,
            multicad_core::parameters::Unit::Degree,
            document,
            evaluated,
        )
    };
    let size = args
        .size
        .map(|size| match size {
            PhillipsHSizeArgs::H0 => model::PhillipsHSize::H0,
            PhillipsHSizeArgs::H1 => model::PhillipsHSize::H1,
            PhillipsHSizeArgs::H2 => model::PhillipsHSize::H2,
            PhillipsHSizeArgs::H3 => model::PhillipsHSize::H3,
            PhillipsHSizeArgs::H4 => model::PhillipsHSize::H4,
        })
        .or_else(|| old.map(|p| p.size))
        .ok_or_else(|| ToolError::invalid("A new Phillips H drive requires its size"))?;
    let inner = match &args.inner {
        None => old
            .map(|p| p.inner.clone())
            .ok_or_else(|| ToolError::invalid("A new Phillips H drive requires its inner form"))?,
        Some(PhillipsInnerArgs::Rounded { radius }) => model::PhillipsInnerForm::Rounded {
            radius: field(
                radius,
                old.and_then(|p| {
                    if let model::PhillipsInnerForm::Rounded { radius } = &p.inner {
                        Some(radius)
                    } else {
                        None
                    }
                }),
                "Phillips inner radius",
            )?,
        },
        Some(PhillipsInnerArgs::ShallowV { angle, e_width }) => {
            let previous = old.and_then(|p| {
                if let model::PhillipsInnerForm::ShallowV { angle, e_width } = &p.inner {
                    Some((angle, e_width))
                } else {
                    None
                }
            });
            model::PhillipsInnerForm::ShallowV {
                angle: patched_dimension(
                    angle,
                    previous.map(|v| v.0),
                    "Phillips inner angle",
                    multicad_core::parameters::Unit::Degree,
                    document,
                    evaluated,
                )?,
                e_width: field(e_width, previous.map(|v| v.1), "Phillips e width")?,
            }
        }
    };
    Ok(model::PhillipsHParameters {
        size,
        inner,
        mouth_diameter: field(
            &args.mouth_diameter,
            old.map(|p| &p.mouth_diameter),
            "Phillips mouth diameter",
        )?,
        mouth_radius: field(
            &args.mouth_radius,
            old.map(|p| &p.mouth_radius),
            "Phillips mouth radius",
        )?,
        b_width: field(&args.b_width, old.map(|p| &p.b_width), "Phillips b width")?,
        g_diameter: field(
            &args.g_diameter,
            old.map(|p| &p.g_diameter),
            "Phillips g diameter",
        )?,
        tilt_angle: angle(
            &args.tilt_angle,
            old.map(|p| &p.tilt_angle),
            "Phillips section tilt",
        )?,
        outer_flank_angle: angle(
            &args.outer_flank_angle,
            old.map(|p| &p.outer_flank_angle),
            "Phillips outer flank angle",
        )?,
        floor_angle: angle(
            &args.floor_angle,
            old.map(|p| &p.floor_angle),
            "Phillips floor angle",
        )?,
        outer_corner_angle: angle(
            &args.outer_corner_angle,
            old.map(|p| &p.outer_corner_angle),
            "Phillips outer corner angle",
        )?,
    })
}
fn parameters(
    args: &MechanicalRecipeArgs,
    previous: Option<&MechanicalPartParameters>,
    doc: &Document,
    eval: &EvaluatedParameters,
) -> Result<MechanicalPartParameters, ToolError> {
    let mm = |q: &Option<Quantity>, old: &multicad_core::constraint_sketch::Dimension| {
        q.as_ref()
            .map(|v| v.dimension(doc, eval).map(|d| d.0))
            .transpose()
            .map(|v| v.unwrap_or_else(|| old.clone()))
    };
    let mismatch = || ToolError::invalid("A mechanical Part edit retains its generator family");
    let p = match args {
        MechanicalRecipeArgs::WoodScrew {
            length_input,
            nominal_diameter,
            head,
            drive,
            reference_profile,
            minimum_tip_inclusive_thread_extent,
            point: _,
            representation: _,
            frame,
        } => {
            let default = model::default_wood_screw();
            let MechanicalPartParameters::WoodScrew { parameters: old } =
                previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            let mut wood = old.as_ref().clone();
            if let Some(value) = length_input {
                wood.length_input = match value {
                    ScrewLengthInputArgs::Underhead { value } => {
                        model::ScrewLengthInput::Underhead {
                            value: value.dimension(doc, eval)?.0,
                        }
                    }
                    ScrewLengthInputArgs::Overall { value } => model::ScrewLengthInput::Overall {
                        value: value.dimension(doc, eval)?.0,
                    },
                };
            }
            wood.nominal_diameter = mm(nominal_diameter, &wood.nominal_diameter)?;
            wood.minimum_tip_inclusive_thread_extent = mm(
                minimum_tip_inclusive_thread_extent,
                &wood.minimum_tip_inclusive_thread_extent,
            )?;
            if let Some(profile) = reference_profile {
                wood.reference_profile.pitch = mm(&profile.pitch, &wood.reference_profile.pitch)?;
                wood.reference_profile.core_diameter = mm(
                    &profile.core_diameter,
                    &wood.reference_profile.core_diameter,
                )?;
                wood.reference_profile.included_angle = patched_dimension(
                    &profile.included_angle,
                    Some(&wood.reference_profile.included_angle),
                    "Wood reference included_angle",
                    multicad_core::parameters::Unit::Degree,
                    doc,
                    eval,
                )?;
            }
            if let Some(head) = head {
                let ScrewHeadArgs::ExternalHex {
                    across_flats,
                    height,
                } = head.as_ref()
                else {
                    return Err(ToolError::new(
                        "unsupported",
                        "Wood screw envelopes support only an external_hex head",
                    ));
                };
                let ScrewHead::ExternalHex {
                    across_flats: a,
                    height: h,
                } = &wood.head
                else {
                    return Err(ToolError::new(
                        "unsupported",
                        "Unsupported retained wood head",
                    ));
                };
                wood.head = ScrewHead::ExternalHex {
                    across_flats: mm(across_flats, a)?,
                    height: mm(height, h)?,
                };
            }
            if drive
                .as_ref()
                .is_some_and(|drive| !matches!(drive.as_ref(), ScrewDriveArgs::ExternalHex))
            {
                return Err(ToolError::new(
                    "unsupported",
                    "Wood screw envelopes support only an external_hex drive",
                ));
            }
            wood.frame = frame.apply(wood.frame, doc, eval)?;
            MechanicalPartParameters::WoodScrew {
                parameters: Box::new(wood),
            }
        }
        MechanicalRecipeArgs::Washer {
            bore_diameter,
            outside_diameter,
            thickness,
            frame,
        } => {
            let default = MechanicalPartParameters::Washer {
                bore_diameter: length(3.2),
                outside_diameter: length(7.),
                thickness: length(0.5),
                frame: Default::default(),
            };
            let MechanicalPartParameters::Washer {
                bore_diameter: b,
                outside_diameter: o,
                thickness: h,
                frame: f,
            } = previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            MechanicalPartParameters::Washer {
                bore_diameter: mm(bore_diameter, b)?,
                outside_diameter: mm(outside_diameter, o)?,
                thickness: mm(thickness, h)?,
                frame: frame.apply(f.clone(), doc, eval)?,
            }
        }
        MechanicalRecipeArgs::BearingEnvelope {
            bore_diameter,
            outside_diameter,
            width,
            frame,
        } => {
            let default = MechanicalPartParameters::BearingEnvelope {
                bore_diameter: length(10.),
                outside_diameter: length(30.),
                width: length(9.),
                frame: Default::default(),
            };
            let MechanicalPartParameters::BearingEnvelope {
                bore_diameter: b,
                outside_diameter: o,
                width: h,
                frame: f,
            } = previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            MechanicalPartParameters::BearingEnvelope {
                bore_diameter: mm(bore_diameter, b)?,
                outside_diameter: mm(outside_diameter, o)?,
                width: mm(width, h)?,
                frame: frame.apply(f.clone(), doc, eval)?,
            }
        }
        MechanicalRecipeArgs::HexNut {
            across_flats,
            thread,
        } => {
            let default = MechanicalPartParameters::HexNut {
                across_flats: length(10.),
                thread: Box::new(ThreadParameters {
                    major_diameter: length(6.),
                    pitch: length(1.),
                    length: length(5.2),
                    representation: ThreadRepresentation::CosmeticEnvelope,
                    ..Default::default()
                }),
            };
            let MechanicalPartParameters::HexNut {
                across_flats: a,
                thread: t,
            } = previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            MechanicalPartParameters::HexNut {
                across_flats: mm(across_flats, a)?,
                thread: Box::new(thread_parameters(thread, Some(t), doc, eval)?),
            }
        }
        MechanicalRecipeArgs::MachineScrew {
            underhead_length,
            length_input,
            shank_diameter,
            head,
            drive,
            thread,
        } => {
            let default = model::default_machine_screw();
            let MechanicalPartParameters::MachineScrew {
                underhead_length: l,
                length_input: old_length_input,
                shank_diameter: d,
                head: old_head,
                drive: old_drive,
                thread: t,
            } = previous.unwrap_or(&default)
            else {
                return Err(mismatch());
            };
            if underhead_length.is_some() && length_input.is_some() {
                return Err(ToolError::invalid(
                    "Specify only one of underhead_length and length_input",
                ));
            }
            let (legacy_length, typed_length) = if let Some(input) = length_input {
                let (overall, value) = match input {
                    ScrewLengthInputArgs::Underhead { value } => (false, value),
                    ScrewLengthInputArgs::Overall { value } => (true, value),
                };
                let value = value.dimension(doc, eval)?.0;
                (
                    None,
                    Some(if overall {
                        model::ScrewLengthInput::Overall { value }
                    } else {
                        model::ScrewLengthInput::Underhead { value }
                    }),
                )
            } else if let Some(value) = underhead_length {
                (Some(value.dimension(doc, eval)?.0), None)
            } else {
                (l.clone(), old_length_input.clone())
            };
            let required = |q: &Option<Quantity>,
                            old: Option<&multicad_core::constraint_sketch::Dimension>,
                            field: &str| {
                q.as_ref()
                    .map(|q| q.dimension(doc, eval).map(|d| d.0))
                    .transpose()?
                    .or_else(|| old.cloned())
                    .ok_or_else(|| {
                        ToolError::invalid(format!(
                            "Changing head/drive kind requires explicit {field}"
                        ))
                    })
            };
            let head = match head.as_deref() {
                None => old_head.clone(),
                Some(ScrewHeadArgs::SocketCap { diameter, height }) => {
                    let old = if let ScrewHead::SocketCap { diameter, height } = old_head {
                        Some((diameter, height))
                    } else {
                        None
                    };
                    ScrewHead::SocketCap {
                        diameter: required(diameter, old.map(|v| v.0), "head diameter")?,
                        height: required(height, old.map(|v| v.1), "head height")?,
                    }
                }
                Some(ScrewHeadArgs::ExternalHex {
                    across_flats,
                    height,
                }) => {
                    let old = if let ScrewHead::ExternalHex {
                        across_flats,
                        height,
                    } = old_head
                    {
                        Some((across_flats, height))
                    } else {
                        None
                    };
                    ScrewHead::ExternalHex {
                        across_flats: required(
                            across_flats,
                            old.map(|v| v.0),
                            "head across_flats",
                        )?,
                        height: required(height, old.map(|v| v.1), "head height")?,
                    }
                }
                Some(ScrewHeadArgs::Pan {
                    diameter,
                    height,
                    crown_radius,
                    top_land_diameter,
                    neck_radius,
                }) => {
                    let old = if let ScrewHead::Pan { profile } = old_head {
                        Some([
                            &profile.diameter,
                            &profile.height,
                            &profile.crown_radius,
                            &profile.top_land_diameter,
                            &profile.neck_radius,
                        ])
                    } else {
                        None
                    };
                    ScrewHead::Pan {
                        profile: Box::new(model::CrownedScrewHeadParameters {
                            diameter: required(diameter, old.map(|v| v[0]), "head diameter")?,
                            height: required(height, old.map(|v| v[1]), "head height")?,
                            crown_radius: required(
                                crown_radius,
                                old.map(|v| v[2]),
                                "head crown_radius",
                            )?,
                            top_land_diameter: required(
                                top_land_diameter,
                                old.map(|v| v[3]),
                                "head top_land_diameter",
                            )?,
                            neck_radius: required(
                                neck_radius,
                                old.map(|v| v[4]),
                                "head neck_radius",
                            )?,
                        }),
                    }
                }
                Some(ScrewHeadArgs::Button {
                    diameter,
                    height,
                    crown_radius,
                    top_land_diameter,
                    neck_radius,
                }) => {
                    let old = if let ScrewHead::Button { profile } = old_head {
                        Some([
                            &profile.diameter,
                            &profile.height,
                            &profile.crown_radius,
                            &profile.top_land_diameter,
                            &profile.neck_radius,
                        ])
                    } else {
                        None
                    };
                    ScrewHead::Button {
                        profile: Box::new(model::CrownedScrewHeadParameters {
                            diameter: required(diameter, old.map(|v| v[0]), "head diameter")?,
                            height: required(height, old.map(|v| v[1]), "head height")?,
                            crown_radius: required(
                                crown_radius,
                                old.map(|v| v[2]),
                                "head crown_radius",
                            )?,
                            top_land_diameter: required(
                                top_land_diameter,
                                old.map(|v| v[3]),
                                "head top_land_diameter",
                            )?,
                            neck_radius: required(
                                neck_radius,
                                old.map(|v| v[4]),
                                "head neck_radius",
                            )?,
                        }),
                    }
                }
                Some(ScrewHeadArgs::Countersunk {
                    diameter,
                    height,
                    included_angle,
                    edge_chamfer,
                    neck_radius,
                }) => {
                    let old = if let ScrewHead::Countersunk { profile } = old_head {
                        Some([
                            &profile.diameter,
                            &profile.height,
                            &profile.included_angle,
                            &profile.edge_chamfer,
                            &profile.neck_radius,
                        ])
                    } else {
                        None
                    };
                    ScrewHead::Countersunk {
                        profile: Box::new(model::CountersunkScrewHeadParameters {
                            diameter: required(diameter, old.map(|v| v[0]), "head diameter")?,
                            height: required(height, old.map(|v| v[1]), "head height")?,
                            included_angle: patched_dimension(
                                included_angle,
                                old.map(|v| v[2]),
                                "head included_angle",
                                multicad_core::parameters::Unit::Degree,
                                doc,
                                eval,
                            )?,
                            edge_chamfer: required(
                                edge_chamfer,
                                old.map(|v| v[3]),
                                "head edge_chamfer",
                            )?,
                            neck_radius: required(
                                neck_radius,
                                old.map(|v| v[4]),
                                "head neck_radius",
                            )?,
                        }),
                    }
                }
            };
            let drive = match drive.as_deref() {
                None => old_drive.clone(),
                Some(ScrewDriveArgs::ExternalHex) => ScrewDrive::ExternalHex,
                Some(ScrewDriveArgs::HexSocket {
                    across_flats,
                    depth,
                }) => {
                    let old = if let ScrewDrive::HexSocket {
                        across_flats,
                        depth,
                    } = old_drive
                    {
                        Some((across_flats, depth))
                    } else {
                        None
                    };
                    ScrewDrive::HexSocket {
                        across_flats: required(
                            across_flats,
                            old.map(|v| v.0),
                            "socket across_flats",
                        )?,
                        depth: required(depth, old.map(|v| v.1), "socket depth")?,
                    }
                }
                Some(ScrewDriveArgs::Hexalobular {
                    major_diameter,
                    minor_diameter,
                    outer_lobe_radius,
                    depth,
                    bottom_angle,
                }) => {
                    let old = if let ScrewDrive::Hexalobular { profile } = old_drive {
                        Some([
                            &profile.major_diameter,
                            &profile.minor_diameter,
                            &profile.outer_lobe_radius,
                            &profile.depth,
                            &profile.bottom_angle,
                        ])
                    } else {
                        None
                    };
                    ScrewDrive::Hexalobular {
                        profile: Box::new(model::HexalobularDriveParameters {
                            major_diameter: required(
                                major_diameter,
                                old.map(|v| v[0]),
                                "hexalobular major_diameter",
                            )?,
                            minor_diameter: required(
                                minor_diameter,
                                old.map(|v| v[1]),
                                "hexalobular minor_diameter",
                            )?,
                            outer_lobe_radius: required(
                                outer_lobe_radius,
                                old.map(|v| v[2]),
                                "hexalobular outer_lobe_radius",
                            )?,
                            depth: required(depth, old.map(|v| v[3]), "hexalobular depth")?,
                            bottom_angle: patched_dimension(
                                bottom_angle,
                                old.map(|v| v[4]),
                                "hexalobular bottom_angle",
                                multicad_core::parameters::Unit::Degree,
                                doc,
                                eval,
                            )?,
                        }),
                    }
                }
                Some(ScrewDriveArgs::PhillipsH { profile }) => {
                    let old = if let ScrewDrive::PhillipsH { profile } = old_drive {
                        Some(profile.as_ref())
                    } else {
                        None
                    };
                    let profile = match profile {
                        Some(args) => phillips_profile_patch(args, old, doc, eval)?,
                        None => old.cloned().ok_or_else(|| {
                            ToolError::invalid(
                                "A new Phillips H drive requires an explicit profile",
                            )
                        })?,
                    };
                    ScrewDrive::PhillipsH {
                        profile: Box::new(profile),
                    }
                }
            };
            MechanicalPartParameters::MachineScrew {
                underhead_length: legacy_length,
                length_input: typed_length,
                shank_diameter: mm(shank_diameter, d)?,
                head,
                drive,
                thread: Box::new(thread_parameters(thread, Some(t), doc, eval)?),
            }
        }
    };
    p.resolve(eval)?;
    Ok(p)
}
pub(super) fn mechanical_part_profiles(_: &dyn Host, call: &Request, _: NoArgs) -> ToolResult {
    if call.backend == GeometryBackend::Rust {
        return Ok(json!({
            "supported_geometry": ["machine_screw", "hex_nut", "washer", "bearing_envelope"],
            "head_forms": ["socket_cap", "external_hex"],
            "drive_forms": ["hex_socket", "external_hex"],
            "active_geometry_support": {
                "head_drive_pairs": [["socket_cap", "hex_socket"], ["external_hex", "external_hex"]],
                "thread_families": ["iso_metric", "custom"],
                "metric_fits": ["nominal_basic", "metric_6g_external"],
                "hex_nut_metric_fits": ["nominal_basic", "metric_6h_internal"],
                "representations": ["modeled", "cosmetic_envelope"],
                "cosmetic_head_drive_pairs": [["socket_cap", "hex_socket"], ["external_hex", "external_hex"], ["countersunk", "hex_socket"]],
                "cosmetic_shank": "The shank and resolved thread envelope diameters must match. The envelope is for display and placement.",
                "plain_shank": "If present, its diameter must be at least the modeled thread crest diameter.",
                "end_treatments": false,
                "radial_taper": false,
                "mesh": true,
                "threaded_step": true,
                "threaded_step_scope": "Modeled straight single-start screws within the bounded spline budget. Fully threaded external-hex and hex-socket cap forms, plus socket caps with a plain shank at the exact thread crest diameter",
                "step_geometry": ["washer", "bearing_envelope", "machine_screw"]
            },
            "machine_screw": "The underhead length runs from the head seat to the tip. Overall length adds the head height. The thread length is its extent ending at the tip, and its frame locates the underhead datum.",
            "nut_thread": "Rust supports modeled hex nuts and cosmetic cylindrical thread envelopes. The thread length sets the nut height.",
            "bearing": "External mounting envelope only; races, rolling elements and seals are not modeled"
        }));
    }
    Ok(json!({
        "supported_geometry": ["washer", "hex_nut", "bearing_envelope", "machine_screw", "wood_screw"],
        "head_forms": ["socket_cap", "external_hex", "pan", "button", "countersunk"],
        "drive_forms": ["hex_socket", "external_hex", "hexalobular", "phillips_h"],
        "catalog_status": "Library supplies sourced dimensional references. Generic configurations and configured deviations do not establish supplier stock or fit certification.",
        "nut_thread": "Generic nut creation defaults to a cosmetic envelope. Library sources retain their own thread representation input. Select modeled for manufacturing export.",
        "machine_screw": "underhead_length retains its legacy neck-to-tip meaning. Alternatively length_input selects underhead or overall with one typed value; overall includes the physical head-top height. Supplying either length replaces the other source; supplying both refuses; omission preserves the exact source expression and convention. thread.length is the threaded extent ending at the tip. thread.frame is the neck/construction datum; it is not a flat bearing seat for countersunk heads. Omitted head/drive inputs retain their source; kind changes need explicit dimensions. Cosmetic manufacturing export is refused.",
        "wood_screw": "Distinct wood source, never a metric M thread: external_hex head and drive only, simplified_envelope representation, manufacturer_unspecified point. length_input retains underhead or overall authorship. reference_profile pitch/core_diameter/included_angle are reference facts, not recovered helical geometry; minimum_tip_inclusive_thread_extent includes the unknown point and transition and is not an actual thread start. Omitted fields preserve exact source. No modeled wood option or machine-thread controls. STEP, mesh, manufacturing-outline and Print exports refuse the envelope, including placed instances and downstream history.",
        "recess_profiles": "Hexalobular uses twelve tangent circular arcs, an explicit functional depth and a separately selected conical bottom. Phillips H retains actual inclined-section dimensions, the H0 rounded or H1-H4 shallow-V inner form, and the mouth roundover. Product gauge penetration is distinct from removed depth.",
        "bearing": "External mounting envelope only; races, rolling elements and seals are not modeled",
        "limits": "Engineering profile targets and nominal-basic threads do not certify gauge acceptance, physical fit, material, strength or current-standard compliance. Inspect retained source facts and configured status separately."
    }))
}
fn frame_value(frame: multicad_core::sketch::Frame) -> Value {
    json!({"origin_mm":frame.origin,"axis_u":frame.u,"axis_v":frame.v,"axis_n":frame.n})
}
fn connection_thread_value(thread: &model::ResolvedThread) -> Value {
    json!({"nominal_diameter_mm":2.*thread.nominal_major_radius_mm,
        "major_radius_mm":thread.major_radius_mm,"pitch_mm":thread.pitch_mm,
        "lead_mm":thread.lead_mm,"length_mm":thread.length_mm,"hand":thread.hand,
        "starts":thread.starts,"representation":thread.representation,
        "depth_mm":thread.profile.depth_mm,"crest_width_mm":thread.profile.crest_width_mm,
        "lower_flank_deg":thread.profile.lower_flank_deg,"upper_flank_deg":thread.profile.upper_flank_deg,
        "fit":thread.fit,"metric_fit_engagement":thread.metric_fit_engagement})
}
fn custom_connection_value(parameters: &model::ResolvedCustomConnectionHardware) -> Value {
    let mut value = match parameters {
        model::ResolvedCustomConnectionHardware::WoodScrew {
            wood_thread,
            plain_length_mm,
            plain_radius_mm,
            head,
            drive,
            underhead_frame,
            wood_thread_start,
            wood_complete_interval_mm,
            ..
        } => {
            json!({"connection_kind":"wood_screw","wood_thread":connection_thread_value(wood_thread),
                "plain_length_mm":plain_length_mm,"plain_diameter_mm":2.*plain_radius_mm,
                "head_height_mm":head.height_mm(),"drive_entry_radius_mm":drive.entry_radius_mm(),
                "drive_removed_depth_mm":drive.removed_depth_mm(),"underhead_frame":underhead_frame,
                "wood_thread_start":wood_thread_start,"wood_complete_interval_mm":wood_complete_interval_mm})
        }
        model::ResolvedCustomConnectionHardware::WoodInsert {
            outer_thread,
            inner_thread,
            outer_thread_start,
            inner_thread_start,
            outer_complete_interval_mm,
            inner_complete_interval_mm,
            ..
        } => {
            json!({"connection_kind":"wood_insert","outer_thread":connection_thread_value(outer_thread),
                "inner_thread":connection_thread_value(inner_thread),"outer_thread_start":outer_thread_start,
                "inner_thread_start":inner_thread_start,"outer_complete_interval_mm":outer_complete_interval_mm,
                "inner_complete_interval_mm":inner_complete_interval_mm})
        }
        model::ResolvedCustomConnectionHardware::HangerBolt {
            wood_thread,
            plain_length_mm,
            plain_radius_mm,
            metric_thread,
            wood_thread_start,
            metric_thread_start,
            wood_complete_interval_mm,
            metric_complete_interval_mm,
            ..
        } => {
            json!({"connection_kind":"hanger_bolt","wood_thread":connection_thread_value(wood_thread),
                "metric_thread":connection_thread_value(metric_thread),"plain_length_mm":plain_length_mm,
                "plain_diameter_mm":2.*plain_radius_mm,"wood_thread_start":wood_thread_start,
                "metric_thread_start":metric_thread_start,"wood_complete_interval_mm":wood_complete_interval_mm,
                "metric_complete_interval_mm":metric_complete_interval_mm})
        }
    };
    value["kind"] = json!("custom_connection");
    value["frame"] = frame_value(parameters.frame());
    value["scope"] = json!(
        "Explicit custom engineered modeled profiles and flat ends; no purchased-part, strength or installation certification"
    );
    value
}
pub(super) fn inspect_mechanical_part(
    host: &dyn Host,
    _: &Request,
    args: InspectMechanicalPartArgs,
) -> ToolResult {
    let (doc, revision, eval) = snapshot(host)?;
    let feature = id(&args.feature)?;
    let source = model::read_mechanical_part(&doc, feature)?;
    let assigned_material = materials::resolve(&doc, source.output.body)?
        .material
        .and_then(|id| doc.get(id))
        .map(|entity| entity.name.as_str());
    let baseline = symbols::pinned_hardware_baseline(&doc, source.output.body, feature)?;
    let mut attribution = source.attribution(
        &eval,
        assigned_material,
        baseline.as_ref().and_then(|pin| pin.material.as_deref()),
    )?;
    let actual = source.parameters.resolve(&eval)?;
    if attribution.reference.is_none()
        && baseline.as_ref().is_some_and(|pin| pin.geometry != actual)
    {
        attribution.basis = model::MechanicalAttributionBasis::CustomDerived;
    }
    let library_pin =
        baseline.map(|pin| json!({"content_hash":pin.pin.to_string(),"revision":pin.revision}));
    let resolved = match actual {
        model::ResolvedMechanicalPart::CustomConnection { parameters } => {
            custom_connection_value(&parameters)
        }
        model::ResolvedMechanicalPart::WoodScrew { parameters: wood } => {
            let model::ResolvedScrewHead::ExternalHex {
                across_flats_mm,
                height_mm,
            } = wood.head
            else {
                return Err(ToolError::new(
                    "unsupported",
                    "Unsupported resolved wood head",
                ));
            };
            json!({"kind":"wood_screw","authored_length_mm":wood.authored_length_mm,"length_convention":wood.length_convention,"underhead_length_mm":wood.underhead_length_mm,"overall_length_mm":wood.overall_length_mm,"nominal_diameter_mm":wood.nominal_diameter_mm,"head":{"kind":"external_hex","across_flats_mm":across_flats_mm,"height_mm":height_mm},"drive":{"kind":"external_hex"},"reference_profile":{"pitch_mm":wood.reference_profile.pitch_mm,"core_diameter_mm":wood.reference_profile.core_diameter_mm,"included_angle_deg":wood.reference_profile.included_angle_deg},"minimum_tip_inclusive_thread_extent_mm":wood.minimum_tip_inclusive_thread_extent_mm,"maximum_possible_plain_length_mm":wood.maximum_possible_plain_length_mm,"point":wood.point,"representation":wood.representation,"manufacturing_available":false,"representation_limit":"Nominal stock envelope with a flat distal limit plane; actual thread and manufacturer point geometry are unknown","frame":frame_value(wood.frame)})
        }
        model::ResolvedMechanicalPart::Washer {
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            frame,
        } => {
            json!({"kind":"washer","bore_diameter_mm":2.*bore_radius_mm,"outside_diameter_mm":2.*outside_radius_mm,"thickness_mm":thickness_mm,"frame":frame_value(frame)})
        }
        model::ResolvedMechanicalPart::BearingEnvelope {
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            frame,
        } => {
            json!({"kind":"bearing_envelope","bore_diameter_mm":2.*bore_radius_mm,"outside_diameter_mm":2.*outside_radius_mm,"width_mm":width_mm,"frame":frame_value(frame)})
        }
        model::ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } => {
            json!({"kind":"hex_nut","across_flats_mm":across_flats_mm,"height_mm":thread.length_mm,"nominal_thread_diameter_mm":2.*thread.nominal_major_radius_mm,"nominal_pitch_diameter_mm":thread.nominal_pitch_diameter_mm,"actual_pitch_diameter_mm":thread.actual_pitch_diameter_mm,"fit":thread.fit,"pipe":thread.pipe,"npt":thread.npt,"lead_screw":thread.lead_screw,"buttress":thread.buttress,"round":thread.round,"metric_fit":thread.metric_fit,"metric_fit_engagement":thread.metric_fit_engagement,"radial_taper":thread.profile.radial_taper,"pitch_mm":thread.pitch_mm,"hand":thread.hand,"representation":thread.representation,"frame":frame_value(thread.frame)})
        }
        model::ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            authored_length_mm,
            length_convention,
            shank_radius_mm,
            head,
            drive,
            thread,
            ..
        } => {
            let head = match head {
                model::ResolvedScrewHead::SocketCap {
                    radius_mm,
                    height_mm,
                } => json!({"kind":"socket_cap","diameter_mm":2.*radius_mm,"height_mm":height_mm}),
                model::ResolvedScrewHead::ExternalHex {
                    across_flats_mm,
                    height_mm,
                } => {
                    json!({"kind":"external_hex","across_flats_mm":across_flats_mm,"height_mm":height_mm})
                }
                model::ResolvedScrewHead::Pan {
                    radius_mm,
                    height_mm,
                    crown_radius_mm,
                    top_land_radius_mm,
                    neck_radius_mm,
                } => {
                    json!({"kind":"pan","diameter_mm":2.*radius_mm,"height_mm":height_mm,"crown_radius_mm":crown_radius_mm,"top_land_diameter_mm":2.*top_land_radius_mm,"neck_radius_mm":neck_radius_mm})
                }
                model::ResolvedScrewHead::Button {
                    radius_mm,
                    height_mm,
                    crown_radius_mm,
                    top_land_radius_mm,
                    neck_radius_mm,
                } => {
                    json!({"kind":"button","diameter_mm":2.*radius_mm,"height_mm":height_mm,"crown_radius_mm":crown_radius_mm,"top_land_diameter_mm":2.*top_land_radius_mm,"neck_radius_mm":neck_radius_mm})
                }
                model::ResolvedScrewHead::Countersunk {
                    radius_mm,
                    height_mm,
                    included_angle_deg,
                    edge_chamfer_mm,
                    neck_radius_mm,
                } => {
                    json!({"kind":"countersunk","diameter_mm":2.*radius_mm,"height_mm":height_mm,"included_angle_deg":included_angle_deg,"edge_chamfer_mm":edge_chamfer_mm,"neck_radius_mm":neck_radius_mm})
                }
            };
            let drive = match drive {
                model::ResolvedScrewDrive::HexSocket {
                    across_flats_mm,
                    depth_mm,
                } => {
                    json!({"kind":"hex_socket","across_flats_mm":across_flats_mm,"depth_mm":depth_mm})
                }
                model::ResolvedScrewDrive::ExternalHex => json!({"kind":"external_hex"}),
                model::ResolvedScrewDrive::Hexalobular {
                    major_radius_mm,
                    minor_radius_mm,
                    outer_lobe_radius_mm,
                    depth_mm,
                    bottom_angle_deg,
                } => {
                    json!({"kind":"hexalobular","major_diameter_mm":2.*major_radius_mm,"minor_diameter_mm":2.*minor_radius_mm,"outer_lobe_radius_mm":outer_lobe_radius_mm,"depth_mm":depth_mm,"bottom_angle_deg":bottom_angle_deg})
                }
                model::ResolvedScrewDrive::PhillipsH { profile } => {
                    let inner = match profile.inner {
                        model::ResolvedPhillipsInnerForm::Rounded { radius_mm } => {
                            json!({"kind":"rounded","radius_mm":radius_mm})
                        }
                        model::ResolvedPhillipsInnerForm::ShallowV {
                            angle_deg,
                            e_width_mm,
                        } => {
                            json!({"kind":"shallow_v","angle_deg":angle_deg,"e_width_mm":e_width_mm})
                        }
                    };
                    json!({"kind":"phillips_h","profile":{"size":profile.size,"mouth_diameter_mm":profile.mouth_diameter_mm,"mouth_radius_mm":profile.mouth_radius_mm,"b_width_mm":profile.b_width_mm,"g_diameter_mm":profile.g_diameter_mm,"tilt_angle_deg":profile.tilt_angle_deg,"outer_flank_angle_deg":profile.outer_flank_angle_deg,"floor_angle_deg":profile.floor_angle_deg,"outer_corner_angle_deg":profile.outer_corner_angle_deg,"inner":inner}})
                }
            };
            json!({"kind":"machine_screw","underhead_length_mm":underhead_length_mm,"authored_length_mm":authored_length_mm,"length_convention":length_convention,"shank_diameter_mm":2.*shank_radius_mm,"thread_start_mm":underhead_length_mm-thread.length_mm,"thread_length_mm":thread.length_mm,"fully_threaded":underhead_length_mm==thread.length_mm,"head":head,"drive":drive,"nominal_thread_diameter_mm":2.*thread.nominal_major_radius_mm,"nominal_pitch_diameter_mm":thread.nominal_pitch_diameter_mm,"actual_pitch_diameter_mm":thread.actual_pitch_diameter_mm,"fit":thread.fit,"pipe":thread.pipe,"npt":thread.npt,"lead_screw":thread.lead_screw,"buttress":thread.buttress,"round":thread.round,"metric_fit":thread.metric_fit,"metric_fit_engagement":thread.metric_fit_engagement,"radial_taper":thread.profile.radial_taper,"pitch_mm":thread.pitch_mm,"hand":thread.hand,"representation":thread.representation,"frame":frame_value(thread.frame)})
        }
    };
    Ok(
        json!({"revision":revision,"feature":feature,"matches_reference_dimensions":attribution.dimensions_match.unwrap_or(false),"source":source,"resolved":resolved,"attribution":attribution,"library_pin":library_pin,"native_validation":"Not performed by this source inspection; use evaluate_model","catalog_verification":"A retained reference and dimensional match do not establish verified catalog membership or stock availability"}),
    )
}
pub(super) fn set_mechanical_part(
    host: &dyn Host,
    call: &Request,
    args: MechanicalPartArgs,
) -> ToolResult {
    let (doc, revision, eval) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            "Mechanical Part request is based on a stale revision",
        )
        .at_revision(revision));
    }
    let edit = args.feature.as_deref().map(id).transpose()?;
    let previous = edit
        .map(|f| model::read_mechanical_part(&doc, f))
        .transpose()?;
    if edit.is_some() && args.name.is_some() {
        return Err(ToolError::invalid(
            "Mechanical Part edits retain their name and output Part; omit name",
        ));
    }
    let parameters = parameters(
        &args.recipe,
        previous.as_ref().map(|p| &p.parameters),
        &doc,
        &eval,
    )?;
    let request = if edit.is_none() {
        Some(model::MechanicalPartRequest::new(
            args.name
                .ok_or_else(|| ToolError::invalid("A new mechanical Part needs a name"))?,
            parameters.clone(),
        ))
    } else {
        None
    };
    let expected = args.expected_revision;
    let owned = call.clone();
    let (prepared, feature, part) = with(host, move |target| {
        revision_guard(target, expected)?;
        let p = match request {
            Some(r) => {
                model::prepare_mechanical_part(target.session(), expected, owned.actor.clone(), &r)?
            }
            None => model::prepare_mechanical_part_edit(
                target.session(),
                expected,
                owned.actor.clone(),
                edit.expect("mechanical Part edit"),
                &parameters,
            )?,
        };
        let output = p.outputs[0].clone();
        Ok::<_, ToolError>((p.prepared, output.feature, output.body))
    })??;
    compile_without_display(prepared.document(), revision, call)?;
    let owned = call.clone();
    let committed = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({"revision":committed,"feature":feature,"part":part}))
}

#[cfg(test)]
mod length_contract_tests {
    use super::*;
    use multicad_core::commands::Session;
    #[test]
    fn typed_length_omission_replacement_and_conflict() {
        let s = Session::new(Document::new("Length wire contract").unwrap()).unwrap();
        let parse = |value: Value, old: Option<&MechanicalPartParameters>| {
            let args: MechanicalRecipeArgs = serde_json::from_value(value).unwrap();
            parameters(&args, old, s.document(), s.parameters())
        };
        let typed = parse(json!({"kind":"machine_screw","length_input":{"kind":"overall","value":{"value":13.,"unit":"mm"}}}),None).unwrap();
        let retained = parse(
            json!({"kind":"machine_screw","shank_diameter":{"value":3.,"unit":"mm"}}),
            Some(&typed),
        )
        .unwrap();
        assert_eq!(retained, typed);
        let old = parse(
            json!({"kind":"machine_screw","underhead_length":{"value":10.,"unit":"mm"}}),
            Some(&typed),
        )
        .unwrap();
        assert_eq!(old, model::default_machine_screw());
        assert!(parse(json!({"kind":"machine_screw","underhead_length":{"value":10.,"unit":"mm"},"length_input":{"kind":"overall","value":{"value":13.,"unit":"mm"}}}),Some(&typed)).is_err());
        let resolved = typed.resolve(s.parameters()).unwrap();
        assert!(matches!(
            resolved,
            model::ResolvedMechanicalPart::MachineScrew {
                underhead_length_mm: 10.,
                ..
            }
        ));
    }
}

#[cfg(test)]
mod profile_contract_tests {
    use super::*;
    use multicad_core::{
        commands::Session, constraint_sketch::Dimension, parameters::BoundExpression,
    };
    fn q(value: f64) -> Value {
        json!({"value":value,"unit":"mm"})
    }
    fn angle(value: f64) -> Value {
        json!({"value":value,"unit":"deg"})
    }
    fn parse(
        s: &Session,
        value: Value,
        old: Option<&MechanicalPartParameters>,
    ) -> Result<MechanicalPartParameters, ToolError> {
        let args: MechanicalRecipeArgs =
            serde_json::from_value(value).map_err(|e| ToolError::invalid(e.to_string()))?;
        parameters(&args, old, s.document(), s.parameters())
    }
    fn pan() -> Value {
        json!({"kind":"pan","diameter":q(5.6),"height":q(2.4),"crown_radius":q(5.),"top_land_diameter":q(4.),"neck_radius":q(0.1)})
    }
    fn lobes() -> Value {
        json!({"kind":"hexalobular","major_diameter":q(2.8),"minor_diameter":q(2.05),"outer_lobe_radius":q(0.28),"depth":q(1.),"bottom_angle":angle(120.)})
    }
    fn h1() -> Value {
        json!({"kind":"phillips_h","profile":{"size":"h1","mouth_diameter":q(3.),"mouth_radius":q(0.5),"b_width":q(0.955),"g_diameter":q(1.295),"tilt_angle":angle(7.125),"outer_flank_angle":angle(26.625),"floor_angle":angle(27.75),"outer_corner_angle":angle(91.875),"inner":{"kind":"shallow_v","angle":angle(137.875),"e_width":q(0.435)}}})
    }
    #[test]
    fn head_drive_partial_edits_preserve_source_and_require_explicit_new_kind_fields() {
        let s = Session::new(Document::new("Profile wire").unwrap()).unwrap();
        let mut source = parse(
            &s,
            json!({"kind":"machine_screw","head":pan(),"drive":lobes(),"thread":{"length":q(9.5)}}),
            None,
        )
        .unwrap();
        let bound = Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=5.6", "mm", None).unwrap(),
        };
        if let MechanicalPartParameters::MachineScrew {
            head: ScrewHead::Pan { profile },
            ..
        } = &mut source
        {
            profile.diameter = bound.clone();
        } else {
            panic!()
        }
        let unchanged = source.clone();
        let edited = parse(
            &s,
            json!({"kind":"machine_screw","head":{"kind":"pan","height":q(2.5)}}),
            Some(&source),
        )
        .unwrap();
        let MechanicalPartParameters::MachineScrew {
            head: ScrewHead::Pan { profile },
            drive,
            ..
        } = &edited
        else {
            panic!()
        };
        assert_eq!(profile.diameter, bound);
        assert_eq!(profile.height, length(2.5));
        let MechanicalPartParameters::MachineScrew {
            drive: old_drive, ..
        } = &source
        else {
            panic!()
        };
        assert_eq!(drive, old_drive);
        assert!(
            parse(
                &s,
                json!({"kind":"machine_screw","head":{"kind":"button","height":q(2.)}}),
                Some(&source)
            )
            .is_err()
        );
        assert!(
            parse(
                &s,
                json!({"kind":"machine_screw","drive":{"kind":"phillips_h"}}),
                Some(&source)
            )
            .is_err()
        );
        assert!(parse(&s,json!({"kind":"machine_screw","drive":{"kind":"hexalobular","bottom_angle":q(120.)}}),Some(&source)).is_err());
        let radians=parse(&s,json!({"kind":"machine_screw","drive":{"kind":"hexalobular","bottom_angle":{"value":std::f64::consts::TAU/3.,"unit":"rad"}}}),Some(&source)).unwrap();
        let model::ResolvedMechanicalPart::MachineScrew {
            drive:
                model::ResolvedScrewDrive::Hexalobular {
                    bottom_angle_deg, ..
                },
            ..
        } = radians.resolve(s.parameters()).unwrap()
        else {
            panic!()
        };
        assert!((bottom_angle_deg - 120.).abs() < 1e-10);
        assert_eq!(
            source, unchanged,
            "Partial argument preparation cannot commit or rewrite its input"
        );
    }
    #[test]
    fn phillips_inner_form_and_overall_length_keep_distinct_typed_authorities() {
        let s = Session::new(Document::new("Phillips wire").unwrap()).unwrap();
        let source = parse(
            &s,
            json!({"kind":"machine_screw","head":pan(),"drive":h1(),"thread":{"length":q(9.5)}}),
            None,
        )
        .unwrap();
        assert_eq!(
            parse(&s, json!({"kind":"machine_screw"}), Some(&source)).unwrap(),
            source
        );
        assert!(parse(&s,json!({"kind":"machine_screw","drive":{"kind":"phillips_h","profile":{"size":"h0"}}}),Some(&source)).is_err());
        let h0=parse(&s,json!({"kind":"machine_screw","drive":{"kind":"phillips_h","profile":{"size":"h0","mouth_diameter":q(1.9),"mouth_radius":q(0.3),"b_width":q(0.595),"g_diameter":q(0.835),"inner":{"kind":"rounded","radius":q(0.25)}}}}),Some(&source)).unwrap();
        let MechanicalPartParameters::MachineScrew {
            drive: ScrewDrive::PhillipsH { profile },
            ..
        } = &h0
        else {
            panic!()
        };
        assert!(matches!(
            profile.inner,
            model::PhillipsInnerForm::Rounded { .. }
        ));
        let wire = serde_json::to_value(&profile.inner).unwrap();
        assert!(wire.get("angle").is_none() && wire.get("e_width").is_none());
        assert!(parse(&s,json!({"kind":"machine_screw","drive":{"kind":"phillips_h","profile":{"inner":{"kind":"rounded","radius":q(0.25),"angle":angle(138.)}}}}),Some(&h0)).is_err());
        let csk = json!({"kind":"countersunk","diameter":q(5.5),"height":q(1.65),"included_angle":angle(90.),"edge_chamfer":q(0.05),"neck_radius":q(0.1)});
        let overall=parse(&s,json!({"kind":"machine_screw","length_input":{"kind":"overall","value":{"expression":"=13","unit":"mm"}},"head":csk,"drive":lobes(),"thread":{"length":q(9.5)}}),None).unwrap();
        let raised = parse(
            &s,
            json!({"kind":"machine_screw","head":{"kind":"countersunk","height":q(2.05)}}),
            Some(&overall),
        )
        .unwrap();
        let model::ResolvedMechanicalPart::MachineScrew {
            authored_length_mm,
            underhead_length_mm,
            ..
        } = raised.resolve(s.parameters()).unwrap()
        else {
            panic!()
        };
        assert_eq!(authored_length_mm, 13.);
        assert!((underhead_length_mm - 10.95).abs() < 1e-12);
        let MechanicalPartParameters::MachineScrew {
            underhead_length: None,
            length_input: Some(model::ScrewLengthInput::Overall { value }),
            ..
        } = raised
        else {
            panic!()
        };
        assert_eq!(value.expression().unwrap().text, "=13");
    }
}
