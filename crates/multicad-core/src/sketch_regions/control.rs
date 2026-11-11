//! Request-local work and cancellation; query providers remain stateless.
use super::*;
use std::cell::Cell;

const MAX_WORK: usize = 4_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryControlError {
    Cancelled,
    Limit,
}
impl std::fmt::Display for QueryControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "Sketch region query was cancelled",
            Self::Limit => "Sketch region query exceeds its cumulative work budget",
        })
    }
}
pub struct QueryControl<'a> {
    interrupt: Option<&'a dyn Fn() -> bool>,
    remaining: Cell<usize>,
    failure: Cell<Option<QueryControlError>>,
}
impl<'a> QueryControl<'a> {
    pub fn new(interrupt: Option<&'a dyn Fn() -> bool>) -> Self {
        Self {
            interrupt,
            remaining: Cell::new(MAX_WORK),
            failure: Cell::new(None),
        }
    }
    /// A smaller caller-owned allowance can bound server-side work further.
    pub fn with_budget(
        interrupt: Option<&'a dyn Fn() -> bool>,
        work: usize,
    ) -> Result<Self, QueryControlError> {
        if work > MAX_WORK {
            return Err(QueryControlError::Limit);
        }
        Ok(Self {
            interrupt,
            remaining: Cell::new(work),
            failure: Cell::new(None),
        })
    }
    pub fn failure(&self) -> Option<QueryControlError> {
        self.failure.get()
    }
    pub fn step(&self) -> Result<(), QueryControlError> {
        if let Some(error) = self.failure.get() {
            return Err(error);
        }
        let error = if self.interrupt.is_some_and(|interrupt| interrupt()) {
            Some(QueryControlError::Cancelled)
        } else if let Some(left) = self.remaining.get().checked_sub(1) {
            self.remaining.set(left);
            None
        } else {
            Some(QueryControlError::Limit)
        };
        if let Some(error) = error {
            self.failure.set(Some(error));
            Err(error)
        } else {
            Ok(())
        }
    }
    pub(super) fn finish(&self) -> Result<(), QueryError> {
        self.failure.get().map_or(Ok(()), |error| {
            Err(QueryError {
                message: error.to_string(),
            })
        })
    }
}
pub(super) fn check_control(control: &QueryControl<'_>) -> Result<(), QueryError> {
    control.step().map_err(|error| QueryError {
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cumulative_failure_cannot_publish_an_unrelated_completed_region() {
        let mut source = Source {
            u: [1., 0., 0.],
            sketch: crate::constraint_sketch::Sketch::default(),
        };
        for offset in [0., 10.] {
            let corners = [
                [offset, 0.],
                [offset + 4., 0.],
                [offset + 4., 3.],
                [offset, 3.],
            ]
            .map(|p| source.add_point(p));
            for i in 0..4 {
                source.add_line(corners[i], corners[(i + 1) % 4]);
            }
        }
        let valid =
            extract_impl(&source, DEFAULT_TOLERANCE_MM, &AnalyticQueries, false, None).unwrap();
        assert_eq!(valid.regions.len(), 2);
        for work in [0, 3, 20] {
            let control = QueryControl::with_budget(None, work).unwrap();
            let _ = extract_impl_controlled(
                &source,
                DEFAULT_TOLERANCE_MM,
                &AnalyticQueries,
                false,
                None,
                &control,
            );
            assert_eq!(control.failure(), Some(QueryControlError::Limit));
            assert!(control.finish().is_err());
            assert_eq!(control.step(), Err(QueryControlError::Limit));
        }
        assert!(matches!(
            QueryControl::with_budget(None, MAX_WORK + 1),
            Err(QueryControlError::Limit)
        ));
        let calls = Cell::new(0);
        let interrupt = || {
            calls.set(calls.get() + 1);
            calls.get() > 12
        };
        assert!(
            extract_impl(
                &source,
                DEFAULT_TOLERANCE_MM,
                &AnalyticQueries,
                false,
                Some(&interrupt)
            )
            .is_err()
        );
        let retry =
            extract_impl(&source, DEFAULT_TOLERANCE_MM, &AnalyticQueries, false, None).unwrap();
        assert_eq!(retry.regions.len(), 2);
    }
}
