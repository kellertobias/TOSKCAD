//! Source-certified curved trims on the original helical and conical surfaces.
use super::*;
use crate::thread_entry_chamfer::{
    Chamfer, QueryOptions,
    trim::{self, Trim},
};

mod curves;
pub(crate) mod plan;
mod shell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidGeometry,
    InvalidPlacement,
    Unsupported,
    Limit,
    Cancelled,
}
impl From<StepError> for Error {
    fn from(value: StepError) -> Self {
        match value {
            StepError::InvalidGeometry => Self::InvalidGeometry,
            StepError::InvalidPlacement => Self::InvalidPlacement,
            StepError::Unsupported => Self::Unsupported,
            StepError::Limit => Self::Limit,
        }
    }
}
impl From<crate::thread_entry_chamfer::Error> for Error {
    fn from(value: crate::thread_entry_chamfer::Error) -> Self {
        match value {
            crate::thread_entry_chamfer::Error::Invalid => Self::InvalidGeometry,
            crate::thread_entry_chamfer::Error::Unresolved => Self::Unsupported,
            crate::thread_entry_chamfer::Error::Limit => Self::Limit,
            crate::thread_entry_chamfer::Error::Cancelled => Self::Cancelled,
        }
    }
}

fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), Error> {
    if interrupt.is_some_and(|callback| callback()) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

pub fn write_thread_entry_chamfer(
    body: &Chamfer,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, Error> {
    write_thread_entry_chamfer_with_interrupt(body, origin, u, v, tolerance_mm, None)
}

pub fn write_thread_entry_chamfer_with_interrupt(
    body: &Chamfer,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<String, Error> {
    let mut writer = Writer::new();
    writer.append_thread_entry_chamfer_with_interrupt(
        body,
        origin,
        u,
        v,
        tolerance_mm,
        interrupt,
    )?;
    writer.finish().map_err(Into::into)
}

impl Writer {
    pub fn append_thread_entry_chamfer(
        &mut self,
        body: &Chamfer,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
    ) -> Result<(), Error> {
        self.append_thread_entry_chamfer_with_interrupt(body, origin, u, v, tolerance_mm, None)
    }
    pub fn append_thread_entry_chamfer_with_interrupt(
        &mut self,
        body: &Chamfer,
        origin: Point,
        u: Point,
        v: Point,
        tolerance_mm: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(), Error> {
        let placement = Placement::new(origin, u, v)?;
        self.append_typed(|file| {
            let plan = plan::prepare(body, &placement, tolerance_mm, interrupt)?;
            shell::encode(file, body, placement, &plan, tolerance_mm, interrupt)
        })
    }
}

/// Reuse the authoritative source trim certificate, never serialized records.
pub(crate) fn read_plan(
    body: &Chamfer,
    frame: [Point; 3],
    tolerance: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<plan::Plan, Error> {
    let [o, u, v] = frame;
    let placement = Placement::new(o, u, v)?;
    plan::prepare(body, &placement, tolerance, interrupt)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(left: bool, rounded: bool, starts: u32) -> crate::helical_thread::Thread {
        let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
        crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: root.map_or(5. * 3_f64.sqrt() / 16., |r| 0.875 * 3_f64.sqrt() / 2. - r),
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts,
            left_hand: left,
            root_radius_mm: root,
        })
        .unwrap()
    }
    #[test]
    fn writer_keeps_original_wall_faces_exact_cone_and_closed_shared_graph() {
        for left in [false, true] {
            for rounded in [false, true] {
                for neck in [false, true] {
                    let source = source(left, rounded, 2);
                    let tail = neck.then_some(crate::thread_end_relief::Intent {
                        width_mm: 0.6,
                        radial_depth_mm: 0.1,
                    });
                    let body = Chamfer::new(source, 0.4, tail).unwrap();
                    let text = write_thread_entry_chamfer(
                        &body,
                        [0.; 3],
                        [1., 0., 0.],
                        [0., 1., 0.],
                        1e-7,
                    )
                    .unwrap();
                    assert_eq!(text.matches("=CONICAL_SURFACE(").count(), 1);
                    assert_eq!(text.matches("=B_SPLINE_SURFACE_WITH_KNOTS(").count(), 8);
                    assert_eq!(text.matches("=MANIFOLD_SOLID_BREP(").count(), 1);
                    assert_eq!(
                        text.matches("=ADVANCED_FACE(").count(),
                        body.face_count() as usize
                    );
                    assert!(text.contains("=SEAM_CURVE("));
                }
            }
        }
    }

    #[test]
    fn typed_cancellation_after_real_encoding_preserves_compound_counters_and_retry() {
        use std::cell::Cell;
        let source = source(false, true, 1);
        let body = Chamfer::new(source, 0.4, None).unwrap();
        let p = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
        let prefix = |writer: &mut Writer| {
            writer
                .append_helical_thread(
                    &source,
                    None,
                    [15., 0., 0.],
                    [1., 0., 0.],
                    [0., 1., 0.],
                    1e-7,
                )
                .unwrap()
        };
        let mut expected = Writer::new();
        prefix(&mut expected);
        expected
            .append_thread_entry_chamfer(&body, p.origin, p.u, p.v, 1e-7)
            .unwrap();
        let expected = expected.finish().unwrap();
        let mut writer = Writer::new();
        prefix(&mut writer);
        let staged = plan::prepare(&body, &p, 1e-7, None).unwrap();
        let cancelled = writer.append_typed::<Error>(|file| {
            shell::encode(
                file,
                &body,
                Placement::new(p.origin, p.u, p.v).expect("fixture placement"),
                &staged,
                1e-7,
                None,
            )
            .expect("qualified fixture encoding");
            Err(Error::Cancelled)
        });
        assert_eq!(cancelled, Err(Error::Cancelled));
        let calls = Cell::new(0);
        let count = || {
            calls.set(calls.get() + 1);
            false
        };
        plan::prepare(&body, &p, 1e-7, Some(&count)).unwrap();
        let limit = calls.get() + 3;
        calls.set(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() >= limit
        };
        assert_eq!(
            writer.append_thread_entry_chamfer_with_interrupt(
                &body,
                p.origin,
                p.u,
                p.v,
                1e-7,
                Some(&cancel)
            ),
            Err(Error::Cancelled)
        );
        writer
            .append_thread_entry_chamfer(&body, p.origin, p.u, p.v, 1e-7)
            .unwrap();
        assert_eq!(writer.finish().unwrap(), expected);
        assert_eq!(body.source_thread(), &source);
    }
}

#[cfg(test)]
mod failures {
    use super::*;
    fn body() -> Chamfer {
        let tool = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        Chamfer::new(tool, 0.4, None).unwrap()
    }
    #[test]
    fn public_errors_and_compound_resource_failure_keep_the_feature_error_contract_and_retry() {
        for (source, expected) in [
            (StepError::InvalidGeometry, Error::InvalidGeometry),
            (StepError::InvalidPlacement, Error::InvalidPlacement),
            (StepError::Unsupported, Error::Unsupported),
            (StepError::Limit, Error::Limit),
        ] {
            assert_eq!(Error::from(source), expected);
        }
        use crate::thread_entry_chamfer::Error as Source;
        for (source, expected) in [
            (Source::Invalid, Error::InvalidGeometry),
            (Source::Unresolved, Error::Unsupported),
            (Source::Limit, Error::Limit),
            (Source::Cancelled, Error::Cancelled),
        ] {
            assert_eq!(Error::from(source), expected);
        }
        let body = body();
        let [o, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        assert_eq!(
            write_thread_entry_chamfer_with_interrupt(&body, o, u, v, 1e-7, Some(&|| true)),
            Err(Error::Cancelled)
        );
        assert_eq!(
            write_thread_entry_chamfer(&body, [1e15, 0., 0.], u, v, 1e-7),
            Err(Error::Limit)
        );
        assert_eq!(
            write_thread_entry_chamfer(&body, o, u, v, f64::NAN),
            Err(Error::InvalidGeometry)
        );
        assert_eq!(
            write_thread_entry_chamfer(&body, o, [1. + 4e-11, 0., 0.], v, 1e-8),
            Err(Error::InvalidPlacement)
        );
        let mut writer = Writer::new();
        let failed = writer.append_typed::<Error>(|file| {
            file.entity(format_args!("CARTESIAN_POINT('',(0.,0.,0.))"));
            file.next = crate::step_read::MAX_STEP_ENTITIES + 2;
            Ok(Entity(1))
        });
        assert_eq!(failed, Err(Error::Limit));
        writer
            .append_thread_entry_chamfer(&body, o, u, v, 1e-7)
            .unwrap();
        assert_eq!(
            writer.finish().unwrap(),
            write_thread_entry_chamfer(&body, o, u, v, 1e-7).unwrap()
        );
    }
}
