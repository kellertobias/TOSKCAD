//! Bounded receiving-hole recommendations, separate from purchased hardware.
//! Source identity and interpretation: library/sources/fastener-connections/.
//! These nominal defaults do not certify a fit, material, strength or toolpath.
use serde::{Deserialize, Serialize};

pub const METRIC_RECEIVING_SOURCE: &str = "norelem-technical-screws-nuts-undated";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricConnectionSize {
    M3,
    M4,
    M5,
    M6,
    M8,
    M10,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricClearanceSeries {
    Fine,
    Medium,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricReceivingRecommendation {
    pub size: MetricConnectionSize,
    pub nominal_diameter_mm: f64,
    pub coarse_pitch_mm: f64,
    /// Pre-tapping drill recommendation, not a modeled internal thread.
    pub tap_drill_diameter_mm: f64,
    /// Fine H12 and medium H13 nominal diameters in the source's ISO 273 table.
    pub fine_clearance_diameter_mm: f64,
    pub medium_clearance_diameter_mm: f64,
    pub medium_countersink: MetricCountersinkRecommendation,
    pub socket_cap_counterbore: MetricCounterboreRecommendation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricCountersinkRecommendation {
    pub diameter_mm: f64,
    /// Form A nominal angle. The source permits +1/0 degrees.
    pub included_angle_deg: f64,
    /// Rounded T1 marked approximately in the source. Never a second geometric
    /// constraint alongside D1, D2 and the nominal included angle.
    pub approximate_reference_depth_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricCounterboreRecommendation {
    pub diameter_mm: f64,
    /// Form K for DIN 912 socket cap heads. Not the shallower Form J/DIN 6912.
    pub depth_mm: f64,
    /// Permitted positive depth deviation; the negative deviation is zero.
    pub positive_depth_deviation_mm: f64,
}

impl MetricReceivingRecommendation {
    pub fn clearance_diameter_mm(self, series: MetricClearanceSeries) -> f64 {
        match series {
            MetricClearanceSeries::Fine => self.fine_clearance_diameter_mm,
            MetricClearanceSeries::Medium => self.medium_clearance_diameter_mm,
        }
    }

    /// Exact depth of the nominal Form A cone intersecting the medium hole.
    /// The rounded approximate T1 in the source is retained for attribution.
    pub fn medium_countersink_depth_mm(self) -> f64 {
        (self.medium_countersink.diameter_mm - self.medium_clearance_diameter_mm)
            / (2. * (self.medium_countersink.included_angle_deg.to_radians() / 2.).tan())
    }
}

impl MetricConnectionSize {
    pub const ALL: [Self; 6] = [Self::M3, Self::M4, Self::M5, Self::M6, Self::M8, Self::M10];

    pub fn receiving_recommendation(self) -> MetricReceivingRecommendation {
        use MetricConnectionSize::*;
        let (d, p, tap, fine, medium, cone, approx_depth, counterbore, depth, deviation) =
            match self {
                M3 => (3., 0.5, 2.5, 3.2, 3.4, 6.6, 1.6, 6., 3.4, 0.2),
                M4 => (4., 0.7, 3.3, 4.3, 4.5, 9., 2.3, 8., 4.6, 0.4),
                M5 => (5., 0.8, 4.2, 5.3, 5.5, 11., 2.8, 10., 5.7, 0.4),
                M6 => (6., 1., 5., 6.4, 6.6, 13., 3.2, 11., 6.8, 0.4),
                M8 => (8., 1.25, 6.8, 8.4, 9., 17.2, 4.1, 15., 9., 0.4),
                M10 => (10., 1.5, 8.5, 10.5, 11., 21.5, 5.3, 18., 11., 0.4),
            };
        MetricReceivingRecommendation {
            size: self,
            nominal_diameter_mm: d,
            coarse_pitch_mm: p,
            tap_drill_diameter_mm: tap,
            fine_clearance_diameter_mm: fine,
            medium_clearance_diameter_mm: medium,
            medium_countersink: MetricCountersinkRecommendation {
                diameter_mm: cone,
                included_angle_deg: 90.,
                approximate_reference_depth_mm: approx_depth,
            },
            socket_cap_counterbore: MetricCounterboreRecommendation {
                diameter_mm: counterbore,
                depth_mm: depth,
                positive_depth_deviation_mm: deviation,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receiving_recommendations_preserve_distinct_drill_and_clearance_intent() {
        for size in MetricConnectionSize::ALL {
            let r = size.receiving_recommendation();
            assert!(r.tap_drill_diameter_mm < r.nominal_diameter_mm);
            assert!(r.nominal_diameter_mm < r.fine_clearance_diameter_mm);
            assert!(r.fine_clearance_diameter_mm < r.medium_clearance_diameter_mm);
            assert!(r.medium_clearance_diameter_mm < r.socket_cap_counterbore.diameter_mm);
            // Reconstruct the source's nominal D2 from the actual conical depth.
            let depth = r.medium_countersink_depth_mm();
            let diameter = r.medium_clearance_diameter_mm
                + 2. * depth * (r.medium_countersink.included_angle_deg.to_radians() / 2.).tan();
            assert!((diameter - r.medium_countersink.diameter_mm).abs() < 1e-12);
            assert!(
                (depth - r.medium_countersink.approximate_reference_depth_mm).abs() <= 0.05 + 1e-12
            );
        }
    }

    #[test]
    fn m4_cone_uses_geometry_instead_of_rounded_reference_depth() {
        let r = MetricConnectionSize::M4.receiving_recommendation();
        assert!((r.medium_countersink_depth_mm() - 2.25).abs() < 1e-12);
        assert_eq!(r.medium_countersink.approximate_reference_depth_mm, 2.3);
    }

    #[test]
    fn metric_sizes_are_explicit_and_do_not_accept_unsourced_nearby_designations() {
        assert_eq!(
            serde_json::from_str::<MetricConnectionSize>("\"m6\"").unwrap(),
            MetricConnectionSize::M6
        );
        for unsupported in ["m7", "m12", "m6x0.75", "1/4-20"] {
            assert!(serde_json::from_value::<MetricConnectionSize>(unsupported.into()).is_err());
        }
    }
}
