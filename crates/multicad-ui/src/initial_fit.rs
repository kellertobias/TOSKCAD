//! Automatic viewport framing when a modeled project opens (CAD-175).
//!
//! The project file stores no camera, so the viewer frames the evaluated model
//! once per opened document. Framing waits for evaluated world bounds and a
//! laid-out viewport, follows viewport resizes that happen before the user
//! touches the camera (window restore and panel layout at startup), and stops
//! for good as soon as the camera differs from the framing it produced.
use crate::camera::Camera;
use eframe::egui::Rect;
use multicad_core::document::EntityId;

/// Smallest viewport side that can be framed meaningfully, in points.
const MIN_SIDE: f32 = 64.;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Framed {
    document: EntityId,
    rect: Rect,
    camera: Camera,
    /// The user navigated (or used Home/Fit); never frame this document again.
    settled: bool,
}
#[derive(Clone, Debug, Default)]
pub struct InitialFit {
    framed: Option<Framed>,
}

fn corners([low, high]: [[f64; 3]; 2]) -> Vec<[f32; 3]> {
    (0..8)
        .map(|c| std::array::from_fn(|i| if c & (1 << i) == 0 { low[i] } else { high[i] } as f32))
        .collect()
}
fn usable(rect: Rect) -> bool {
    rect.is_finite() && rect.width() >= MIN_SIDE && rect.height() >= MIN_SIDE
}
fn usable_bounds([low, high]: [[f64; 3]; 2]) -> bool {
    (0..3).all(|i| low[i].is_finite() && high[i].is_finite() && high[i] >= low[i])
}
fn fitted(mut camera: Camera, bounds: [[f64; 3]; 2], rect: Rect) -> Camera {
    camera.fit(&corners(bounds), rect);
    camera
}

/// The standard isometric orientation, framing the whole model when it has
/// evaluated bounds; an empty or failed model gets the bounded default view
/// at the origin. The projection mode is kept.
pub fn home(current: Camera, bounds: Option<[[f64; 3]; 2]>, rect: Rect) -> Camera {
    let camera = Camera {
        target: [0.; 3],
        perspective: current.perspective,
        ..Default::default()
    };
    match bounds {
        Some(bounds) if usable(rect) && usable_bounds(bounds) => fitted(camera, bounds, rect),
        _ => camera,
    }
}

impl InitialFit {
    /// The camera to show this frame, if the viewer should frame the model.
    pub fn update(
        &mut self,
        document: EntityId,
        bounds: Option<[[f64; 3]; 2]>,
        camera: Camera,
        rect: Option<Rect>,
    ) -> Option<Camera> {
        let rect = rect.filter(|r| usable(*r))?;
        let bounds = bounds.filter(|b| usable_bounds(*b));
        match &mut self.framed {
            Some(framed) if framed.document == document => {
                if framed.settled {
                    return None;
                }
                if camera != framed.camera {
                    framed.settled = true;
                    return None;
                }
                let resized = (framed.rect.size() - rect.size()).length() > 1.;
                let bounds = bounds?;
                if !resized {
                    return None;
                }
                let camera = fitted(camera, bounds, rect);
                framed.rect = rect;
                framed.camera = camera;
                Some(camera)
            }
            _ => {
                let camera = fitted(camera, bounds?, rect);
                self.framed = Some(Framed {
                    document,
                    rect,
                    camera,
                    settled: false,
                });
                Some(camera)
            }
        }
    }
    /// An explicit Home or Fit ends automatic framing for this document.
    pub fn settle(&mut self) {
        if let Some(framed) = &mut self.framed {
            framed.settled = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Pos2, Vec2};

    const CABINET: [[f64; 3]; 2] = [[0., 0., 0.], [600., 560., 720.]];
    fn rect(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(225., 190.), Vec2::new(w, h))
    }
    fn visible(camera: Camera, bounds: [[f64; 3]; 2], rect: Rect) -> bool {
        corners(bounds).into_iter().all(|p| {
            let s = camera.project(p, rect);
            rect.contains(Pos2::new(s[0], s[1]))
        })
    }

