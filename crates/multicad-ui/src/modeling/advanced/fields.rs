//! Authored dimensions stay bound; literal directions and plane axes are explicit.
use super::*;
use multicad_core::{
    constraint_sketch::Dimension,
    model::{AdvancedParameters, PrimitiveFrame},
};
fn fields(values: &[Dimension; 3], unit: &'static str) -> Result<[DimensionField; 3], String> {
    Ok([
        DimensionField::from_dimension(&values[0], unit)?,
        DimensionField::from_dimension(&values[1], unit)?,
        DimensionField::from_dimension(&values[2], unit)?,
    ])
}
fn resolve(values: &[DimensionField; 3], document: &Document) -> Result<[Dimension; 3], String> {
    Ok([
        values[0].resolve(document)?,
        values[1].resolve(document)?,
        values[2].resolve(document)?,
    ])
}
fn vector_fields(ui: &mut egui::Ui, values: &mut [DimensionField; 3], prefix: &str) -> bool {
    let mut changed = false;
    for (field, axis) in values.iter_mut().zip(["X", "Y", "Z"]) {
        changed |= field.show(
            ui,
            &format!("{prefix} {axis}"),
            "Value or =$parameter; dimensions remain associative",
        );
    }
    changed
}
fn direction(ui: &mut egui::Ui, title: &str, value: &mut [f64; 3]) -> bool {
    ui.label(title);
    let mut changed = false;
    for (axis, v) in ["X", "Y", "Z"].into_iter().zip(value) {
        ui.horizontal(|ui| {
            ui.label(axis);
            changed |= ui
                .add(egui::DragValue::new(v).speed(0.01).max_decimals(6))
                .changed();
        });
    }
    changed
}
pub(super) struct PlaneFields {
    origin: [DimensionField; 3],
    u: [f64; 3],
    v: [f64; 3],
}
impl PlaneFields {
    fn new(p: &PrimitiveFrame) -> Result<Self, String> {
        Ok(Self {
            origin: fields(&p.origin, "mm")?,
            u: p.u,
            v: p.v,
        })
    }
    fn resolve(&self, document: &Document) -> Result<PrimitiveFrame, String> {
        let frame = PrimitiveFrame {
            origin: resolve(&self.origin, document)?,
            u: self.u,
            v: self.v,
        };
        frame.validate().map_err(|e| e.to_string())?;
        Ok(frame)
    }
    fn show(&mut self, ui: &mut egui::Ui, title: &str) -> bool {
        ui.strong(title);
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            for (label, u, v) in [
                ("XY", [1., 0., 0.], [0., 1., 0.]),
                ("YZ", [0., 1., 0.], [0., 0., 1.]),
                ("XZ", [1., 0., 0.], [0., 0., 1.]),
            ] {
                if ui
                    .button(label)
                    .on_hover_text("Set literal plane directions; keep the bound origin")
                    .clicked()
                {
                    self.u = u;
                    self.v = v;
                    changed = true;
                }
            }
        });
        changed |= vector_fields(ui, &mut self.origin, "Plane origin");
        crate::controls::inspector_collapsing(
            ui,
            "advanced-plane-directions",
            "Custom plane directions",
            |ui| {
                changed |= direction(ui, "Plane U (unit direction)", &mut self.u);
                changed |= direction(ui, "Plane V (orthogonal unit direction)", &mut self.v);
                ui.small("U and V must be unit length and perpendicular. Preview reports invalid frames; axes are never silently corrected.");
            },
        );
        changed
    }
}
pub(super) enum ParameterFields {
    Shell {
        thickness: DimensionField,
    },
    Sweep,
    Loft {
        ruled: bool,
        seams: Vec<EntityId>,
    },
    Draft {
        angle: DimensionField,
        neutral: PlaneFields,
        pull: [f64; 3],
    },
    Mirror {
        plane: PlaneFields,
    },
    Linear {
        count: DimensionField,
        step: [DimensionField; 3],
        merge: bool,
    },
    Circular {
        count: DimensionField,
        angle: DimensionField,
        origin: [DimensionField; 3],
        axis: [f64; 3],
        merge: bool,
    },
}
impl ParameterFields {
    pub fn new(p: &AdvancedParameters) -> Result<Self, String> {
        Ok(match p {
            AdvancedParameters::Shell(p) => Self::Shell {
                thickness: DimensionField::from_dimension(&p.thickness, "mm")?,
            },
            AdvancedParameters::Sweep(_) => Self::Sweep,
            AdvancedParameters::Loft(p) => Self::Loft {
                ruled: p.ruled,
                seams: p.seams.clone(),
            },
            AdvancedParameters::Draft(p) => Self::Draft {
                angle: DimensionField::from_dimension(&p.angle, "deg")?,
                neutral: PlaneFields::new(&p.neutral)?,
                pull: p.pull_direction,
            },
            AdvancedParameters::Mirror(p) => Self::Mirror {
                plane: PlaneFields::new(&p.plane)?,
            },
            AdvancedParameters::LinearPattern(p) => Self::Linear {
                count: DimensionField::from_dimension(&p.count, "count")?,
                step: fields(&p.step, "mm")?,
                merge: p.merge,
            },
            AdvancedParameters::CircularPattern(p) => Self::Circular {
                count: DimensionField::from_dimension(&p.count, "count")?,
                angle: DimensionField::from_dimension(&p.step_angle, "deg")?,
                origin: fields(&p.origin, "mm")?,
                axis: p.axis,
                merge: p.merge,
            },
        })
    }
    pub fn resolve(&self, d: &Document) -> Result<AdvancedParameters, String> {
        Ok(match self {
            Self::Shell { thickness } => AdvancedParameters::Shell(model::ShellParameters {
                thickness: thickness.resolve(d)?,
            }),
            Self::Sweep => AdvancedParameters::Sweep(Default::default()),
            Self::Loft { ruled, seams } => AdvancedParameters::Loft(model::LoftParameters {
                ruled: *ruled,
                seams: seams.clone(),
            }),
            Self::Draft {
                angle,
                neutral,
                pull,
            } => AdvancedParameters::Draft(model::DraftParameters {
                angle: angle.resolve(d)?,
                neutral: neutral.resolve(d)?,
                pull_direction: *pull,
            }),
            Self::Mirror { plane } => AdvancedParameters::Mirror(model::MirrorParameters {
                plane: plane.resolve(d)?,
            }),
            Self::Linear { count, step, merge } => {
                AdvancedParameters::LinearPattern(model::LinearPatternParameters {
                    count: count.resolve(d)?,
                    step: resolve(step, d)?,
                    merge: *merge,
                })
            }
            Self::Circular {
                count,
                angle,
                origin,
                axis,
                merge,
            } => AdvancedParameters::CircularPattern(model::CircularPatternParameters {
                count: count.resolve(d)?,
                step_angle: angle.resolve(d)?,
                origin: resolve(origin, d)?,
                axis: *axis,
                merge: *merge,
            }),
        })
    }
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        match self {
            Self::Shell { thickness } => {
                changed |= thickness.show(
                    ui,
                    "Wall thickness",
                    "Signed millimeters: positive inward, negative outward; select opening faces",
                );
                ui.small("This operation hollows the native Part with selected opening faces. It is not a mesh offset or slicer wall setting.");
            }
            Self::Sweep => {
                ui.small("One closed section follows an explicitly ordered open straight-line path. Arc/spline paths and section holes are unsupported in this version.");
            }
            Self::Loft { ruled, .. } => {
                changed |= ui
                    .checkbox(ruled, "Ruled surfaces between sections")
                    .on_hover_text(
                        "Straight ruled transitions instead of a smooth through-sections surface",
                    )
                    .changed();
                ui.small("Sections and their start curves are explicitly ordered below. Holes or incompatible section correspondence are rejected by native Preview.");
            }
            Self::Draft {
                angle,
                neutral,
                pull,
            } => {
                changed|=angle.show(ui,"Draft angle","Signed angle about the neutral plane; native Preview checks the selected faces");
                changed |= neutral.show(ui, "Neutral plane");
                changed |= direction(ui, "Pull direction (world axes)", pull);
            }
            Self::Mirror { plane } => {
                changed |= plane.show(ui, "Mirror plane");
                ui.small("Replace this Part's shape with its reflected shape. Source history remains editable; linked Instances follow the Part definition.");
            }
            Self::Linear { count, step, merge } => {
                changed |= count.show(
                    ui,
                    "Member count",
                    "Integer 1–128, including the original; expressions use count units",
                );
                changed |= vector_fields(ui, step, "Step");
                changed |= ui
                    .checkbox(merge, "Fuse touching members")
                    .on_hover_text(
                        "Perform a real native union; a failed union keeps this editable draft",
                    )
                    .changed();
                ui.small("One parametric Part contains the original and generated members. Members are not independent assembly Instances.");
            }
            Self::Circular {
                count,
                angle,
                origin,
                axis,
                merge,
            } => {
                changed |= count.show(ui, "Member count", "Integer 1–128, including the original");
                changed |= angle.show(
                    ui,
                    "Step angle",
                    "Angular spacing per member, not the total sweep",
                );
                changed |= vector_fields(ui, origin, "Axis origin");
                changed |= direction(ui, "Rotation axis (world direction)", axis);
                changed |= ui.checkbox(merge, "Fuse touching members").changed();
                ui.small("One native compound Part contains all members unless a real union is requested.");
            }
        }
        changed
    }
}
