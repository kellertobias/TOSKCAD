//! Dimensional attribution is separate from catalog trust and stock availability.
use super::*;
use std::collections::BTreeMap;
fn thread_reference_values(thread: &ResolvedThread) -> BTreeMap<&'static str, f64> {
    BTreeMap::from([
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
    ])
}
impl ResolvedMechanicalPart {
    fn reference_values(&self) -> BTreeMap<&'static str, f64> {
        match self {
            Self::MachineScrew {
                underhead_length_mm,
                shank_radius_mm,
                head,
                drive,
                thread,
            } => {
                let mut values = thread_reference_values(thread);
                values.insert("underhead_length_mm", *underhead_length_mm);
                values.insert("shank_radius_mm", *shank_radius_mm);
                values.insert("head_height_mm", head.height_mm());
                match head {
                    ResolvedScrewHead::SocketCap { radius_mm, .. } => {
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
            MechanicalPartParameters::HexNut { thread: a, .. },
            MechanicalPartParameters::HexNut { thread: b, .. },
        ) = (&self.parameters, &reference.reference_geometry)
            && (a.family != b.family
                || a.fit != b.fit
                || a.runout != b.runout
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
                || at.pipe != bt.pipe
                || at.npt != bt.npt
                || at.buttress != bt.buttress
                || at.runout != bt.runout)
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
