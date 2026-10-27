use crate::camera::Camera;
use eframe::egui::Rect;
/// Coarsen display lines at distant zoom without changing the configured sketch
/// snap spacing. Center the finite display patch on the current view.
pub fn floor_grid(camera: Camera, rect: Rect, base_mm: f64) -> Vec<[[f64; 3]; 2]> {
    let facing = camera.pitch.sin().abs() as f64;
    if !base_mm.is_finite()
        || base_mm <= 0.
        || !camera.scale.is_finite()
        || camera.scale <= 0.
        || facing < 0.05
        || !facing.is_finite()
        || !rect.is_finite()
        || camera.target.iter().any(|v| !v.is_finite())
    {
        return vec![];
    }
    let scale = camera.scale as f64;
    let factor = (8. / (base_mm * scale)).max(1.).log10().ceil();
    let mut step = base_mm * 10_f64.powf(factor);
    let extent = rect.width().max(rect.height()) as f64 / scale / facing;
    while extent / step > 100. {
        step *= 10.;
    }
    let count = (extent / step).ceil().clamp(4., 100.) as i32;
    let center = [
        (camera.target[0] as f64 / step).round() * step,
        (camera.target[1] as f64 / step).round() * step,
    ];
    let half = count as f64 * step;
    let mut lines = vec![];
    for i in -count..=count {
        let offset = i as f64 * step;
        lines.extend([
            [
                [center[0] + offset, center[1] - half, 0.],
                [center[0] + offset, center[1] + half, 0.],
            ],
            [
                [center[0] - half, center[1] + offset, 0.],
                [center[0] + half, center[1] + offset, 0.],
            ],
        ]);
    }
    lines
}
