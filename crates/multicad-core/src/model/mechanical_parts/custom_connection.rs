//! Explicit engineered connection hardware. No purchased standard or wood profile is inferred.
use super::*;
use crate::references::ReferenceFrame;
pub const CUSTOM_CONNECTION_HARDWARE_CAPABILITY: &str = "model.mechanical_part.custom_connection.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "connection_kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CustomConnectionHardwareParameters {
    WoodScrew {
        wood_thread: Box<ThreadParameters>,
        plain_length: Dimension,
        plain_diameter: Dimension,
        head: Box<ScrewHead>,
        drive: Box<ScrewDrive>,
        frame: PrimitiveFrame,
    },
    WoodInsert {
        outer_thread: Box<ThreadParameters>,
        inner_thread: Box<ThreadParameters>,
        frame: PrimitiveFrame,
    },
    HangerBolt {
        wood_thread: Box<ThreadParameters>,
        plain_length: Dimension,
        plain_diameter: Dimension,
        metric_thread: Box<ThreadParameters>,
        frame: PrimitiveFrame,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedCustomConnectionHardware {
    WoodScrew {
        wood_thread: Box<ResolvedThread>,
        plain_length_mm: f64,
        plain_radius_mm: f64,
        head: ResolvedScrewHead,
        drive: ResolvedScrewDrive,
        frame: Frame,
        underhead_frame: ReferenceFrame,
        wood_thread_start: ReferenceFrame,
        wood_complete_interval_mm: [f64; 2],
    },
    WoodInsert {
        outer_thread: Box<ResolvedThread>,
        inner_thread: Box<ResolvedThread>,
        frame: Frame,
        outer_thread_start: ReferenceFrame,
        inner_thread_start: ReferenceFrame,
        outer_complete_interval_mm: [f64; 2],
        inner_complete_interval_mm: [f64; 2],
    },
    HangerBolt {
        wood_thread: Box<ResolvedThread>,
        plain_length_mm: f64,
        plain_radius_mm: f64,
        metric_thread: Box<ResolvedThread>,
        frame: Frame,
        wood_thread_start: ReferenceFrame,
        metric_thread_start: ReferenceFrame,
        wood_complete_interval_mm: [f64; 2],
        metric_complete_interval_mm: [f64; 2],
    },
}
impl ResolvedCustomConnectionHardware {
    pub fn frame(&self) -> Frame {
        match self {
            Self::WoodScrew { frame, .. }
            | Self::WoodInsert { frame, .. }
            | Self::HangerBolt { frame, .. } => *frame,
        }
    }
    /// Component-local frame to map to the retained connection station.
    /// A headed screw mounts at its underhead datum; other forms at wood entry.
    pub fn mount_frame(&self) -> ReferenceFrame {
        match self {
            Self::WoodScrew {
                underhead_frame, ..
            } => *underhead_frame,
            _ => start(self.frame(), 0.),
        }
    }
}
fn start(frame: Frame, station: f64) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: std::array::from_fn(|i| frame.origin[i] + station * frame.n[i]),
        u: frame.u,
        v: frame.v,
    }
}
impl CustomConnectionHardwareParameters {
    pub fn label(&self) -> &'static str {
        match self {
            Self::WoodScrew { .. } => "Custom modeled wood screw",
            Self::WoodInsert { .. } => "Custom modeled female wood insert",
            Self::HangerBolt { .. } => "Custom modeled hanger bolt",
        }
    }
    pub fn frame(&self) -> &PrimitiveFrame {
        match self {
            Self::WoodScrew { frame, .. }
            | Self::WoodInsert { frame, .. }
            | Self::HangerBolt { frame, .. } => frame,
        }
    }
    pub fn threads(&self) -> Vec<&ThreadParameters> {
        match self {
            Self::WoodScrew { wood_thread, .. } => vec![wood_thread],
            Self::WoodInsert {
                outer_thread,
                inner_thread,
                ..
            } => vec![outer_thread, inner_thread],
            Self::HangerBolt {
                wood_thread,
                metric_thread,
                ..
            } => vec![wood_thread, metric_thread],
        }
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        let mut f = self.frame().origin.iter().collect::<Vec<_>>();
        for t in self.threads() {
            f.extend(t.dimensions());
        }
        match self {
            Self::WoodScrew {
                plain_length,
                plain_diameter,
                head,
                drive,
                ..
            } => {
                f.extend([plain_length, plain_diameter]);
                f.extend(head.dimensions());
                f.extend(drive.dimensions());
            }
            Self::HangerBolt {
                plain_length,
                plain_diameter,
                ..
            } => f.extend([plain_length, plain_diameter]),
            _ => {}
        }
        f
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame().validate()?;
        for t in self.threads() {
            t.validate()?;
            if t.frame != PrimitiveFrame::default()
                || t.end_treatments.is_some()
                || t.representation_parameter.is_some()
                || t.representation != super::super::ThreadRepresentation::Modeled
                || t.starts != 1
            {
                return Err(invalid(
                    "Custom connection hardware requires literal Modeled single-start threads with identity local frames and flat untreated ends",
                ));
            }
        }
        let custom = |t: &ThreadParameters| -> Result<(), DocumentError> {
            if t.family != super::super::ThreadFamily::Custom
                || t.custom.is_none()
                || t.fit != super::super::ThreadFit::NominalBasic
            {
                return Err(invalid(
                    "Wood engagement requires an explicit Custom profile; ISO metric and purchased wood profiles are not aliases",
                ));
            }
            Ok(())
        };
        let metric = |t: &ThreadParameters, fit| -> Result<(), DocumentError> {
            if t.family != super::super::ThreadFamily::IsoMetric || t.fit != fit {
                return Err(invalid(
                    "Custom metal engagement requires the explicitly supported sourced metric fit",
                ));
            }
            Ok(())
        };
        match self {
            Self::WoodScrew {
                wood_thread,
                plain_length,
                plain_diameter,
                head,
                drive,
                ..
            } => {
                custom(wood_thread)?;
                check_dimension(plain_length, Unit::Millimeter)?;
                check_dimension(plain_diameter, Unit::Millimeter)?;
                head.validate_dimensions()?;
                drive.validate_dimensions()?;
            }
            Self::WoodInsert {
                outer_thread,
                inner_thread,
                ..
            } => {
                custom(outer_thread)?;
                metric(inner_thread, super::super::ThreadFit::Metric6HInternal)?;
            }
            Self::HangerBolt {
                wood_thread,
                metric_thread,
                plain_length,
                plain_diameter,
                ..
            } => {
                custom(wood_thread)?;
                metric(metric_thread, super::super::ThreadFit::Metric6gExternal)?;
                check_dimension(plain_length, Unit::Millimeter)?;
                check_dimension(plain_diameter, Unit::Millimeter)?;
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        p: &EvaluatedParameters,
    ) -> Result<ResolvedCustomConnectionHardware, DocumentError> {
        self.validate()?;
        let frame = self.frame().resolve(p)?;
        let thread = |t: &ThreadParameters, mode| -> Result<ResolvedThread, DocumentError> {
            let mut r = t.resolve(p, mode)?;
            if r.profile.radial_taper != 0. {
                return Err(invalid(
                    "Custom connection hardware currently requires straight actual thread profiles",
                ));
            }
            r.frame = frame;
            Ok(r)
        };
        let plain = |length: &Dimension,
                     diameter: &Dimension|
         -> Result<(f64, f64), DocumentError> {
            let l = number(length, Unit::Millimeter, p)?;
            let r = number(diameter, Unit::Millimeter, p)? / 2.;
            if l < 0. || r <= 0. {
                return Err(invalid(
                    "Custom hardware needs a nonnegative plain length and positive authored plain diameter",
                ));
            }
            Ok((l, r))
        };
        Ok(match self {
            Self::WoodScrew {
                wood_thread,
                plain_length,
                plain_diameter,
                head,
                drive,
                ..
            } => {
                let wood = thread(wood_thread, ThreadMode::NewExternal)?;
                let (l, r) = plain(plain_length, plain_diameter)?;
                // Reuse exactly the existing headed-thread geometric safety rules, with
                // the actual Custom profile. This temporary validation value is not source.
                let validation = MechanicalPartParameters::MachineScrew {
                    underhead_length: Some(super::super::length(l + wood.length_mm)),
                    length_input: None,
                    shank_diameter: plain_diameter.clone(),
                    head: head.as_ref().clone(),
                    drive: drive.as_ref().clone(),
                    thread: wood_thread.clone(),
                }
                .resolve(p)?;
                let ResolvedMechanicalPart::MachineScrew { head, drive, .. } = validation else {
                    unreachable!()
                };
                let end = wood.length_mm;
                ResolvedCustomConnectionHardware::WoodScrew {
                    wood_thread: Box::new(wood),
                    plain_length_mm: l,
                    plain_radius_mm: r,
                    head,
                    drive,
                    frame,
                    underhead_frame: start(frame, -l),
                    wood_thread_start: start(frame, 0.),
                    wood_complete_interval_mm: [0., end],
                }
            }
            Self::WoodInsert {
                outer_thread,
                inner_thread,
                ..
            } => {
                let outer = thread(outer_thread, ThreadMode::NewExternal)?;
                let inner = thread(inner_thread, ThreadMode::CutInternal)?;
                if inner.length_mm > outer.length_mm
                    || outer.major_radius_mm - outer.profile.depth_mm - inner.major_radius_mm
                        < inner.minimum_wall_mm.max(outer.minimum_wall_mm)
                {
                    return Err(invalid(
                        "Custom insert needs sufficient material between its actual outer root and inner major radius across the full known inner span",
                    ));
                }
                let (a, b) = (outer.length_mm, inner.length_mm);
                ResolvedCustomConnectionHardware::WoodInsert {
                    outer_thread: Box::new(outer),
                    inner_thread: Box::new(inner),
                    frame,
                    outer_thread_start: start(frame, 0.),
                    inner_thread_start: start(frame, 0.),
                    outer_complete_interval_mm: [0., a],
                    inner_complete_interval_mm: [0., b],
                }
            }
            Self::HangerBolt {
                wood_thread,
                metric_thread,
                plain_length,
                plain_diameter,
                ..
            } => {
                let wood = thread(wood_thread, ThreadMode::NewExternal)?;
                let mut metric = thread(metric_thread, ThreadMode::NewExternal)?;
                let (l, r) = plain(plain_length, plain_diameter)?;
                let (a, b) = (wood.length_mm, metric.length_mm);
                let metric_start = start(frame, -l - b);
                metric.frame.origin = metric_start.origin_mm;
                ResolvedCustomConnectionHardware::HangerBolt {
                    wood_thread: Box::new(wood),
                    metric_thread: Box::new(metric),
                    plain_length_mm: l,
                    plain_radius_mm: r,
                    frame,
                    wood_thread_start: start(frame, 0.),
                    metric_thread_start: metric_start,
                    wood_complete_interval_mm: [0., a],
                    metric_complete_interval_mm: [0., b],
                }
            }
        })
    }
}
/// Explicit engineered example, not a manufacturer/DIN wood-thread profile.
pub fn default_custom_wood_thread() -> ThreadParameters {
    ThreadParameters {
        family: super::super::ThreadFamily::Custom,
        major_diameter: super::super::length(6.),
        pitch: super::super::length(2.),
        length: super::super::length(12.),
        custom: Some(super::super::CustomThreadProfile {
            depth: super::super::length(1.),
            crest_width: super::super::length(0.4),
            lower_flank: super::super::angle(30.),
            upper_flank: super::super::angle(30.),
            radial_taper: Dimension::Literal {
                value: 0.,
                unit: "ratio".into(),
            },
        }),
        minimum_wall: super::super::length(0.5),
        ..Default::default()
    }
}
pub fn default_custom_connection_wood_screw() -> MechanicalPartParameters {
    MechanicalPartParameters::CustomConnection {
        parameters: Box::new(CustomConnectionHardwareParameters::WoodScrew {
            wood_thread: Box::new(default_custom_wood_thread()),
            plain_length: super::super::length(0.),
            plain_diameter: super::super::length(6.),
            head: Box::new(ScrewHead::ExternalHex {
                across_flats: super::super::length(10.),
                height: super::super::length(4.),
            }),
            drive: Box::new(ScrewDrive::ExternalHex),
            frame: Default::default(),
        }),
    }
}
pub fn default_custom_connection_wood_insert() -> MechanicalPartParameters {
    let mut outer = default_custom_wood_thread();
    outer.major_diameter = super::super::length(10.);
    outer.length = super::super::length(4.);
    let inner = ThreadParameters {
        major_diameter: super::super::length(3.),
        pitch: super::super::length(0.5),
        length: super::super::length(4.),
        fit: super::super::ThreadFit::Metric6HInternal,
        ..Default::default()
    };
    MechanicalPartParameters::CustomConnection {
        parameters: Box::new(CustomConnectionHardwareParameters::WoodInsert {
            outer_thread: Box::new(outer),
            inner_thread: Box::new(inner),
            frame: Default::default(),
        }),
    }
}
pub fn default_custom_connection_hanger_bolt() -> MechanicalPartParameters {
    let metric = ThreadParameters {
        major_diameter: super::super::length(3.),
        pitch: super::super::length(0.5),
        length: super::super::length(12.),
        fit: super::super::ThreadFit::Metric6gExternal,
        metric_fit_engagement: Some(super::super::MetricFitEngagement::NormalGroupAssumed {}),
        ..Default::default()
    };
    MechanicalPartParameters::CustomConnection {
        parameters: Box::new(CustomConnectionHardwareParameters::HangerBolt {
            wood_thread: Box::new(default_custom_wood_thread()),
            plain_length: super::super::length(2.),
            plain_diameter: super::super::length(3.),
            metric_thread: Box::new(metric),
            frame: Default::default(),
        }),
    }
}
