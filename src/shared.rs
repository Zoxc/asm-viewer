//! A list built once and passed on by its pointer: what every list of rows the UI draws
//! is. Framework-free.

use std::{fmt, ops::Deref, sync::Arc};

/// A list built once per change and passed on by its pointer: two are equal exactly when
/// they are the same build, never when they are two builds of equal rows. Handing one to a
/// `VirtualScrollView` is then a pointer compare and not a walk of ten thousand rows.
///
/// It derefs to its slice, so the length, the indexing and the iteration are the slice's
/// own.
pub struct Shared<T>(Arc<[T]>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Shared(Arc::default())
    }
}

impl<T> From<Vec<T>> for Shared<T> {
    fn from(rows: Vec<T>) -> Self {
        Shared(rows.into())
    }
}

impl<T> Deref for Shared<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T: fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests;
