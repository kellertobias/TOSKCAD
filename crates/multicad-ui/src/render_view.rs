//! Export → Render view: capture the current viewport camera and display
//! style into the document's render scene, and show a stored scene in the
//! viewport. Both only set camera state; nothing renders continuously.
//!
//! The viewport camera orbits `target` at a distance of twice the viewport
//! height in millimeters (`2 h / scale`). Its perspective projection
//! therefore has a fixed vertical field of view of 2·atan(1/4), and its
//! orthographic projection shows `h / scale` millimeters vertically.
use crate::{camera::Camera, navigation::DisplayStyle};
use multicad_core::model::{Projection, RenderCamera, RenderDisplay};

/// The viewport's perspective vertical field of view in degrees.
pub fn viewport_fov_deg() -> f64 {
    2. * 0.25f64.atan().to_degrees()
}
pub fn display_of(style: DisplayStyle) -> RenderDisplay {
    match style {
        DisplayStyle::Filled => RenderDisplay::Shaded,
        DisplayStyle::VisibleEdges => RenderDisplay::ShadedEdges,
        DisplayStyle::AllEdges => RenderDisplay::Xray,
        DisplayStyle::Wire => RenderDisplay::Wireframe,
    }
}
pub fn style_of(display: RenderDisplay) -> DisplayStyle {
    match display {
        RenderDisplay::Shaded => DisplayStyle::Filled,
        RenderDisplay::ShadedEdges => DisplayStyle::VisibleEdges,
        RenderDisplay::Xray => DisplayStyle::AllEdges,
        RenderDisplay::Wireframe => DisplayStyle::Wire,
    }
}
/// The render camera matching the viewport camera in a viewport
/// `height_px` pixels tall.
pub fn capture(camera: &Camera, height_px: f32) -> Result<RenderCamera, String> {
    if !(height_px.is_finite() && height_px > 0.) || !camera.scale.is_finite() || camera.scale <= 0.
    {
        return Err("The viewport has no size yet".into());
    }
    let (s, c) = (camera.yaw as f64).sin_cos();
    let (sp, cp) = (camera.pitch as f64).sin_cos();
    let forward = [cp * s, cp * c, sp];
    let height_mm = height_px as f64 / camera.scale as f64;
    let distance = 2. * height_mm;
    let target = camera.target.map(f64::from);
    Ok(RenderCamera {
        eye_mm: std::array::from_fn(|i| target[i] + forward[i] * distance),
        target_mm: target,
        projection: if camera.perspective {
            Projection::Perspective {
                fov_deg: viewport_fov_deg(),
            }
        } else {
            Projection::Orthographic { height_mm }
        },
    })
}
/// The viewport camera showing a render camera in a viewport `height_px`
/// pixels tall. A perspective field of view other than the viewport's is
/// shown at the viewport's own angle, framing the same target distance.
pub fn restore(render: &RenderCamera, height_px: f32) -> Result<Camera, String> {
    if !(height_px.is_finite() && height_px > 0.) {
        return Err("The viewport has no size yet".into());
    }
    let v: [f64; 3] = std::array::from_fn(|i| render.eye_mm[i] - render.target_mm[i]);
    let distance = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if distance <= 0. {
        return Err("The render camera eye and target coincide".into());
    }
    let f = v.map(|c| c / distance);
    let (perspective, height_mm) = match render.projection {
        Projection::Perspective { .. } => (true, distance / 2.),
        Projection::Orthographic { height_mm } => (false, height_mm),
    };
    Ok(Camera {
        yaw: f[0].atan2(f[1]) as f32,
        pitch: f[2].clamp(-1., 1.).asin() as f32,
        scale: (height_px as f64 / height_mm).clamp(0.02, 100.) as f32,
        target: render.target_mm.map(|c| c as f32),
        perspective,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }

    #[test]
    fn capture_and_restore_round_trip_the_viewport_camera() {
        for perspective in [true, false] {
            let camera = Camera {
                yaw: -0.6,
                pitch: 0.5,
                scale: 2.4,
                target: [90., 40., 12.],
                perspective,
            };
            let render = capture(&camera, 600.).unwrap();
            // The eye sits 2 h / scale from the target, above it for a
            // positive pitch.
            let d: f64 = (0..3)
                .map(|i| (render.eye_mm[i] - render.target_mm[i]).powi(2))
                .sum::<f64>()
                .sqrt();
            close(d, 2. * 600. / 2.4);
            assert!(render.eye_mm[2] > render.target_mm[2]);
            match render.projection {
                Projection::Perspective { fov_deg } => {
                    assert!(perspective);
                    close(fov_deg, 28.072_486_935_852_954);
                }
                Projection::Orthographic { height_mm } => {
                    assert!(!perspective);
                    close(height_mm, 250.);
                }
            }
            let back = restore(&render, 600.).unwrap();
            close(back.yaw as f64, -0.6);
            close(back.pitch as f64, 0.5);
            close(back.scale as f64, 2.4);
            assert_eq!(back.perspective, perspective);
            for i in 0..3 {
                close(back.target[i] as f64, camera.target[i] as f64);
            }
            // A different viewport height keeps the framed size.
            let taller = restore(&render, 1200.).unwrap();
            close(taller.scale as f64, 4.8);
        }
        for style in DisplayStyle::ALL {
            assert_eq!(style_of(display_of(style)), style);
        }
        assert!(capture(&Camera::default(), 0.).is_err());
    }

    #[test]
    fn a_captured_view_is_a_valid_render_scene() {
        let camera = Camera {
            perspective: true,
            ..Default::default()
        };
        let scene = multicad_core::model::RenderScene {
            camera: capture(&camera, 500.).unwrap(),
            ..Default::default()
        };
        scene.validate().unwrap();
    }
}
