//! Configured screw profiles. Source tables and gauge acceptance are separate.
use super::*;
pub const SCREW_PROFILES_CAPABILITY: &str = "model.mechanical_part.screw_profiles.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrownedScrewHeadParameters {
    pub diameter: Dimension,
    pub height: Dimension,
    pub crown_radius: Dimension,
    pub top_land_diameter: Dimension,
    pub neck_radius: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountersunkScrewHeadParameters {
    pub diameter: Dimension,
    pub height: Dimension,
    pub included_angle: Dimension,
    pub edge_chamfer: Dimension,
    pub neck_radius: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HexalobularDriveParameters {
    pub major_diameter: Dimension,
    pub minor_diameter: Dimension,
    pub outer_lobe_radius: Dimension,
    pub depth: Dimension,
    pub bottom_angle: Dimension,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhillipsHSize {
    H0,
    H1,
    H2,
    H3,
    H4,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhillipsInnerForm {
    Rounded {
        radius: Dimension,
    },
    ShallowV {
        angle: Dimension,
        e_width: Dimension,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhillipsHParameters {
    pub size: PhillipsHSize,
    pub mouth_diameter: Dimension,
    pub mouth_radius: Dimension,
    pub b_width: Dimension,
    pub g_diameter: Dimension,
    pub tilt_angle: Dimension,
    pub outer_flank_angle: Dimension,
    pub floor_angle: Dimension,
    pub outer_corner_angle: Dimension,
    pub inner: PhillipsInnerForm,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedPhillipsInnerForm {
    Rounded { radius_mm: f64 },
    ShallowV { angle_deg: f64, e_width_mm: f64 },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPhillipsHParameters {
    pub size: PhillipsHSize,
    pub mouth_diameter_mm: f64,
    pub mouth_radius_mm: f64,
    pub b_width_mm: f64,
    pub g_diameter_mm: f64,
    pub tilt_angle_deg: f64,
    pub outer_flank_angle_deg: f64,
    pub floor_angle_deg: f64,
    pub outer_corner_angle_deg: f64,
    pub inner: ResolvedPhillipsInnerForm,
}
impl PhillipsHParameters {
    pub(super) fn fields(&self) -> Vec<(&Dimension, Unit)> {
        let mut fields = vec![
            (&self.mouth_diameter, Unit::Millimeter),
            (&self.mouth_radius, Unit::Millimeter),
            (&self.b_width, Unit::Millimeter),
            (&self.g_diameter, Unit::Millimeter),
            (&self.tilt_angle, Unit::Degree),
            (&self.outer_flank_angle, Unit::Degree),
            (&self.floor_angle, Unit::Degree),
            (&self.outer_corner_angle, Unit::Degree),
        ];
        match &self.inner {
            PhillipsInnerForm::Rounded { radius } => fields.push((radius, Unit::Millimeter)),
            PhillipsInnerForm::ShallowV { angle, e_width } => {
                fields.push((angle, Unit::Degree));
                fields.push((e_width, Unit::Millimeter));
            }
        }
        fields
    }
    pub(super) fn resolve(
        &self,
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedPhillipsHParameters, DocumentError> {
        let mm = |d| number(d, Unit::Millimeter, evaluated);
        let deg = |d| number(d, Unit::Degree, evaluated);
        let profile = ResolvedPhillipsHParameters {
            size: self.size,
            mouth_diameter_mm: mm(&self.mouth_diameter)?,
            mouth_radius_mm: mm(&self.mouth_radius)?,
            b_width_mm: mm(&self.b_width)?,
            g_diameter_mm: mm(&self.g_diameter)?,
            tilt_angle_deg: deg(&self.tilt_angle)?,
            outer_flank_angle_deg: deg(&self.outer_flank_angle)?,
            floor_angle_deg: deg(&self.floor_angle)?,
            outer_corner_angle_deg: deg(&self.outer_corner_angle)?,
            inner: match &self.inner {
                PhillipsInnerForm::Rounded { radius } => ResolvedPhillipsInnerForm::Rounded {
                    radius_mm: mm(radius)?,
                },
                PhillipsInnerForm::ShallowV { angle, e_width } => {
                    ResolvedPhillipsInnerForm::ShallowV {
                        angle_deg: deg(angle)?,
                        e_width_mm: mm(e_width)?,
                    }
                }
            },
        };
        profile.validate()?;
        Ok(profile)
    }
}
impl ResolvedPhillipsHParameters {
    fn validate(&self) -> Result<(), DocumentError> {
        let positive = |v: f64| v.is_finite() && v > 0.;
        let acute = |v: f64| positive(v) && v < 90.;
        if ![
            self.mouth_diameter_mm,
            self.mouth_radius_mm,
            self.b_width_mm,
            self.g_diameter_mm,
        ]
        .into_iter()
        .all(positive)
            || self.b_width_mm >= self.g_diameter_mm
            || self.g_diameter_mm >= self.mouth_diameter_mm
            || self.mouth_radius_mm >= self.mouth_diameter_mm / 2.
            || !acute(self.tilt_angle_deg)
            || !acute(self.outer_flank_angle_deg)
            || !acute(self.floor_angle_deg)
            || !(positive(self.outer_corner_angle_deg) && self.outer_corner_angle_deg < 180.)
        {
            return Err(invalid(
                "Phillips H needs positive lengths and valid inclined-section angles",
            ));
        }
        match (&self.inner, self.size) {
            (ResolvedPhillipsInnerForm::Rounded { radius_mm }, PhillipsHSize::H0)
                if positive(*radius_mm) && self.derived_e_width_mm() < self.b_width_mm => {}
            (
                ResolvedPhillipsInnerForm::ShallowV {
                    angle_deg,
                    e_width_mm,
                },
                size,
            ) if size != PhillipsHSize::H0
                && angle_deg.is_finite()
                && *angle_deg > 90.
                && *angle_deg < 180.
                && positive(*e_width_mm)
                && *e_width_mm < self.b_width_mm => {}
            _ => {
                return Err(invalid(
                    "Phillips H0 requires a positive rounded inner form; H1–H4 require a shallow V inner form",
                ));
            }
        }
        Ok(())
    }
}
impl ScrewHead {
    pub(super) fn validate_dimensions(&self) -> Result<(), DocumentError> {
        for d in self.dimensions() {
            let unit = if matches!(self,Self::Countersunk {profile} if std::ptr::eq(d,&profile.included_angle))
            {
                Unit::Degree
            } else {
                Unit::Millimeter
            };
            check_dimension(d, unit)?;
        }
        Ok(())
    }
}
impl ScrewDrive {
    pub(super) fn validate_dimensions(&self) -> Result<(), DocumentError> {
        if let Self::PhillipsH { profile } = self {
            for (d, u) in profile.fields() {
                check_dimension(d, u)?;
            }
            return Ok(());
        }
        for d in self.dimensions() {
            let unit = if matches!(self,Self::Hexalobular {profile} if std::ptr::eq(d,&profile.bottom_angle))
            {
                Unit::Degree
            } else {
                Unit::Millimeter
            };
            check_dimension(d, unit)?;
        }
        Ok(())
    }
}
impl ResolvedPhillipsHParameters {
    /// Width between the true tangent points in the normal C-C section.
    pub fn derived_e_width_mm(&self) -> f64 {
        match self.inner {
            ResolvedPhillipsInnerForm::Rounded { radius_mm } => {
                2. * radius_mm * (self.outer_corner_angle_deg.to_radians() / 2.).cos()
            }
            ResolvedPhillipsInnerForm::ShallowV { e_width_mm, .. } => e_width_mm,
        }
    }
    /// Retained mouth diameter is the theoretical sharp-support diameter.
    pub fn entry_radius_mm(&self) -> f64 {
        let a = self.outer_flank_angle_deg.to_radians();
        self.mouth_diameter_mm / 2. + self.mouth_radius_mm * (1. - a.sin()) / a.cos()
    }
    pub fn removed_depth_mm(&self) -> f64 {
        (self.mouth_diameter_mm - self.g_diameter_mm)
            / (2. * self.outer_flank_angle_deg.to_radians().tan())
            + self.g_diameter_mm / 2. * self.floor_angle_deg.to_radians().tan()
    }
    /// Exact radial wing extrema; corner prisms remove material within this envelope.
    pub fn radial_envelope_mm(&self, depth: f64) -> f64 {
        let a = self.outer_flank_angle_deg.to_radians();
        let c = self.floor_angle_deg.to_radians();
        let r = self.mouth_radius_mm;
        let join = r * (1. - a.sin());
        let tg = (self.mouth_diameter_mm - self.g_diameter_mm) / (2. * a.tan());
        if depth <= join {
            self.entry_radius_mm() - (r * r - (depth - r).powi(2)).max(0.).sqrt()
        } else if depth <= tg {
            self.mouth_diameter_mm / 2. - depth * a.tan()
        } else {
            (self.removed_depth_mm() - depth).max(0.) / c.tan()
        }
    }
}
impl ResolvedScrewDrive {
    pub fn entry_radius_mm(&self) -> f64 {
        match self {
            Self::ExternalHex => 0.,
            Self::HexSocket {
                across_flats_mm, ..
            } => across_flats_mm / 3_f64.sqrt(),
            Self::Hexalobular {
                major_radius_mm, ..
            } => *major_radius_mm,
            Self::PhillipsH { profile } => profile.entry_radius_mm(),
        }
    }
    pub fn removed_depth_mm(&self) -> f64 {
        match self {
            Self::ExternalHex => 0.,
            Self::HexSocket { depth_mm, .. } => *depth_mm,
            Self::Hexalobular {
                major_radius_mm,
                depth_mm,
                bottom_angle_deg,
                ..
            } => depth_mm + major_radius_mm / (bottom_angle_deg.to_radians() / 2.).tan(),
            Self::PhillipsH { profile } => profile.removed_depth_mm(),
        }
    }
    pub fn radial_envelope_mm(&self, depth: f64) -> f64 {
        match self {
            Self::ExternalHex => 0.,
            Self::HexSocket {
                across_flats_mm, ..
            } => across_flats_mm / 3_f64.sqrt(),
            Self::Hexalobular {
                major_radius_mm,
                depth_mm,
                bottom_angle_deg,
                ..
            } => {
                if depth <= *depth_mm {
                    *major_radius_mm
                } else {
                    (major_radius_mm
                        - (depth - depth_mm) * (bottom_angle_deg.to_radians() / 2.).tan())
                    .max(0.)
                }
            }
            Self::PhillipsH { profile } => profile.radial_envelope_mm(depth),
        }
    }
    fn depth_breaks(&self) -> Vec<f64> {
        match self {
            Self::Hexalobular { depth_mm, .. } => vec![*depth_mm],
            Self::PhillipsH { profile } => vec![
                profile.mouth_radius_mm * (1. - profile.outer_flank_angle_deg.to_radians().sin()),
                (profile.mouth_diameter_mm - profile.g_diameter_mm)
                    / (2. * profile.outer_flank_angle_deg.to_radians().tan()),
            ],
            _ => vec![],
        }
    }
}
impl ResolvedScrewHead {
    pub fn neck_radius_mm(&self) -> f64 {
        match self {
            Self::Pan { neck_radius_mm, .. }
            | Self::Button { neck_radius_mm, .. }
            | Self::Countersunk { neck_radius_mm, .. } => *neck_radius_mm,
            _ => 0.,
        }
    }
    pub fn top_land_radius_mm(&self) -> f64 {
        match self {
            Self::Pan {
                top_land_radius_mm, ..
            }
            | Self::Button {
                top_land_radius_mm, ..
            } => *top_land_radius_mm,
            Self::Countersunk {
                radius_mm,
                edge_chamfer_mm,
                ..
            } => radius_mm - edge_chamfer_mm,
            _ => self.minimum_radius_mm(),
        }
    }
    /// Radial material boundary in the head and its neck blend, z from neck datum.
    pub fn radial_envelope_mm(&self, z: f64, shank: f64) -> f64 {
        let neck = self.neck_radius_mm();
        if z >= 0. {
            let center_z = match self {
                Self::Countersunk {
                    included_angle_deg, ..
                } => neck * (included_angle_deg.to_radians() / 4.).tan(),
                _ => neck,
            };
            if neck > 0. && z < center_z {
                return shank + neck - (neck * neck - (z - center_z).powi(2)).max(0.).sqrt();
            }
            return shank;
        }
        match self {
            Self::Pan {
                radius_mm,
                height_mm,
                crown_radius_mm,
                top_land_radius_mm,
                ..
            }
            | Self::Button {
                radius_mm,
                height_mm,
                crown_radius_mm,
                top_land_radius_mm,
                ..
            } => {
                let center =
                    -height_mm + (crown_radius_mm.powi(2) - top_land_radius_mm.powi(2)).sqrt();
                let skirt = center - (crown_radius_mm.powi(2) - radius_mm.powi(2)).sqrt();
                if z < skirt {
                    (crown_radius_mm.powi(2) - (z - center).powi(2))
                        .max(0.)
                        .sqrt()
                } else {
                    *radius_mm
                }
            }
            Self::Countersunk {
                radius_mm,
                height_mm,
                included_angle_deg,
                edge_chamfer_mm,
                ..
            } => {
                let theta = included_angle_deg.to_radians() / 2.;
                let top = -height_mm;
                let unblended = radius_mm.min(shank - z * theta.tan());
                let center_z = neck * (theta / 2.).tan();
                let cone_z = center_z - neck * theta.sin();
                if neck > 0. && z > cone_z {
                    shank + neck - (neck * neck - (z - center_z).powi(2)).max(0.).sqrt()
                } else if z < top + edge_chamfer_mm {
                    (radius_mm - edge_chamfer_mm + (z - top)).min(unblended)
                } else {
                    unblended
                }
            }
            _ => self.minimum_radius_mm(),
        }
    }
}
/// Validate complete radial head/recess/neck envelopes before native construction.
/// This uses exact piecewise extrema and conservative monotone interval bounds;
/// native policy/validity and source dimensional checks remain additional gates.
pub fn validate_screw_profile_envelope(
    head: &ResolvedScrewHead,
    drive: &ResolvedScrewDrive,
    shank: f64,
    underhead: f64,
    thread: &ResolvedThread,
) -> Result<(), DocumentError> {
    let positive = |v: f64| v.is_finite() && v > 0.;
    let nonnegative = |v: f64| v.is_finite() && v >= 0.;
    let fail = || {
        invalid(
            "Screw profile needs valid tangent head/neck geometry and a recess with positive surrounding material and floor",
        )
    };
    let k = head.height_mm();
    let radius = head.minimum_radius_mm();
    let plain = underhead - thread.length_mm;
    let mut z_breaks = vec![-k, 0., plain, underhead];
    match head {
        ResolvedScrewHead::Pan {
            crown_radius_mm: c,
            top_land_radius_mm: l,
            neck_radius_mm: n,
            ..
        }
        | ResolvedScrewHead::Button {
            crown_radius_mm: c,
            top_land_radius_mm: l,
            neck_radius_mm: n,
            ..
        } => {
            if !positive(*c)
                || !positive(*l)
                || !nonnegative(*n)
                || *l >= radius
                || radius > *c
                || shank + n >= radius
                || *n > plain
            {
                return Err(fail());
            }
            let center = -k + (c * c - l * l).sqrt();
            let skirt = center - (c * c - radius * radius).sqrt();
            if !skirt.is_finite() || skirt <= -k || skirt >= 0. {
                return Err(fail());
            }
            z_breaks.extend([skirt, *n]);
        }
        ResolvedScrewHead::Countersunk {
            included_angle_deg,
            edge_chamfer_mm: e,
            neck_radius_mm: n,
            ..
        } => {
            if !positive(*included_angle_deg)
                || *included_angle_deg >= 180.
                || !nonnegative(*e)
                || !nonnegative(*n)
            {
                return Err(fail());
            }
            let theta = included_angle_deg.to_radians() / 2.;
            let rim = k - (radius - shank) / theta.tan();
            let center = n * (theta / 2.).tan();
            if !rim.is_finite() || rim < 0. || *e >= radius || *e > rim || center > plain {
                return Err(fail());
            }
            let cone_z = center - n * theta.sin();
            if cone_z < -k + rim {
                return Err(fail());
            }
            z_breaks.extend([-k + *e, -k + rim, cone_z, center]);
        }
        _ => {}
    }
    match drive {
        ResolvedScrewDrive::ExternalHex => return Ok(()),
        ResolvedScrewDrive::HexSocket {
            across_flats_mm,
            depth_mm,
        } => {
            if !positive(*across_flats_mm) || !positive(*depth_mm) {
                return Err(fail());
            }
        }
        ResolvedScrewDrive::Hexalobular {
            major_radius_mm,
            minor_radius_mm,
            outer_lobe_radius_mm,
            depth_mm,
            bottom_angle_deg,
        } => {
            if !positive(*depth_mm) || !positive(*bottom_angle_deg) || *bottom_angle_deg >= 180. {
                return Err(fail());
            }
            HexalobularContour::new(
                2. * major_radius_mm,
                2. * minor_radius_mm,
                *outer_lobe_radius_mm,
            )?;
        }
        ResolvedScrewDrive::PhillipsH { profile } => {
            profile.validate()?;
            let entry_join =
                profile.mouth_radius_mm * (1. - profile.outer_flank_angle_deg.to_radians().sin());
            let flank_join = (profile.mouth_diameter_mm - profile.g_diameter_mm)
                / (2. * profile.outer_flank_angle_deg.to_radians().tan());
            if entry_join >= flank_join {
                return Err(invalid(
                    "Phillips entry roundover must finish before the outer-flank/floor junction",
                ));
            }
        }
    }
    let depth = drive.removed_depth_mm();
    let entry = drive.entry_radius_mm();
    if !positive(depth)
        || !positive(entry)
        || depth >= k + underhead
        || entry >= head.top_land_radius_mm()
    {
        return Err(fail());
    }
    let body = |d: f64| {
        let z = d - k;
        if z >= plain {
            // The retained thread root/core is the conservative material bound;
            // crest phase cannot be used to claim a complete recess wall.
            let station = z - plain;
            let mut major = thread.major_radius_mm + thread.profile.radial_taper * station;
            if let Some(limit) = thread
                .npt
                .as_ref()
                .and_then(|p| p.stock_crest_radius_limit_mm)
            {
                major = major.min(limit);
            }
            major - thread.profile.depth_mm
        } else {
            head.radial_envelope_mm(z, shank)
        }
    };
    let mut breaks = vec![0., depth];
    breaks.extend(drive.depth_breaks());
    breaks.extend(z_breaks.into_iter().map(|z| z + k));
    breaks.retain(|d| d.is_finite() && *d >= 0. && *d <= depth);
    breaks.sort_by(f64::total_cmp);
    breaks.dedup();
    let mut pending: Vec<_> = breaks.windows(2).map(|w| (w[0], w[1], 0u8)).collect();
    let mut subdivisions = 0;
    while let Some((left, right, level)) = pending.pop() {
        let rl = body(left);
        let rr = body(right);
        let cut = drive.radial_envelope_mm(left);
        if !positive(rl) || !positive(rr) || !cut.is_finite() {
            return Err(fail());
        }
        // Each head piece is monotone, and every drive envelope decreases.
        if rl.min(rr) > cut {
            continue;
        }
        if rl <= cut
            || rr <= drive.radial_envelope_mm(right)
            || level >= 48
            || subdivisions >= 16384
        {
            return Err(fail());
        }
        let mid = left + (right - left) / 2.;
        if mid == left || mid == right {
            return Err(fail());
        }
        subdivisions += 1;
        pending.push((left, mid, level + 1));
        pending.push((mid, right, level + 1));
    }
    Ok(())
}
impl ResolvedScrewHead {
    pub(super) fn additional_profile_values(&self) -> Vec<(&'static str, f64)> {
        match self {
            Self::Pan {
                crown_radius_mm,
                top_land_radius_mm,
                neck_radius_mm,
                ..
            }
            | Self::Button {
                crown_radius_mm,
                top_land_radius_mm,
                neck_radius_mm,
                ..
            } => vec![
                ("Crown radius", *crown_radius_mm),
                ("Top land diameter", 2. * top_land_radius_mm),
                ("Neck radius", *neck_radius_mm),
            ],
            Self::Countersunk {
                included_angle_deg,
                edge_chamfer_mm,
                neck_radius_mm,
                ..
            } => vec![
                ("Head included angle (degrees)", *included_angle_deg),
                ("Edge chamfer", *edge_chamfer_mm),
                ("Neck radius", *neck_radius_mm),
            ],
            _ => vec![],
        }
    }
}
impl ResolvedScrewDrive {
    pub(super) fn additional_profile_values(&self) -> Vec<(&'static str, f64)> {
        match self {
            Self::Hexalobular {
                major_radius_mm,
                minor_radius_mm,
                outer_lobe_radius_mm,
                depth_mm,
                bottom_angle_deg,
            } => vec![
                ("Recess major diameter", 2. * major_radius_mm),
                ("Recess minor diameter", 2. * minor_radius_mm),
                ("Outer lobe radius", *outer_lobe_radius_mm),
                ("Functional recess depth", *depth_mm),
                ("Bottom included angle (degrees)", *bottom_angle_deg),
            ],
            Self::PhillipsH { profile } => {
                let p = profile.as_ref();
                let size = match p.size {
                    PhillipsHSize::H0 => 0.,
                    PhillipsHSize::H1 => 1.,
                    PhillipsHSize::H2 => 2.,
                    PhillipsHSize::H3 => 3.,
                    PhillipsHSize::H4 => 4.,
                };
                let mut values = vec![
                    ("Phillips H size", size),
                    ("Theoretical mouth diameter", p.mouth_diameter_mm),
                    ("Mouth roundover radius", p.mouth_radius_mm),
                    ("B width", p.b_width_mm),
                    ("G diameter", p.g_diameter_mm),
                    ("Tilt angle (degrees)", p.tilt_angle_deg),
                    ("Outer flank angle (degrees)", p.outer_flank_angle_deg),
                    ("Floor angle (degrees)", p.floor_angle_deg),
                    (
                        "Outer corner included angle (degrees)",
                        p.outer_corner_angle_deg,
                    ),
                ];
                match p.inner {
                    ResolvedPhillipsInnerForm::Rounded { radius_mm } => {
                        values.push(("Inner radius", radius_mm))
                    }
                    ResolvedPhillipsInnerForm::ShallowV {
                        angle_deg,
                        e_width_mm,
                    } => {
                        values.push(("Inner included angle (degrees)", angle_deg));
                        values.push(("E width", e_width_mm));
                    }
                }
                values
            }
            _ => vec![],
        }
    }
}
