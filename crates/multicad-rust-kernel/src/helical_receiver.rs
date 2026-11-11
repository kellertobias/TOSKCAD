//! Exact cylindrical stock with a finite, cap-entry threaded blind cavity.
use crate::helical_thread::{Thread, ThreadError};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Receiver {
    thread: Thread,
    radius_mm: f64,
    stock_length_mm: f64,
}
impl Receiver {
    pub fn new(thread: Thread, radius_mm: f64, stock_length_mm: f64) -> Result<Self, ThreadError> {
        if !radius_mm.is_finite()
            || !stock_length_mm.is_finite()
            || radius_mm <= thread.spec().major_radius_mm
            || stock_length_mm <= thread.spec().length_mm
        {
            return Err(ThreadError::Invalid);
        }
        if radius_mm > 1e7 || stock_length_mm > 1e7 {
            return Err(ThreadError::Limit);
        }
        Ok(Self {
            thread,
            radius_mm,
            stock_length_mm,
        })
    }
    pub fn thread(&self) -> &Thread {
        &self.thread
    }
    pub fn radius_mm(&self) -> f64 {
        self.radius_mm
    }
    pub fn stock_length_mm(&self) -> f64 {
        self.stock_length_mm
    }
    pub fn volume_mm3(&self) -> f64 {
        std::f64::consts::PI * self.radius_mm.powi(2) * self.stock_length_mm
            - self.thread.volume_mm3()
    }
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        [x, y, z, tolerance].into_iter().all(f64::is_finite)
            && tolerance >= 0.
            && z >= -tolerance
            && z <= self.stock_length_mm + tolerance
            && x.hypot(y) <= self.radius_mm + tolerance
            && (z >= self.thread.spec().length_mm - tolerance
                || x.hypot(y) >= self.thread.radius(y.atan2(x), z.max(0.)) - tolerance)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{display::Tessellation, helical_thread::Spec, helical_thread_mesh, manufacturing};
    fn receiver(left_hand: bool, starts: u32, rounded: bool) -> Receiver {
        let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
        Receiver::new(
            Thread::new(Spec {
                major_radius_mm: 3.,
                pitch_mm: 1.,
                length_mm: 2.5,
                depth_mm: root.map_or(5. * 3_f64.sqrt() / 16., |r| 0.875 * 3_f64.sqrt() / 2. - r),
                crest_width_mm: 0.125,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                starts,
                left_hand,
                root_radius_mm: root,
            })
            .unwrap(),
            5.,
            4.,
        )
        .unwrap()
    }
    #[test]
    fn partial_cylindrical_receivers_keep_a_real_floor_and_closed_manufacturing_boundary() {
        for starts in [1, 2, 8] {
            for left in [false, true] {
                for rounded in [false, true] {
                    let receiver = receiver(left, starts, rounded);
                    assert!(!receiver.contains([0., 0., 1.], 0.));
                    assert!(receiver.contains([0., 0., 3.], 0.));
                    assert!(receiver.contains([4., 0., 1.], 0.));
                    assert!(!receiver.contains([6., 0., 1.], 0.));
                    let options = Tessellation {
                        deflection: 0.1,
                        angle: 0.3,
                        triangle_limit: 150_000,
                    };
                    let mesh =
                        helical_thread_mesh::tessellate_receiver(&receiver, options).unwrap();
                    let proof = manufacturing::certify(&mesh).unwrap();
                    assert!(
                        (proof.signed_volume_mm3 - receiver.volume_mm3()).abs()
                            <= helical_thread_mesh::receiver_volume_error_bound(&receiver, options)
                    );
                    assert_eq!(
                        mesh.face_indices
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>(),
                        (0..4 + 4 * starts).collect()
                    );
                    for [o, u, v] in [
                        [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                        [[2., -3., 4.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                    ] {
                        let text =
                            crate::step::write_helical_receiver(&receiver, o, u, v, 1e-7).unwrap();
                        let volume = crate::step_read::verify_helical_receiver(
                            &text, &receiver, o, u, v, 1e-7,
                        )
                        .unwrap();
                        assert!((volume - receiver.volume_mm3()).abs() < 1e-6);
                        let (metrics, imported) =
                            crate::step_read::read_import_solid(&text, 1e-7).unwrap();
                        assert!((metrics.volume_mm3 - receiver.volume_mm3()).abs() < 1e-6);
                        assert!(crate::step_read::helical_receiver_import_matches(
                            &imported,
                            &receiver,
                            [o, u, v],
                            1e-7
                        ));
                        assert!(matches!(
                            imported.profile,
                            crate::step_read::ImportProfile::HelicalReceiver { .. }
                        ));
                    }
                }
            }
        }
    }
    #[test]
    fn receivers_refuse_invalid_dimensions_and_precision_budgets_without_rewriting_the_floor() {
        let source = receiver(false, 1, false);
        for (radius, height, expected) in [
            (f64::NAN, 4., ThreadError::Invalid),
            (3., 4., ThreadError::Invalid),
            (5., 2.5, ThreadError::Invalid),
            (2e7, 4., ThreadError::Limit),
            (5., 2e7, ThreadError::Limit),
        ] {
            assert_eq!(
                Receiver::new(*source.thread(), radius, height),
                Err(expected)
            );
        }
        assert!(!source.contains([f64::NAN, 0., 1.], 0.));
        assert!(!source.contains([0., 0., 3.], -1.));
        assert!(!source.contains([0., 0., 5.], 0.));
        assert!(!source.contains([0., 0., -1.], 0.));
        let tiny = Receiver::new(*source.thread(), 5., 2.5 + 1e-8).unwrap();
        assert!(!crate::step::supports_helical_receiver(
            &tiny, [0.; 3], 1e-7
        ));
        let options = Tessellation {
            deflection: 0.1,
            angle: 0.3,
            triangle_limit: 150_000,
        };
        let mesh = helical_thread_mesh::tessellate_receiver(&source, options).unwrap();
        assert_eq!(
            helical_thread_mesh::tessellate_receiver(
                &source,
                Tessellation {
                    triangle_limit: mesh.face_indices.len() - 1,
                    ..options
                }
            )
            .unwrap_err(),
            crate::display::MeshError::BudgetExceeded
        );
        assert_eq!(
            helical_thread_mesh::tessellate_receiver(
                &source,
                Tessellation {
                    angle: 0.,
                    ..options
                }
            )
            .unwrap_err(),
            crate::display::MeshError::InvalidOptions
        );
        assert_eq!(
            helical_thread_mesh::tessellate_receiver(&source, options).unwrap(),
            mesh
        );
    }
}
