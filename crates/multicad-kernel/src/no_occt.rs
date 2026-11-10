//! The link-free implementation of the old provider boundary.
use std::{convert::Infallible, fmt, marker::PhantomData, ops::Deref, pin::Pin};

#[derive(Debug)]
pub(crate) struct Unavailable;

impl fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpenCascade is unavailable in this build")
    }
}

impl std::error::Error for Unavailable {}

type Result<T> = std::result::Result<T, Unavailable>;

/// Geometry cannot create a native handle in this configuration.
pub(crate) struct UniquePtr<T>(Infallible, PhantomData<T>);

impl<T> UniquePtr<T> {
    pub(crate) fn is_null(&self) -> bool {
        match self.0 {}
    }

    pub(crate) fn pin_mut(&mut self) -> Pin<&mut T> {
        match self.0 {}
    }
}

impl<T> Deref for UniquePtr<T> {
    type Target = T;

    fn deref(&self) -> &T {
        match self.0 {}
    }
}

include!(concat!(env!("OUT_DIR"), "/no_occt.rs"));
