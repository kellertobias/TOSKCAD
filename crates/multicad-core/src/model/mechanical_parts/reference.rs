//! Dimensional attribution is separate from catalog trust and stock availability.
use super::*;
use std::collections::BTreeMap;
fn thread_reference_values(thread: &ResolvedThread) -> BTreeMap<&'static str, f64> {
    let mut values = BTreeMap::from([
        ("height_mm", thread.length_mm),
        ("nominal_major_radius_mm", thread.nominal_major_radius_mm),
        ("major_radius_mm", thread.major_radius_mm),
        ("pitch_mm", thread.pitch_mm),
        (
            "signed_lead_mm",
            thread.lead_mm
                * if thread.hand == super::super::ThreadHand::Left {
                    -1.
                } else {
                    1.
                },
        ),
        ("depth_mm", thread.profile.depth_mm),
        ("crest_width_mm", thread.profile.crest_width_mm),
        ("lower_flank_deg", thread.profile.lower_flank_deg),
        ("upper_flank_deg", thread.profile.upper_flank_deg),
        ("radial_taper", thread.profile.radial_taper),
    ]);
    if let Some(engagement) = &thread.metric_fit_engagement
        && let Some(length) = engagement.intended_length_mm
    {
        values.insert("metric_fit_intended_length_mm", length);
    }
    values
}
fn engagement_kind(
    thread: &ThreadParameters,
) -> Option<std::mem::Discriminant<super::super::MetricFitEngagement>> {
    thread
        .metric_fit_engagement
        .as_ref()
        .map(std::mem::discriminant)
}
impl ResolvedMechanicalPart {
    fn reference_values(&self) -> BTreeMap<&'static str, f64> {
        match self {
            Self::WoodScrew { parameters: p } => {
                let mut values = BTreeMap::from([
                    ("underhead_length_mm", p.underhead_length_mm),
                    ("nominal_wood_diameter_mm", p.nominal_diameter_mm),
                    ("head_height_mm", p.head.height_mm()),
                    ("reference_pitch_mm", p.reference_profile.pitch_mm),
                    (
                        "reference_core_diameter_mm",
                        p.reference_profile.core_diameter_mm,
                    ),
                    (
                        "reference_included_angle_deg",
                        p.reference_profile.included_angle_deg,
                    ),
                    (
                        "minimum_tip_inclusive_thread_extent_mm",
                        p.minimum_tip_inclusive_thread_extent_mm,
                    ),
                ]);
                if let ResolvedScrewHead::ExternalHex {
                    across_flats_mm, ..
                } = &p.head
                {
                    values.insert("head_across_flats_mm", *across_flats_mm);
                }
                values
            }
            Self::MachineScrew {
                underhead_length_mm,
                shank_radius_mm,
                head,
                drive,
                thread,
                ..
            } => {
                let mut values = thread_reference_values(thread);
                values.insert("underhead_length_mm", *underhead_length_mm);
                values.insert("shank_radius_mm", *shank_radius_mm);
                values.insert("head_height_mm", head.height_mm());
                match head {
                    ResolvedScrewHead::SocketCap { radius_mm, .. }
                    | ResolvedScrewHead::Pan { radius_mm, .. }
                    | ResolvedScrewHead::Button { radius_mm, .. }
                    | ResolvedScrewHead::Countersunk { radius_mm, .. } => {
                        values.insert("head_radius_mm", *radius_mm);
                    }
                    ResolvedScrewHead::ExternalHex {
                        across_flats_mm, ..
                    } => {
                        values.insert("head_across_flats_mm", *across_flats_mm);
                    }
                }
                if let ResolvedScrewDrive::HexSocket {
                    across_flats_mm,
                    depth_mm,
                } = drive
                {
                    values.insert("socket_across_flats_mm", *across_flats_mm);
                    values.insert("socket_depth_mm", *depth_mm);
                }
                values.extend(head.additional_profile_values());
                values.extend(drive.additional_profile_values());
                values
            }
            Self::Washer {
                bore_radius_mm,
                outside_radius_mm,
                thickness_mm,
                ..
            } => BTreeMap::from([
                ("bore_radius_mm", *bore_radius_mm),
                ("outside_radius_mm", *outside_radius_mm),
                ("height_mm", *thickness_mm),
            ]),
            Self::BearingEnvelope {
                bore_radius_mm,
                outside_radius_mm,
                width_mm,
                ..
            } => BTreeMap::from([
                ("bore_radius_mm", *bore_radius_mm),
                ("outside_radius_mm", *outside_radius_mm),
                ("height_mm", *width_mm),
            ]),
            Self::HexNut {
                across_flats_mm,
                thread,
            } => {
                let mut values = thread_reference_values(thread);
                values.insert("across_flats_mm", *across_flats_mm);
                values
            }
        }
    }
}
impl MechanicalPartDefinition {
    /// Compare physical/profile dimensions to the retained literal reference.
    /// Placement and cosmetic/modeled display choice do not change the basis.
    /// A match does not verify the reference, availability, fit or standard.
    pub fn matches_reference_dimensions(
        &self,
        evaluated: &EvaluatedParameters,
    ) -> Result<bool, DocumentError> {
        self.validate()?;
        let Some(reference) = &self.provenance else {
            return Ok(false);
        };
        if let (
            MechanicalPartParameters::WoodScrew { parameters: a },
            MechanicalPartParameters::WoodScrew { parameters: b },
        ) = (&self.parameters, &reference.reference_geometry)
            && (a.point != b.point
                || a.representation != b.representation
                || std::mem::discriminant(&a.head) != std::mem::discriminant(&b.head)
                || std::mem::discriminant(&a.drive) != std::mem::discriminant(&b.drive))
        {
            return Ok(false);
        }
        if let (
            MechanicalPartParameters::HexNut { thread: a, .. },
            MechanicalPartParameters::HexNut { thread: b, .. },
        ) = (&self.parameters, &reference.reference_geometry)
            && (a.family != b.family
                || a.fit != b.fit
                || engagement_kind(a) != engagement_kind(b)
                || a.runout != b.runout
                || a.end_treatments != b.end_treatments
                || a.pipe != b.pipe
                || a.npt != b.npt
                || a.buttress != b.buttress)
        {
            return Ok(false);
        }
        if let (
            MechanicalPartParameters::MachineScrew {
                head: a,
                drive: ad,
                thread: at,
                ..
            },
            MechanicalPartParameters::MachineScrew {
                head: b,
                drive: bd,
                thread: bt,
                ..
            },
        ) = (&self.parameters, &reference.reference_geometry)
            && (std::mem::discriminant(a) != std::mem::discriminant(b)
                || std::mem::discriminant(ad) != std::mem::discriminant(bd)
                || at.family != bt.family
                || at.fit != bt.fit
                || engagement_kind(at) != engagement_kind(bt)
                || at.pipe != bt.pipe
                || at.npt != bt.npt
                || at.buttress != bt.buttress
                || at.runout != bt.runout
                || at.end_treatments != bt.end_treatments)
        {
            return Ok(false);
        }
        let actual = self.parameters.resolve(evaluated)?.reference_values();
        let expected = reference
            .reference_geometry
            .resolve(evaluated)?
            .reference_values();
        Ok(actual.len() == expected.len()
            && actual.iter().all(|(key, a)| {
                expected
                    .get(key)
                    .is_some_and(|b| (a - b).abs() <= 1e-9 + a.abs().max(b.abs()) * 1e-10)
            }))
    }
}
