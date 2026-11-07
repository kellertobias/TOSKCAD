//! Dimensional attribution is separate from catalog trust and stock availability.
use super::*;
use std::collections::BTreeMap;

/// The basis of the current authored part, not a purchasing or fit claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanicalAttributionBasis {
    Authored,
    ReferenceDimensions,
    CustomDerived,
}

impl MechanicalAttributionBasis {
    pub fn label(self) -> &'static str {
        match self {
            Self::Authored => "Authored hardware",
            Self::ReferenceDimensions => "Reference dimensions",
            Self::CustomDerived => "Custom / derived",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MechanicalCatalogAttribution {
    pub catalog_id: String,
    pub revision: String,
    pub designation: String,
    pub source_url: String,
    pub material: Option<String>,
    pub grade: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MechanicalPartAttribution {
    pub basis: MechanicalAttributionBasis,
    pub reference: Option<MechanicalCatalogAttribution>,
    pub actual_material: Option<String>,
    pub template_material: Option<String>,
    /// `None` means there is no retained catalog geometry to compare.
    pub dimensions_match: Option<bool>,
    /// A label comparison when both the assigned and retained materials are
    /// known. It does not verify composition, grade or physical equivalence.
    pub material_match: Option<bool>,
}
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
            Self::CustomConnection { .. } => BTreeMap::new(),
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
    /// Describe current source values alongside the retained reference. The
    /// caller resolves the actual and pinned-template material assignments
    /// independently. Catalog text never supplies the actual material.
    pub fn attribution(
        &self,
        evaluated: &EvaluatedParameters,
        actual_material: Option<&str>,
        template_material: Option<&str>,
    ) -> Result<MechanicalPartAttribution, DocumentError> {
        let dimensions_match = self
            .provenance
            .as_ref()
            .map(|_| self.matches_reference_dimensions(evaluated))
            .transpose()?;
        let reference = self
            .provenance
            .as_ref()
            .map(|source| MechanicalCatalogAttribution {
                catalog_id: source.catalog_id.clone(),
                revision: source.revision.clone(),
                designation: source.designation.clone(),
                source_url: source.source_url.clone(),
                material: source
                    .catalog_facts
                    .as_ref()
                    .and_then(|facts| facts.material.clone()),
                grade: source
                    .catalog_facts
                    .as_ref()
                    .and_then(|facts| facts.grade.clone()),
            });
        let retained_material = reference
            .as_ref()
            .and_then(|reference| reference.material.as_deref())
            .or(template_material);
        let material_match = actual_material
            .zip(retained_material)
            .map(|(actual, retained)| actual.trim().eq_ignore_ascii_case(retained.trim()));
        let assigned_outside_reference = actual_material.is_some() && retained_material.is_none();
        let basis = if dimensions_match == Some(false)
            || material_match == Some(false)
            || (reference.is_some() && assigned_outside_reference)
        {
            MechanicalAttributionBasis::CustomDerived
        } else if dimensions_match == Some(true) {
            MechanicalAttributionBasis::ReferenceDimensions
        } else {
            MechanicalAttributionBasis::Authored
        };
        Ok(MechanicalPartAttribution {
            basis,
            reference,
            actual_material: actual_material.map(str::to_owned),
            template_material: template_material.map(str::to_owned),
            dimensions_match,
            material_match,
        })
    }

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
