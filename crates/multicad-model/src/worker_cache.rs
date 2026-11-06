//! Explicit evaluation-cache lifetime for a native owner thread.
//!
//! An owner can execute several synchronous model consumers, including exports,
//! without rebuilding unchanged features. Native models and cached solids stay
//! on that thread; only the owner's detached results may cross its boundary.
use crate::EvaluationCache;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

thread_local! {
    static CURRENT: RefCell<Option<EvaluationCache>> = const { RefCell::new(None) };
}

/// Reuse completed evaluations during `run`, then destroy the cache on this
/// thread. Nested scopes use the existing owner and do not clear its cache.
/// Ordinary compiler calls outside a scope retain their uncached behavior.
pub fn with_evaluation_cache<R>(run: impl FnOnce() -> R) -> R {
    let _owner = EvaluationOwner::enter();
    run()
}

/// RAII lifetime for long-running native worker loops. This guard and every
/// cached native solid are created and dropped on the owner thread.
pub(crate) struct EvaluationOwner(bool, PhantomData<Rc<()>>);
impl EvaluationOwner {
    pub(crate) fn enter() -> Self {
        Self(
            CURRENT.with(|slot| {
                let Ok(mut cache) = slot.try_borrow_mut() else {
                    return false;
                };
                if cache.is_some() {
                    false
                } else {
                    *cache = Some(EvaluationCache::default());
                    true
                }
            }),
            PhantomData,
        )
    }
}
impl Drop for EvaluationOwner {
    fn drop(&mut self) {
        if self.0 {
            let cache = CURRENT.with(|slot| slot.borrow_mut().take());
            drop(cache);
        }
    }
}

pub(crate) fn with_current_cache<R>(run: impl FnOnce(Option<&mut EvaluationCache>) -> R) -> R {
    CURRENT.with(|slot| match slot.try_borrow_mut() {
        Ok(mut cache) => run(cache.as_mut()),
        // Recursive template validation cannot borrow its enclosing compile's
        // cache. It retains the existing full, uncached validation path.
        Err(_) => run(None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        document::Document,
        model::{PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length, prepare_primitive},
    };
    fn source() -> (Document, multicad_core::document::EntityId) {
        let mut session = Session::new(Document::new("Owned native cache").unwrap()).unwrap();
        let request = PrimitiveRequest::new(
            "Block",
            PrimitiveParameters::Box {
                size: [length(2.), length(3.), length(4.)],
                frame: PrimitiveFrame::default(),
            },
        );
        let prepared = prepare_primitive(&session, 0, Actor::local(Channel::Ui), &request).unwrap();
        session.commit(prepared.prepared).unwrap();
        (session.document().clone(), request.feature)
    }
    fn compile(document: &Document) {
        let model = crate::Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(document, 1)
        .unwrap();
        assert_eq!(model.snapshot().shapes.len(), 1);
        assert!((model.snapshot().shapes.values().next().unwrap().volume_mm3 - 24.).abs() < 1e-9);
    }
    #[test]
    fn native_owner_reuses_features_across_jobs_and_nested_scopes_then_drops_them() {
        let (document, feature) = source();
        {
            let _owner = EvaluationOwner::enter();
            compile(&document);
            with_current_cache(|cache| {
                assert!(cache.unwrap().last_compile().evaluated.contains(&feature))
            });
            with_evaluation_cache(|| compile(&document));
            with_current_cache(|cache| {
                assert!(cache.unwrap().last_compile().reused.contains(&feature))
            });
            compile(&document);
            with_current_cache(|cache| assert!(cache.unwrap().last_compile().evaluated.is_empty()));
            std::thread::spawn(|| with_current_cache(|cache| assert!(cache.is_none())))
                .join()
                .unwrap();
        }
        with_current_cache(|cache| assert!(cache.is_none()));
        with_evaluation_cache(|| {
            compile(&document);
            with_current_cache(|cache| {
                assert!(cache.unwrap().last_compile().evaluated.contains(&feature))
            });
        });
    }
    #[test]
    fn unwinding_native_owner_discards_its_cache_before_another_owner_starts() {
        let (document, feature) = source();
        let result = std::panic::catch_unwind(|| {
            with_evaluation_cache(|| {
                compile(&document);
                panic!("failed native owner");
            })
        });
        assert!(result.is_err());
        with_current_cache(|cache| assert!(cache.is_none()));
        with_evaluation_cache(|| {
            compile(&document);
            with_current_cache(|cache| {
                assert!(cache.unwrap().last_compile().evaluated.contains(&feature))
            });
        });
    }
}
