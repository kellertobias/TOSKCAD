//! Explicit evaluation-cache lifetime for a native owner thread.
//!
//! An owner can execute several synchronous model consumers, including exports,
//! without rebuilding unchanged features. Native models and cached solids stay
//! on that thread; only the owner's detached results may cross its boundary.
use crate::EvaluationCache;
use std::cell::RefCell;

thread_local! {
    static CURRENT: RefCell<Option<EvaluationCache>> = const { RefCell::new(None) };
}

/// Reuse completed evaluations during `run`, then destroy the cache on this
/// thread. Nested scopes use the existing owner and do not clear its cache.
/// Ordinary compiler calls outside a scope retain their uncached behavior.
pub fn with_evaluation_cache<R>(run: impl FnOnce() -> R) -> R {
    let created = CURRENT.with(|slot| {
        let Ok(mut cache) = slot.try_borrow_mut() else {
            return false;
        };
        if cache.is_some() {
            false
        } else {
            *cache = Some(EvaluationCache::default());
            true
        }
    });
    struct Owner(bool);
    impl Drop for Owner {
        fn drop(&mut self) {
            if self.0 {
                let cache = CURRENT.with(|slot| slot.borrow_mut().take());
                drop(cache);
            }
        }
    }
    let _owner = Owner(created);
    run()
}

pub(crate) fn with_current_cache<R>(run: impl FnOnce(Option<&mut EvaluationCache>) -> R) -> R {
    CURRENT.with(|slot| match slot.try_borrow_mut() {
        Ok(mut cache) => run(cache.as_mut()),
        // Recursive template validation cannot borrow its enclosing compile's
        // cache. It retains the existing full, uncached validation path.
        Err(_) => run(None),
    })
}
