//! Bounded ZA axial-straight worm and actual generating-envelope wheel.
//! Canonical source designation, matching and assembly poses belong to core.
use crate::{
    GeometryError, GeometryErrorKind, GeometryPolicy, HelicalThread, RigidTransform, Solid, ffi,
};
use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, AnchoredFrame, Origin, TopologyKey, TopologyKind},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::PI,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZaWorm {
    pub axial_module_mm: f64,
    pub threaded_length_mm: f64,
    pub bore_radius_mm: f64,
    pub axial_thinning_mm: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratedZaWormWheel {
    pub teeth: u32,
    pub axial_module_mm: f64,
    pub face_width_mm: f64,
    pub bore_radius_mm: f64,
    pub pitch_thinning_mm: f64,
}
fn failure(kind: GeometryErrorKind, message: impl ToString) -> GeometryError {
    GeometryError {
        operation: "ZA worm gearing",
        kind,
        message: message.to_string(),
    }
}
fn native(error: impl ToString) -> GeometryError {
    failure(GeometryErrorKind::Native, error)
}
fn invalid(message: impl ToString) -> GeometryError {
    failure(GeometryErrorKind::InvalidResult, message)
}
fn common(
    module: f64,
    thinning: f64,
    bore: f64,
    root: f64,
    policy: GeometryPolicy,
) -> Result<(), GeometryError> {
    policy.validate()?;
    if !module.is_finite()
        || !(0.5..=2.).contains(&module)
        || !thinning.is_finite()
        || !(0. ..=0.1 * module).contains(&thinning)
        || !bore.is_finite()
        || bore < 0.
    {
        return Err(failure(
            GeometryErrorKind::Input,
            "ZA gearing needs module 0.5–2 mm, finite nonnegative bore and thinning within 0–0.1 module",
        ));
    }
    policy.length(module)?;
    if bore > 0. {
        policy.length(bore)?;
    }
    if root - bore < policy.minimum_feature_mm {
        return Err(failure(
            GeometryErrorKind::Input,
            "Bore consumes the minimum root wall",
        ));
    }
    Ok(())
}
impl ZaWorm {
    fn checked(self, policy: GeometryPolicy) -> Result<ffi::ThreadData, GeometryError> {
        let m = self.axial_module_mm;
        common(
            m,
            self.axial_thinning_mm,
            self.bore_radius_mm,
            3.75 * m,
            policy,
        )?;
        if !self.threaded_length_mm.is_finite()
            || !(16. * m..=32. * m).contains(&self.threaded_length_mm)
        {
            return Err(failure(
                GeometryErrorKind::Limit,
                "ZA worm threaded length must be 16–32 axial modules",
            ));
        }
        let slope = 20_f64.to_radians().tan() * 1.01_f64.sqrt();
        let angle = slope.atan().to_degrees();
        HelicalThread {
            major_radius_mm: 6. * m,
            pitch_mm: PI * m,
            length_mm: self.threaded_length_mm,
            depth_mm: 2.25 * m,
            crest_width_mm: PI * m / 2. - self.axial_thinning_mm - 2. * m * slope,
            lower_flank_deg: angle,
            upper_flank_deg: angle,
            radial_taper: 0.,
            starts: 1,
            left_hand: false,
        }
        .checked(policy)
    }
}
impl GeneratedZaWormWheel {
    fn checked(self, policy: GeometryPolicy) -> Result<ffi::WormWheelData, GeometryError> {
        let m = self.axial_module_mm;
        if !(32..=64).contains(&self.teeth) {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Generated ZA wheel needs 32–64 teeth",
            ));
        }
        let pitch_radius = f64::from(self.teeth) * m / 2.;
        // Actual cutter tip rc=6.25m: minimum torus root radius at v=0.
        common(
            m,
            self.pitch_thinning_mm,
            self.bore_radius_mm,
            pitch_radius - 1.25 * m,
            policy,
        )?;
        if !self.face_width_mm.is_finite() || !(0.5 * m..=2. * m).contains(&self.face_width_mm) {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Generated ZA wheel width must be 0.5–2 axial modules",
            ));
        }
        policy.length(self.face_width_mm)?;
        policy.point([pitch_radius + m, pitch_radius + m, self.face_width_mm / 2.])?;
        policy.point([
            -pitch_radius - m,
            -pitch_radius - m,
            -self.face_width_mm / 2.,
        ])?;
        Ok(ffi::WormWheelData {
            teeth: self.teeth,
            module: m,
            width: self.face_width_mm,
            bore: self.bore_radius_mm,
            thinning: self.pitch_thinning_mm,
            tolerance: policy.linear_tolerance_mm,
            minimum: policy.minimum_feature_mm,
        })
    }
}
// One native face may legitimately inherit several source sectors after a
// Boolean merge; reject duplicate receipts, not distinct origins on one face.
impl Solid {
    fn name_za_faces(
        &mut self,
        feature: EntityId,
        wheel: Option<u32>,
        bore: bool,
    ) -> Result<(), GeometryError> {
        let mut elements: BTreeMap<_, _> = self
            .topology_snapshot(feature, 0)?
            .elements
            .into_iter()
            .map(|e| (e.key, e))
            .collect();
        let mut receipts = BTreeSet::new();
        let mut worm_mask = 0_u8;
        let mut wheel_masks = vec![0_u8; wheel.unwrap_or(0) as usize];
        let mut found_bore = false;
        for seed in ffi::seeds(self.occt()?).map_err(native)? {
            if !receipts.insert((seed.face, seed.role, seed.source_index)) {
                return Err(invalid("Duplicate native ZA face receipt"));
            }
            let role = if let Some(teeth) = wheel {
                match seed.role {
                    0 | 1 if seed.source_index < teeth => {
                        wheel_masks[seed.source_index as usize] |= 1 << seed.role;
                        format!(
                            "worm_wheel/sector/{}/end.{}",
                            seed.source_index,
                            if seed.role == 0 { "start" } else { "end" }
                        )
                    }
                    2 if bore && seed.source_index == 0 => {
                        found_bore = true;
                        "worm_wheel/bore".into()
                    }
                    6 if seed.source_index / 8 < teeth => {
                        let index = seed.source_index / 8;
                        let (name, bit) = match seed.source_index % 8 {
                            0 => ("flank.minus", 2),
                            1 => ("root.edge.minus", 3),
                            2 => ("root.torus", 4),
                            3 => ("root.edge.plus", 5),
                            4 => ("flank.plus", 6),
                            7 => ("tip", 7),
                            _ => return Err(invalid("Unknown generated-wheel side receipt")),
                        };
                        wheel_masks[index as usize] |= 1 << bit;
                        format!("worm_wheel/sector/{index}/{name}")
                    }
                    _ => return Err(invalid("Unknown generated-wheel face receipt")),
                }
            } else {
                match (seed.role, seed.source_index) {
                    (0, 0) => {
                        worm_mask |= 1;
                        "worm/end.start".into()
                    }
                    (1, 0) => {
                        worm_mask |= 2;
                        "worm/end.end".into()
                    }
                    (2, 0) => {
                        worm_mask |= 4;
                        "worm/flank.lower".into()
                    }
                    (2, 1) => {
                        worm_mask |= 8;
                        "worm/crest".into()
                    }
                    (2, 2) => {
                        worm_mask |= 16;
                        "worm/flank.upper".into()
                    }
                    (3, 0) => {
                        worm_mask |= 32;
                        "worm/root".into()
                    }
                    (5, 0) if bore => {
                        found_bore = true;
                        "worm/bore".into()
                    }
                    _ => return Err(invalid("Unknown ZA worm face receipt")),
                }
            };
            let key = TopologyKey {
                kind: TopologyKind::Face,
                ordinal: seed.face,
            };
            let element = elements
                .get_mut(&key)
                .ok_or_else(|| invalid("ZA ancestry names an absent face"))?;
            let origin = Origin {
                source: feature,
                role,
            };
            element.origins.insert(origin.clone());
            if let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                && !element.frames.iter().any(|known| known.origin == origin)
            {
                element.frames.push(AnchoredFrame { origin, frame });
            }
        }
        if found_bore != bore
            || (wheel.is_none() && worm_mask != 63)
            || wheel_masks.iter().any(|mask| *mask != 255)
            || elements
                .values()
                .any(|e| e.key.kind == TopologyKind::Face && e.origins.is_empty())
        {
            return Err(invalid(
                "ZA solid needs complete face ancestry, both caps, all bounded flank/root/tip roles and the requested bore",
            ));
        }
        let nodes = ffi::topology(self.occt()?).map_err(native)?;
        self.finish_provenance(elements, &nodes)
    }
    /// Right, single-start, q=10 axial-straight worm. Its axis is local +Z,
    /// ends are ±L/2, and its phase-zero crest faces -X at local Z=0.
    pub fn za_worm_named(
        spec: ZaWorm,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Worm feature identity must not be nil",
            ));
        }
        let data = spec.checked(policy)?;
        let mut source = Self::adopt(
            ffi::za_worm(&data, spec.bore_radius_mm).map_err(native)?,
            policy,
            "ZA worm",
        )?;
        if source.solid_count()? != 1 {
            return Err(invalid("ZA worm must retain one connected solid"));
        }
        // Native neutral receipts refer to the pretransform body. Assign only
        // worm roles first; the proper rigid transform carries face history.
        source.name_za_faces(feature, None, spec.bore_radius_mm > 0.)?;
        let phase = PI - PI * spec.threaded_length_mm / (PI * spec.axial_module_mm);
        let (sin, cos) = phase.sin_cos();
        source.transformed(RigidTransform {
            rotation: [[cos, -sin, 0.], [sin, cos, 0.], [0., 0., 1.]],
            translation_mm: [0., 0., -spec.threaded_length_mm / 2.],
        })
    }
    /// Generated wheel axis is local +Z; face width is centered at Z=0.
    /// The phase-zero tooth space is centered on local +X.
    pub fn generated_za_worm_wheel_named(
        spec: GeneratedZaWormWheel,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Wheel feature identity must not be nil",
            ));
        }
        let data = spec.checked(policy)?;
        let mut solid = Self::adopt(
            ffi::za_worm_wheel(&data).map_err(native)?,
            policy,
            "generated ZA worm wheel",
        )?;
        if solid.solid_count()? != 1 {
            return Err(invalid(
                "Generated ZA wheel must retain one connected solid",
            ));
        }
        solid.name_za_faces(feature, Some(spec.teeth), spec.bore_radius_mm > 0.)?;
        Ok(solid)
    }
}
