use crate::camera::Camera;
use eframe::egui::Rect;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DisplayQuality {
    Distant,
    Coarse,
    Normal,
    Fine,
    Detail,
    Precision,
}
impl DisplayQuality {
    pub const ALL: [Self; 6] = [
        Self::Distant,
        Self::Coarse,
        Self::Normal,
        Self::Fine,
        Self::Detail,
        Self::Precision,
    ];
    pub fn deflection_mm(self) -> f64 {
        match self {
            Self::Distant => 2.,
            Self::Coarse => 0.5,
            Self::Normal => 0.1,
            Self::Fine => 0.02,
            Self::Detail => 0.005,
            Self::Precision => 0.001,
        }
    }
    /// Aim for less than one logical pixel of curve deviation. The kernel's
    /// numerical tolerance and exact geometry are never changed by this choice.
    pub fn for_view(camera: Camera, rect: Rect, bounds: [[f64; 3]; 2]) -> Option<Self> {
        if !camera.scale.is_finite()
            || camera.scale <= 0.
            || !rect.is_finite()
            || !rect.is_positive()
            || !camera.yaw.is_finite()
            || !camera.pitch.is_finite()
            || camera.target.iter().any(|v| !v.is_finite())
            || bounds
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e7)
            || (0..3).any(|i| bounds[0][i] > bounds[1][i])
        {
            return None;
        }
        let mut target = 0.75 / camera.scale as f64;
        if camera.perspective {
            let distance = rect.height().max(1.) / camera.scale * 2.;
            let mut near = f32::INFINITY;
            for corner in 0..8 {
                let p =
                    std::array::from_fn(|i| bounds[usize::from(corner & (1 << i) != 0)][i] as f32);
                near = near.min(camera.project(p, rect)[2]);
            }
            if !near.is_finite() {
                return None;
            }
            target *= near.max(0.1) as f64 / distance as f64;
        }
        Some(
            Self::ALL
                .into_iter()
                .find(|q| q.deflection_mm() <= target)
                .unwrap_or(Self::Precision),
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refinement {
    Started,
    Unchanged,
    Unsupported,
}