    #[test]
    fn default_camera_puts_a_cabinet_outside_the_viewport() {
        // The reported symptom: without framing, a 600 mm panel overflows.
        assert!(!visible(Camera::default(), CABINET, rect(965., 670.)));
    }

    #[test]
    fn frames_once_after_bounds_and_layout_then_respects_navigation() {
        let doc = EntityId::new();
        let mut fit = InitialFit::default();
        let mut camera = Camera::default();
        // No layout or no evaluated bounds yet: wait instead of consuming.
        assert_eq!(fit.update(doc, Some(CABINET), camera, None), None);
        assert_eq!(
            fit.update(doc, Some(CABINET), camera, Some(rect(10., 10.))),
            None
        );
        assert_eq!(fit.update(doc, None, camera, Some(rect(965., 670.))), None);
        camera = fit
            .update(doc, Some(CABINET), camera, Some(rect(965., 670.)))
            .expect("frames when evaluated bounds arrive");
        assert!(visible(camera, CABINET, rect(965., 670.)));
        assert_eq!(camera.yaw, Camera::default().yaw);
        // Later frames do not override.
        assert_eq!(
            fit.update(doc, Some(CABINET), camera, Some(rect(965., 670.))),
            None
        );
        // A startup resize before any navigation reframes for the final size.
        camera = fit
            .update(doc, Some(CABINET), camera, Some(rect(1500., 900.)))
            .expect("follows the settled window size");
        assert!(visible(camera, CABINET, rect(1500., 900.)));
        // After the user zooms, nothing is overridden, even on resize.
        camera.zoom(400.);
        assert_eq!(
            fit.update(doc, Some(CABINET), camera, Some(rect(1500., 900.))),
            None
        );
        assert_eq!(
            fit.update(doc, Some(CABINET), camera, Some(rect(900., 700.))),
            None
        );
        // Another document is framed again.
        let other = EntityId::new();
        assert!(
            fit.update(other, Some(CABINET), camera, Some(rect(900., 700.)))
                .is_some()
        );
        fit.settle();
        assert_eq!(
            fit.update(
                other,
                Some(CABINET),
                Camera::default(),
                Some(rect(800., 700.))
            ),
            None
        );
    }

    #[test]
    fn empty_or_failed_models_stay_bounded() {
        let doc = EntityId::new();
        let mut fit = InitialFit::default();
        let nan = [[f64::NAN; 3], [1.; 3]];
        assert_eq!(
            fit.update(doc, Some(nan), Camera::default(), Some(rect(800., 600.))),
            None
        );
        let home_empty = home(Camera::default(), None, rect(800., 600.));
        assert_eq!(home_empty.target, [0.; 3]);
        assert_eq!(home_empty.scale, Camera::default().scale);
        // A degenerate point model still yields a finite bounded scale.
        let point = home(
            Camera::default(),
            Some([[5.; 3], [5.; 3]]),
            rect(800., 600.),
        );
        assert!(point.scale.is_finite() && (0.02..=100.).contains(&point.scale));
    }

    #[test]
    fn home_restores_orientation_and_frames_the_whole_model() {
        let mut turned = Camera {
            yaw: 2.,
            pitch: -0.3,
            scale: 9.,
            target: [1000.; 3],
            perspective: true,
        };
        let r = rect(965., 670.);
        let homed = home(turned, Some(CABINET), r);
        assert_eq!(
            (homed.yaw, homed.pitch),
            (Camera::default().yaw, Camera::default().pitch)
        );
        assert!(homed.perspective, "projection mode is kept");
        assert!(visible(homed, CABINET, r));
        // Fit keeps the current orientation.
        turned.fit(&corners(CABINET), r);
        assert_eq!((turned.yaw, turned.pitch), (2., -0.3));
    }
}
