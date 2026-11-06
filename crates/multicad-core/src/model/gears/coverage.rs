//! Retained pitch convention, intrinsic hubs and KHK shifted external spur profiles.
use super::*;
pub const GEAR_COVERAGE_CAPABILITY: &str = "model.gear.coverage.1";
pub const TEETH_PER_INCH_CAPABILITY: &str = "parameters.teeth_per_inch.1";
pub const SHIFTED_SPUR_SOURCE_URL: &str = "https://khkgears.us/media/izojpong/gear_guide1.pdf";
pub const SHIFTED_SPUR_SOURCE: &str = "KHK Gear Guide, printed 604 Table 3.5 and 608 Table 4.3; custom circular root fillets, not generated rack trochoids. External 20-degree Spur only; coefficient -0.5..1 is an engineering coverage bound, not a standard limit.";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearPitchInput {
    #[default]
    Module,
    DiametralPitch,
}
impl GearPitchInput {
    pub fn is_module(&self) -> bool {
        *self == Self::Module
    }
    pub fn unit(self) -> Unit {
        match self {
            Self::Module => Unit::Millimeter,
            Self::DiametralPitch => Unit::TeethPerInch,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearHub {
    pub diameter: Dimension,
    pub axial_start: Dimension,
    pub axial_end: Dimension,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGearHub {
    pub radius_mm: f64,
    pub axial_start_mm: f64,
    pub axial_end_mm: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShiftedSpurMate {
    pub teeth: Dimension,
    pub profile_shift: Dimension,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedShiftedSpurPair {
    pub working_pressure_angle_rad: f64,
    pub center_distance_mm: f64,
    pub tip_shortening_coefficient: f64,
    pub contact_ratio: f64,
}
fn literal_zero(d: &Dimension) -> bool {
    matches!(d,Dimension::Literal{value,..} if *value == 0.)
}
pub fn couple_shifted_spur_pair_recipes(
    recipes: &mut [GearParameters; 2],
) -> Result<(), DocumentError> {
    if recipes
        .iter()
        .all(|p| p.shift_mate.is_none() && literal_zero(&p.profile_shift))
    {
        return Ok(());
    }
    if recipes.iter().any(|p| p.family != GearFamily::Spur) {
        return Err(invalid(
            "Shifted matched profiles require two external Spur members",
        ));
    }
    for i in 0..2 {
        if let Some(mate) = &recipes[i].shift_mate
            && (mate.teeth != recipes[1 - i].teeth
                || mate.profile_shift != recipes[1 - i].profile_shift)
        {
            return Err(invalid(
                "Explicit shifted mate sources disagree with the opposite original typed recipe",
            ));
        }
    }
    let sources = recipes.clone();
    for i in 0..2 {
        recipes[i].shift_mate = Some(ShiftedSpurMate {
            teeth: sources[1 - i].teeth.clone(),
            profile_shift: sources[1 - i].profile_shift.clone(),
        });
    }
    Ok(())
}
pub(super) fn validate_shifted_pair_sources(
    recipes: [&GearParameters; 2],
) -> Result<(), DocumentError> {
    if recipes.iter().any(|p| p.shift_mate.is_some()) {
        for i in 0..2 {
            let mate = recipes[i]
                .shift_mate
                .as_ref()
                .ok_or_else(|| invalid("Both matched shifted profiles must retain mate sources"))?;
            if recipes[i].family != GearFamily::Spur
                || mate.teeth != recipes[1 - i].teeth
                || mate.profile_shift != recipes[1 - i].profile_shift
            {
                return Err(invalid(
                    "Shifted pair mate sources disagree; edit both recipes atomically",
                ));
            }
        }
    }
    Ok(())
}
fn operating(z: f64, sum: f64, alpha: f64) -> Result<(f64, f64, f64), DocumentError> {
    let target = alpha.tan() - alpha + 2. * sum * alpha.tan() / z;
    if target <= 0. || !target.is_finite() {
        return Err(invalid(
            "Shift sum has no positive acute working pressure angle",
        ));
    }
    let (mut lo, mut hi) = (0., std::f64::consts::FRAC_PI_2 - 1e-6);
    for _ in 0..80 {
        let mid = (lo + hi) / 2.;
        if mid.tan() - mid < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let aw = (lo + hi) / 2.;
    let y = z / 2. * (alpha.cos() / aw.cos() - 1.);
    Ok((aw, y, sum - y))
}
impl GearParameters {
    pub fn resolved_module_mm(&self, p: &EvaluatedParameters) -> Result<f64, DocumentError> {
        let v = number(&self.module, self.pitch_input.unit(), p)?;
        if v <= 0. {
            return Err(invalid("Selected pitch value must be positive"));
        }
        Ok(if self.pitch_input == GearPitchInput::DiametralPitch {
            25.4 / v
        } else {
            v
        })
    }
    pub(super) fn validate_coverage(&self) -> Result<(), DocumentError> {
        let external = matches!(
            self.family,
            GearFamily::Spur
                | GearFamily::Helical
                | GearFamily::Herringbone
                | GearFamily::DoubleHelical
        );
        if !external && (self.hub.is_some() || self.pitch_input != GearPitchInput::Module) {
            return Err(invalid(
                "Intrinsic hubs and diametral pitch currently require external cylindrical gears",
            ));
        }
        if self.shift_mate.is_some() && self.family != GearFamily::Spur {
            return Err(invalid(
                "Matched shift context belongs only to external Spur",
            ));
        }
        Ok(())
    }
    pub(super) fn shift_shortening(
        &self,
        p: &EvaluatedParameters,
        z: f64,
        x: f64,
        alpha: f64,
    ) -> Result<f64, DocumentError> {
        let Some(mate) = &self.shift_mate else {
            return Ok(0.);
        };
        if (alpha - 20_f64.to_radians()).abs() > 1e-12 {
            return Err(invalid("Matched shifted Spur requires 20-degree pressure"));
        }
        let zm = number(&mate.teeth, Unit::Count, p)?;
        let xm = number(&mate.profile_shift, Unit::Ratio, p)?;
        if !(4. ..=128.).contains(&zm) || zm.fract().abs() > 1e-9 || !(-0.5..=1.).contains(&xm) {
            return Err(invalid(
                "Shift mate needs whole 4–128 teeth and shift -0.5..1",
            ));
        }
        Ok(operating(z + zm, x + xm, alpha)?.2)
    }
    pub(super) fn resolve_hub(
        &self,
        p: &EvaluatedParameters,
        root: f64,
        bore: f64,
        wall: f64,
        width: f64,
    ) -> Result<Option<ResolvedGearHub>, DocumentError> {
        let Some(h) = &self.hub else {
            return Ok(None);
        };
        let radius = number(&h.diameter, Unit::Millimeter, p)? / 2.;
        let start = number(&h.axial_start, Unit::Millimeter, p)?;
        let end = number(&h.axial_end, Unit::Millimeter, p)?;
        if radius <= 0.
            || radius > root - 0.01
            || radius - bore < wall
            || end <= start
            || end.min(width) - start.max(0.) < 0.01
        {
            return Err(invalid(
                "Hub requires positive axial extent, at least 0.01 mm overlap, radius below root by 0.01 mm, and minimum bore wall",
            ));
        }
        Ok(Some(ResolvedGearHub {
            radius_mm: radius,
            axial_start_mm: start,
            axial_end_mm: end,
        }))
    }
}
fn flank_start(g: &ResolvedGear) -> f64 {
    let foot = (g.root_radius_mm.powi(2) + 2. * g.root_radius_mm * g.root_fillet_radius_mm).sqrt();
    if foot <= g.base_radius_mm {
        0.
    } else {
        ((g.root_radius_mm + g.root_fillet_radius_mm).powi(2) - g.base_radius_mm.powi(2)).sqrt()
            / g.base_radius_mm
            - g.root_fillet_radius_mm / g.base_radius_mm
    }
}
pub fn shifted_spur_pair_geometry(
    a: &ResolvedGear,
    b: &ResolvedGear,
) -> Result<ResolvedShiftedSpurPair, DocumentError> {
    if a.family != GearFamily::Spur || b.family != GearFamily::Spur {
        return Err(invalid("Shifted pair geometry requires external Spur"));
    }
    let m = a.transverse_module_mm;
    let z = f64::from(a.teeth + b.teeth);
    let alpha = a.transverse_pressure_angle_rad;
    let (aw, y, delta) = operating(
        z,
        a.profile_shift_coefficient + b.profile_shift_coefficient,
        alpha,
    )?;
    let center = m * (z / 2. + y);
    let rw1 = center * f64::from(a.teeth) / z;
    let rw2 = center * f64::from(b.teeth) / z;
    let q1 = (a.tip_radius_mm.powi(2) - a.base_radius_mm.powi(2)).sqrt();
    let q2 = (b.tip_radius_mm.powi(2) - b.base_radius_mm.powi(2)).sqrt();
    let approach = q2 - rw2 * aw.sin();
    let recess = q1 - rw1 * aw.sin();
    let contact = (approach + recess) / (PI * m * alpha.cos());
    if !contact.is_finite()
        || contact <= 1.
        || approach <= 0.
        || recess <= 0.
        || rw1 * aw.sin() - approach < a.base_radius_mm * flank_start(a)
        || rw2 * aw.sin() - recess < b.base_radius_mm * flank_start(b)
    {
        return Err(invalid(
            "Shifted pair needs contact ratio >1 and full active contact above both actual circular-fillet joins",
        ));
    }
    Ok(ResolvedShiftedSpurPair {
        working_pressure_angle_rad: aw,
        center_distance_mm: center,
        tip_shortening_coefficient: delta,
        contact_ratio: contact,
    })
}

pub(super) fn validate_shifted_profile(g: &ResolvedGear) -> Result<(), DocumentError> {
    let root = g.root_radius_mm;
    let f = g.root_fillet_radius_mm;
    let b = g.base_radius_mm;
    let foot = (root * root + 2. * root * f).sqrt();
    let t0 = flank_start(g);
    let ta = (g.tip_radius_mm.powi(2) / b.powi(2) - 1.).sqrt();
    let root_half = if foot < b {
        g.half_base_angle_rad + (f / (root + f)).asin()
    } else {
        let angle = t0 - g.half_base_angle_rad;
        let px = b * (t0.cos() + t0 * t0.sin()) * g.half_base_angle_rad.cos()
            + b * (t0.sin() - t0 * t0.cos()) * g.half_base_angle_rad.sin();
        let py = -b * (t0.cos() + t0 * t0.sin()) * g.half_base_angle_rad.sin()
            + b * (t0.sin() - t0 * t0.cos()) * g.half_base_angle_rad.cos();
        -(py - f * (angle + PI / 2.).sin()).atan2(px - f * (angle + PI / 2.).cos())
    };
    if !t0.is_finite()
        || !root_half.is_finite()
        || t0 < 0.
        || t0 >= ta
        || root_half <= 0.
        || root_half >= PI / f64::from(g.teeth) - 1e-5 / root
    {
        return Err(invalid(
            "Shifted custom circular root leaves no separated root gap or complete involute flank",
        ));
    }
    Ok(())
}
