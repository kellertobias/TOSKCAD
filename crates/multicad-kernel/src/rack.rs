//! Straight-sided rack geometry with explicit engineering root circles.
//! Nominal series, gear compatibility and standards claims belong to core.
use crate::{
    GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile, ProfileEdge, Solid,
};
use multicad_core::{document::EntityId, references::Origin};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StraightRack {
    /// Bounded to 4..=64 cells to retain complete per-tooth provenance.
    pub teeth: u32,
    pub pitch_mm: f64,
    pub pitch_thickness_mm: f64,
    pub pressure_angle_rad: f64,
    pub addendum_mm: f64,
    pub dedendum_mm: f64,
    /// Tangent circular root transition, not a simulated cutter trochoid.
    pub root_fillet_radius_mm: f64,
    pub backing_thickness_mm: f64,
    pub face_width_mm: f64,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "straight rack",
        kind,
        message: message.into(),
    }
}
struct Section {
    half_length: f64,
    root_y: f64,
    bottom_y: f64,
    circle_x: f64,
    circle_y: f64,
    tangent_x: f64,
    tangent_y: f64,
    tip_x: f64,
}
impl StraightRack {
    fn checked(self, policy: GeometryPolicy) -> Result<Section, GeometryError> {
        policy.validate()?;
        if !(4..=64).contains(&self.teeth) {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Rack needs 4 to 64 whole pitch cells",
            ));
        }
        for value in [
            self.pitch_mm,
            self.pitch_thickness_mm,
            self.addendum_mm,
            self.dedendum_mm,
            self.root_fillet_radius_mm,
            self.backing_thickness_mm,
            self.face_width_mm,
        ] {
            policy.length(value)?;
        }
        if !self.pressure_angle_rad.is_finite()
            || self.pressure_angle_rad <= 0.
            || self.pressure_angle_rad >= std::f64::consts::FRAC_PI_2
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Rack pressure angle must be finite and strictly between zero and pi/2",
            ));
        }
        let (sin, cos) = self.pressure_angle_rad.sin_cos();
        let slope = self.pressure_angle_rad.tan();
        let circle_y = -self.dedendum_mm + self.root_fillet_radius_mm;
        let circle_x =
            self.pitch_thickness_mm / 2. + self.root_fillet_radius_mm / cos - circle_y * slope;
        let tangent_x = circle_x - self.root_fillet_radius_mm * cos;
        let tangent_y = circle_y - self.root_fillet_radius_mm * sin;
        let tip_x = self.pitch_thickness_mm / 2. - self.addendum_mm * slope;
        let minimum = policy.minimum_feature_mm;
        if ![circle_x, circle_y, tangent_x, tangent_y, tip_x]
            .iter()
            .all(|v| v.is_finite())
            || self.pitch_mm / 2. - circle_x < minimum
            || 2. * tip_x < minimum
            || tangent_y > -minimum
            || self.addendum_mm - tangent_y < minimum
            || self.root_fillet_radius_mm * (std::f64::consts::FRAC_PI_2 - self.pressure_angle_rad)
                < minimum
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Rack needs distinct tangent root circles, positive end/root flats, tooth tips and straight flanks crossing the pitch line",
            ));
        }
        let section = Section {
            half_length: f64::from(self.teeth) * self.pitch_mm / 2.,
            root_y: -self.dedendum_mm,
            bottom_y: -self.dedendum_mm - self.backing_thickness_mm,
            circle_x,
            circle_y,
            tangent_x,
            tangent_y,
            tip_x,
        };
        policy.point([-section.half_length, section.bottom_y, 0.])?;
        policy.point([section.half_length, self.addendum_mm, self.face_width_mm])?;
        Ok(section)
    }
}
struct Outline {
    edges: Vec<ProfileEdge>,
    origins: Vec<Origin>,
    feature: EntityId,
}
impl Outline {
    fn add(&mut self, edge: ProfileEdge, role: impl Into<String>) {
        self.edges.push(edge);
        self.origins.push(Origin {
            source: self.feature,
            role: role.into(),
        });
    }
    fn line(&mut self, a: [f64; 2], b: [f64; 2], role: impl Into<String>) {
        self.add(ProfileEdge::Line { a, b }, role);
    }
    fn arc(&mut self, a: [f64; 2], b: [f64; 2], center: [f64; 2], role: impl Into<String>) {
        self.add(
            ProfileEdge::Arc {
                center,
                a,
                b,
                clockwise: true,
            },
            role,
        );
    }
}
impl Solid {
    /// Native planar line/circle profile extruded over Z=0..face_width.
    /// X spans exactly whole centered pitch cells; pitch line is local Y=0.
    pub fn straight_rack_named(
        spec: StraightRack,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Rack feature identity must not be nil",
            ));
        }
        let s = spec.checked(policy)?;
        let mut profile = Outline {
            edges: Vec::with_capacity(spec.teeth as usize * 6 + 4),
            origins: Vec::with_capacity(spec.teeth as usize * 6 + 4),
            feature,
        };
        let left_bottom = [-s.half_length, s.bottom_y];
        let right_bottom = [s.half_length, s.bottom_y];
        let mut current = [s.half_length, s.root_y];
        profile.line(left_bottom, right_bottom, "rack/backing/bottom");
        profile.line(right_bottom, current, "rack/end/right");
        // Counterclockwise boundary: walk its upper edge from right to left.
        for tooth in (0..spec.teeth).rev() {
            let center = (f64::from(tooth) + 0.5 - f64::from(spec.teeth) / 2.) * spec.pitch_mm;
            let right_root = [center + s.circle_x, s.root_y];
            let right_tangent = [center + s.tangent_x, s.tangent_y];
            let right_tip = [center + s.tip_x, spec.addendum_mm];
            let left_tip = [center - s.tip_x, spec.addendum_mm];
            let left_tangent = [center - s.tangent_x, s.tangent_y];
            let left_root = [center - s.circle_x, s.root_y];
            profile.line(current, right_root, format!("rack/space/{tooth}"));
            profile.arc(
                right_root,
                right_tangent,
                [center + s.circle_x, s.circle_y],
                format!("rack/tooth/{tooth}/fillet/right"),
            );
            profile.line(
                right_tangent,
                right_tip,
                format!("rack/tooth/{tooth}/flank/right"),
            );
            profile.line(right_tip, left_tip, format!("rack/tooth/{tooth}/tip"));
            profile.line(
                left_tip,
                left_tangent,
                format!("rack/tooth/{tooth}/flank/left"),
            );
            profile.arc(
                left_tangent,
                left_root,
                [center - s.circle_x, s.circle_y],
                format!("rack/tooth/{tooth}/fillet/left"),
            );
            current = left_root;
        }
        profile.line(current, [-s.half_length, s.root_y], "rack/space/left_end");
        profile.line([-s.half_length, s.root_y], left_bottom, "rack/end/left");
        let solid = Self::extrude_profile_named(
            &Profile {
                frame: PlaneFrame::default(),
                outer: profile.edges,
                holes: vec![],
            },
            spec.face_width_mm,
            feature,
            &profile.origins,
            policy,
        )?;
        if solid.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Rack must retain one connected backing solid",
            ));
        }
        Ok(solid)
    }
}
