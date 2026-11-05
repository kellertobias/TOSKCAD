//! Straight basic-rack dimensions and explicitly custom circular root fillets.
use super::*;

pub const RACK_GEAR_CAPABILITY: &str = "model.gear.rack.1";
pub const RACK_PROFILE_SOURCE: &str = "KHK Basic Guide B, printed pages 6–7 and 14–15: straight rack, p=pi*m, ha=m, hf=1.25m; custom circular root fillet and finite pitch-cell ends";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackDimensions {
    pub backing_thickness: Dimension,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRack {
    pub teeth: u32,
    pub module_mm: f64,
    pub pitch_mm: f64,
    pub pressure_angle_rad: f64,
    pub length_mm: f64,
    pub face_width_mm: f64,
    pub addendum_mm: f64,
    pub dedendum_mm: f64,
    pub backing_thickness_mm: f64,
    pub root_fillet_radius_mm: f64,
    pub nominal_tooth_thickness_mm: f64,
    pub design_tooth_thickness_mm: f64,
    pub physical_tooth_thickness_mm: f64,
    pub tip_width_mm: f64,
    pub root_flat_width_mm: f64,
    /// Positive-X root fillet center relative to one tooth center.
    pub fillet_center_x_mm: f64,
    pub fillet_center_y_mm: f64,
    pub flank_tangent_y_mm: f64,
    pub frame: Frame,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedGearGeometry {
    Rotary(ResolvedGear),
    Rack(ResolvedRack),
    Bevel(ResolvedBevel),
}
impl ResolvedGearGeometry {
    pub fn frame(&self) -> Frame {
        match self {
            Self::Rotary(r) => r.frame,
            Self::Rack(r) => r.frame,
            Self::Bevel(r) => r.frame,
        }
    }
    pub fn teeth(&self) -> u32 {
        match self {
            Self::Rotary(r) => r.teeth,
            Self::Rack(r) => r.teeth,
            Self::Bevel(r) => r.teeth,
        }
    }
    pub fn family(&self) -> GearFamily {
        match self {
            Self::Rotary(r) => r.family,
            Self::Rack(_) => GearFamily::Rack,
            Self::Bevel(_) => GearFamily::Bevel,
        }
    }
    pub fn rotary(&self) -> Option<&ResolvedGear> {
        match self {
            Self::Rotary(r) => Some(r),
            Self::Rack(_) | Self::Bevel(_) => None,
        }
    }
    pub fn bevel(&self) -> Option<&ResolvedBevel> {
        match self {
            Self::Bevel(r) => Some(r),
            _ => None,
        }
    }
    pub fn rack(&self) -> Option<&ResolvedRack> {
        match self {
            Self::Rack(r) => Some(r),
            Self::Rotary(_) | Self::Bevel(_) => None,
        }
    }
}
impl GearParameters {
    pub fn resolve_geometry(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedGearGeometry, DocumentError> {
        if self.family == GearFamily::Bevel {
            return self
                .resolve_bevel(parameters)
                .map(ResolvedGearGeometry::Bevel);
        }
        if self.family != GearFamily::Rack {
            return self.resolve(parameters).map(ResolvedGearGeometry::Rotary);
        }
        self.validate()?;
        let module = number(&self.module, Unit::Millimeter, parameters)?;
        let count = number(&self.teeth, Unit::Count, parameters)?;
        let pressure = number(&self.pressure_angle, Unit::Radian, parameters)?;
        let width = number(&self.face_width, Unit::Millimeter, parameters)?;
        let backlash = number(&self.backlash, Unit::Millimeter, parameters)?;
        let allowance = number(&self.fabrication_allowance, Unit::Millimeter, parameters)?;
        let fillet = number(&self.root_fillet_ratio, Unit::Ratio, parameters)?;
        let wall = number(&self.minimum_wall, Unit::Millimeter, parameters)?;
        let backing = number(
            &self
                .rack
                .as_ref()
                .expect("validated rack dimensions")
                .backing_thickness,
            Unit::Millimeter,
            parameters,
        )?;
        if module < 0.01
            || !(4. ..=64.).contains(&count)
            || count.fract().abs() > 1e-9
            || !(14.5_f64.to_radians()..=35_f64.to_radians()).contains(&pressure)
            || width <= 0.
            || backlash < 0.
            || allowance < 0.
            || wall <= 0.
            || backing < wall
            || !(0.01..=0.2).contains(&fillet)
        {
            return Err(invalid(
                "Rack needs module ≥0.01 mm, 4–64 whole pitch cells, 14.5–35 degree pressure angle, positive width/wall, backing at least minimum wall, nonnegative thinning and custom root-fillet ratio 0.01–0.2",
            ));
        }
        if self.hand != GearHand::None
            || number(&self.helix_angle, Unit::Radian, parameters)? != 0.
            || number(&self.center_gap, Unit::Millimeter, parameters)? != 0.
            || number(&self.bore_diameter, Unit::Millimeter, parameters)? != 0.
            || number(&self.profile_shift, Unit::Ratio, parameters)? != 0.
        {
            return Err(invalid(
                "Straight racks need zero helix, hand none, zero center gap, bore and profile shift",
            ));
        }
        let pitch = PI * module;
        let nominal = pitch / 2.;
        let design = nominal - backlash;
        let physical = design - allowance;
        let addendum = module;
        let dedendum = 1.25 * module;
        let radius = fillet * module;
        // Outward normal of x+y*tan(alpha)=physical/2 points into the gap.
        let cy = -dedendum + radius;
        let cx = physical / 2. + radius / pressure.cos() - cy * pressure.tan();
        let tangent_y = cy - radius * pressure.sin();
        let tip = physical - 2. * addendum * pressure.tan();
        let root_flat = pitch - 2. * cx;
        if physical <= 0. || tip <= 1e-5 || root_flat <= 1e-5 || tangent_y >= addendum {
            return Err(invalid(
                "Rack thinning and custom root fillet must leave positive tip/root flats and straight flanks",
            ));
        }
        Ok(ResolvedGearGeometry::Rack(ResolvedRack {
            teeth: count as u32,
            module_mm: module,
            pitch_mm: pitch,
            pressure_angle_rad: pressure,
            length_mm: count * pitch,
            face_width_mm: width,
            addendum_mm: addendum,
            dedendum_mm: dedendum,
            backing_thickness_mm: backing,
            root_fillet_radius_mm: radius,
            nominal_tooth_thickness_mm: nominal,
            design_tooth_thickness_mm: design,
            physical_tooth_thickness_mm: physical,
            tip_width_mm: tip,
            root_flat_width_mm: root_flat,
            fillet_center_x_mm: cx,
            fillet_center_y_mm: cy,
            flank_tangent_y_mm: tangent_y,
            frame: self.frame.resolve(parameters)?,
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedRackTravel {
    pub rack_member: GearPairMember,
    pub pinion_member: GearPairMember,
    pub min_translation_mm: f64,
    pub max_translation_mm: f64,
    pub min_drive_angle_rad: f64,
    pub max_drive_angle_rad: f64,
    pub translation_mm: f64,
    pub pinion_period_rad: f64,
}
impl GearPairParameters {
    pub(super) fn resolve_rack_pair(
        &self,
        gears: [ResolvedGearGeometry; 2],
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedGearPair, DocumentError> {
        let (rack_index, pinion_index) = match (&gears[0], &gears[1]) {
            (ResolvedGearGeometry::Rack(_), ResolvedGearGeometry::Rotary(p))
                if p.family == GearFamily::Spur =>
            {
                (0, 1)
            }
            (ResolvedGearGeometry::Rotary(p), ResolvedGearGeometry::Rack(_))
                if p.family == GearFamily::Spur =>
            {
                (1, 0)
            }
            _ => {
                return Err(invalid(
                    "A straight rack pair needs exactly one rack and one unshifted spur pinion",
                ));
            }
        };
        let rack = gears[rack_index].rack().expect("rack index");
        let pinion = gears[pinion_index].rotary().expect("pinion index");
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.);
        if !close(rack.module_mm, pinion.transverse_module_mm)
            || !close(
                rack.pressure_angle_rad,
                pinion.transverse_pressure_angle_rad,
            )
            || !close(rack.face_width_mm, pinion.face_width_mm)
            || pinion.teeth < pinion.minimum_unshifted_teeth
        {
            return Err(invalid(
                "Rack and pinion need matching module, pressure angle and face width, with an unshifted non-undercut spur pinion",
            ));
        }
        // Deliberately conservative finite travel: the full pinion tip circle's
        // horizontal projection remains within the complete rack pitch cells.
        let limit = rack.length_mm / 2. - pinion.tip_radius_mm;
        if limit < rack.pitch_mm / 2. {
            return Err(invalid(
                "Rack is too short for a complete pinion pitch cycle inside the conservative finite-end envelope",
            ));
        }
        let drive = number(&self.drive_angle, Unit::Radian, evaluated)?;
        let angle_limit = limit / pinion.pitch_radius_mm;
        // Exact derived endpoints are commonly authored in degrees. Permit
        // only conversion roundoff, then use the exact endpoint geometry.
        let roundoff = 8. * f64::EPSILON * angle_limit.max(1.);
        if drive < -angle_limit - roundoff || drive > angle_limit + roundoff {
            return Err(invalid(format!(
                "Rack drive exceeds finite end travel: ±{limit} mm (±{angle_limit} radians)"
            )));
        }
        let drive = drive.clamp(-angle_limit, angle_limit);
        let slide = pinion.pitch_radius_mm * drive;
        let pinion_phase = -PI / 2.
            - if rack.teeth % 2 == 1 {
                PI / f64::from(pinion.teeth)
            } else {
                0.
            }
            + drive;
        let mut phases = [0.; 2];
        phases[pinion_index] = pinion_phase;
        let base = self.frame.resolve(evaluated)?;
        let mating = std::array::from_fn(|index| {
            let (sin, cos) = phases[index].rem_euclid(2. * PI).sin_cos();
            crate::references::ReferenceFrame {
                origin_mm: std::array::from_fn(|i| {
                    base.origin[i]
                        + if index == rack_index {
                            base.u[i] * slide
                        } else {
                            base.v[i] * pinion.pitch_radius_mm
                        }
                }),
                u: std::array::from_fn(|i| base.u[i] * cos + base.v[i] * sin),
                v: std::array::from_fn(|i| -base.u[i] * sin + base.v[i] * cos),
            }
        });
        let travel = ResolvedRackTravel {
            rack_member: if rack_index == 0 {
                GearPairMember::A
            } else {
                GearPairMember::B
            },
            pinion_member: if pinion_index == 0 {
                GearPairMember::A
            } else {
                GearPairMember::B
            },
            min_translation_mm: -limit,
            max_translation_mm: limit,
            min_drive_angle_rad: -angle_limit,
            max_drive_angle_rad: angle_limit,
            translation_mm: slide,
            pinion_period_rad: 2. * PI / f64::from(pinion.teeth),
        };
        let center = pinion.pitch_radius_mm;
        Ok(ResolvedGearPair {
            gears,
            center_distance_mm: center,
            phases_rad: phases,
            mating,
            rack_travel: Some(travel),
            bevel: None,
        })
    }
}
