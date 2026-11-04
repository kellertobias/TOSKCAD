//! Explicit custom end geometry. These are neither DIN 76 nor natural runout.
use super::*;
pub const THREAD_END_TREATMENT_CAPABILITY: &str = "model.thread.end_treatments.1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ThreadStartTreatment {
    FlatCut,
    Chamfer45 { radial_depth: Dimension },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ThreadEndTreatment {
    FlatCut,
    CylindricalRelief {
        length: Dimension,
        radial_depth: Dimension,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadEndTreatments {
    pub start: ThreadStartTreatment,
    pub end: ThreadEndTreatment,
}
impl ThreadEndTreatments {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        let mut result = Vec::new();
        if let ThreadStartTreatment::Chamfer45 { radial_depth } = &self.start {
            result.push(radial_depth);
        }
        if let ThreadEndTreatment::CylindricalRelief {
            length,
            radial_depth,
        } = &self.end
        {
            result.extend([length, radial_depth]);
        }
        result
    }
    pub(super) fn validate(&self) -> Result<(), DocumentError> {
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub(super) fn resolve(
        &self,
        parameters: &EvaluatedParameters,
        major_radius: f64,
        root_radius: f64,
        pitch: f64,
        length: f64,
        mode: ThreadMode,
    ) -> Result<ResolvedThreadEndTreatments, DocumentError> {
        let positive = |d: &Dimension| -> Result<f64, DocumentError> {
            let value = number(d, Unit::Millimeter, parameters)?;
            if value < 0.001 {
                return Err(invalid(
                    "Enabled thread treatment dimensions must be at least 0.001 mm",
                ));
            }
            Ok(value)
        };
        let start_depth = match &self.start {
            ThreadStartTreatment::FlatCut => 0.,
            ThreadStartTreatment::Chamfer45 { radial_depth } => positive(radial_depth)?,
        };
        let (end_length, end_depth) = match &self.end {
            ThreadEndTreatment::FlatCut => (0., 0.),
            ThreadEndTreatment::CylindricalRelief {
                length,
                radial_depth,
            } => (positive(length)?, positive(radial_depth)?),
        };
        if start_depth + end_length + pitch > length {
            return Err(invalid(
                "Thread end treatments must retain at least one complete central pitch inside the authored length",
            ));
        }
        if mode != ThreadMode::CutInternal
            && (major_radius - start_depth <= 0.001 || root_radius - end_depth <= 0.001)
        {
            return Err(invalid(
                "External end treatment leaves no positive shaft core",
            ));
        }
        let maximum_cutter_radius = if mode == ThreadMode::CutInternal {
            major_radius
                .max(root_radius + start_depth)
                .max(major_radius + end_depth)
        } else {
            major_radius
        };
        Ok(ResolvedThreadEndTreatments {
            intent: self.clone(),
            start_chamfer_radial_depth_mm: start_depth,
            end_relief_length_mm: end_length,
            end_relief_radial_depth_mm: end_depth,
            full_profile_start_mm: start_depth,
            full_profile_end_mm: length - end_length,
            full_profile_length_mm: length - end_length - start_depth,
            maximum_cutter_radius_mm: maximum_cutter_radius,
            scope: "Custom 45-degree entry chamfer and square cylindrical terminal relief inside authored length; not DIN76 shoulder relief or natural helical runout",
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedThreadEndTreatments {
    pub intent: ThreadEndTreatments,
    pub start_chamfer_radial_depth_mm: f64,
    pub end_relief_length_mm: f64,
    pub end_relief_radial_depth_mm: f64,
    pub full_profile_start_mm: f64,
    pub full_profile_end_mm: f64,
    pub full_profile_length_mm: f64,
    pub maximum_cutter_radius_mm: f64,
    pub scope: &'static str,
}
