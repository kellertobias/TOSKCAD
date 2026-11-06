//! Wood reference intent and a deliberately nonmanufacturing stock envelope.
use super::*;
pub const WOOD_SCREW_CAPABILITY: &str = "model.mechanical_part.wood_screw.1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WoodPointIntent {
    ManufacturerUnspecified,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WoodScrewRepresentation {
    SimplifiedEnvelope,
}
/// Reference dimensions only; no implied helical truncations, start or point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WoodVReference {
    pub pitch: Dimension,
    pub core_diameter: Dimension,
    pub included_angle: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WoodScrewParameters {
    pub length_input: ScrewLengthInput,
    pub nominal_diameter: Dimension,
    pub head: ScrewHead,
    pub drive: ScrewDrive,
    pub reference_profile: WoodVReference,
    /// Source lower bound including the unknown point and thread-start transition.
    pub minimum_tip_inclusive_thread_extent: Dimension,
    pub point: WoodPointIntent,
    pub representation: WoodScrewRepresentation,
    pub frame: PrimitiveFrame,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWoodVReference {
    pub pitch_mm: f64,
    pub core_diameter_mm: f64,
    pub included_angle_deg: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWoodScrew {
    pub authored_length_mm: f64,
    pub length_convention: ScrewLengthConvention,
    pub underhead_length_mm: f64,
    pub overall_length_mm: f64,
    pub nominal_diameter_mm: f64,
    pub head: ResolvedScrewHead,
    pub drive: ResolvedScrewDrive,
    pub reference_profile: ResolvedWoodVReference,
    pub minimum_tip_inclusive_thread_extent_mm: f64,
    /// Upper bound on a possible plain zone, not the actual thread start.
    pub maximum_possible_plain_length_mm: f64,
    pub point: WoodPointIntent,
    pub representation: WoodScrewRepresentation,
    pub frame: Frame,
}
impl WoodScrewParameters {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        let mut fields = vec![
            self.length_input.value(),
            &self.nominal_diameter,
            &self.reference_profile.pitch,
            &self.reference_profile.core_diameter,
            &self.reference_profile.included_angle,
            &self.minimum_tip_inclusive_thread_extent,
        ];
        fields.extend(self.head.dimensions());
        fields.extend(self.drive.dimensions());
        fields
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame.validate()?;
        for d in [
            self.length_input.value(),
            &self.nominal_diameter,
            &self.reference_profile.pitch,
            &self.reference_profile.core_diameter,
            &self.minimum_tip_inclusive_thread_extent,
        ] {
            check_dimension(d, Unit::Millimeter)?;
        }
        check_dimension(&self.reference_profile.included_angle, Unit::Degree)?;
        self.head.validate_dimensions()?;
        self.drive.validate_dimensions()?;
        if !matches!(
            (&self.head, &self.drive),
            (ScrewHead::ExternalHex { .. }, ScrewDrive::ExternalHex)
        ) {
            return Err(invalid(
                "Wood screw envelopes currently require an external-hex head and external-hex drive; other head/drive definitions are unavailable",
            ));
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedWoodScrew, DocumentError> {
        self.validate()?;
        let mm = |d| number(d, Unit::Millimeter, evaluated);
        let head = self.head.resolve(evaluated)?;
        let drive = self.drive.resolve(evaluated)?;
        let authored_length_mm = mm(self.length_input.value())?;
        let length_convention = self.length_input.convention();
        let underhead_length_mm = match length_convention {
            ScrewLengthConvention::Underhead => authored_length_mm,
            ScrewLengthConvention::Overall => authored_length_mm - head.height_mm(),
        };
        let nominal_diameter_mm = mm(&self.nominal_diameter)?;
        let pitch_mm = mm(&self.reference_profile.pitch)?;
        let core_diameter_mm = mm(&self.reference_profile.core_diameter)?;
        let included_angle_deg = number(
            &self.reference_profile.included_angle,
            Unit::Degree,
            evaluated,
        )?;
        let minimum_tip_inclusive_thread_extent_mm = mm(&self.minimum_tip_inclusive_thread_extent)?;
        if ![
            underhead_length_mm,
            nominal_diameter_mm,
            pitch_mm,
            core_diameter_mm,
            head.height_mm(),
            minimum_tip_inclusive_thread_extent_mm,
        ]
        .into_iter()
        .all(|v| v.is_finite() && v > 0.)
            || core_diameter_mm >= nominal_diameter_mm
            || !included_angle_deg.is_finite()
            || included_angle_deg <= 0.
            || included_angle_deg >= 180.
            || head.minimum_radius_mm() <= nominal_diameter_mm / 2.
            || minimum_tip_inclusive_thread_extent_mm < 0.6 * underhead_length_mm
            || minimum_tip_inclusive_thread_extent_mm > underhead_length_mm
        {
            return Err(invalid(
                "Wood reference needs positive stock/head/length/profile dimensions, core smaller than nominal, angle between 0 and 180 degrees, and a tip-inclusive minimum extent between 0.6 and 1 times underhead length",
            ));
        }
        let overall_length_mm = underhead_length_mm + head.height_mm();
        if !overall_length_mm.is_finite() {
            return Err(invalid("Wood overall length must be finite"));
        }
        Ok(ResolvedWoodScrew {
            authored_length_mm,
            length_convention,
            underhead_length_mm,
            overall_length_mm,
            nominal_diameter_mm,
            head,
            drive,
            reference_profile: ResolvedWoodVReference {
                pitch_mm,
                core_diameter_mm,
                included_angle_deg,
            },
            minimum_tip_inclusive_thread_extent_mm,
            maximum_possible_plain_length_mm: underhead_length_mm
                - minimum_tip_inclusive_thread_extent_mm,
            point: self.point,
            representation: self.representation,
            frame: self.frame.resolve(evaluated)?,
        })
    }
}
/// Generic editable envelope with retained reference intent, without catalog attribution.
pub fn default_wood_screw() -> MechanicalPartParameters {
    MechanicalPartParameters::WoodScrew {
        parameters: Box::new(WoodScrewParameters {
            length_input: ScrewLengthInput::Underhead {
                value: super::super::length(40.),
            },
            nominal_diameter: super::super::length(6.),
            head: ScrewHead::ExternalHex {
                across_flats: super::super::length(10.),
                height: super::super::length(4.),
            },
            drive: ScrewDrive::ExternalHex,
            reference_profile: WoodVReference {
                pitch: super::super::length(2.6),
                core_diameter: super::super::length(4.2),
                included_angle: Dimension::Literal {
                    value: 60.,
                    unit: "deg".into(),
                },
            },
            minimum_tip_inclusive_thread_extent: super::super::length(24.),
            point: WoodPointIntent::ManufacturerUnspecified,
            representation: WoodScrewRepresentation::SimplifiedEnvelope,
            frame: PrimitiveFrame::default(),
        }),
    }
}
