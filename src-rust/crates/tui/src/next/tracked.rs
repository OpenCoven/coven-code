//! Presentation-cache invalidation at the mutable state boundary. Even an
//! indexed in-place edit changes identity before a mutable reference escapes.
use std::{
    cell::Cell,
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(Debug, Clone)]
pub(crate) struct Stamp(Arc<()>);
impl PartialEq for Stamp {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Stamp {}

#[derive(Debug)]
pub struct Tracked<T> {
    value: T,
    stamp: Stamp,
    dirty_from: Cell<usize>,
}
impl<T> From<T> for Tracked<T> {
    fn from(value: T) -> Self {
        Self {
            value,
            stamp: Stamp(Arc::new(())),
            dirty_from: Cell::new(0),
        }
    }
}
impl<T: Default> Default for Tracked<T> {
    fn default() -> Self {
        T::default().into()
    }
}
impl<T> Tracked<T> {
    pub fn into_inner(self) -> T {
        self.value
    }
    pub(crate) fn stamp(&self) -> Stamp {
        self.stamp.clone()
    }
    pub(crate) fn take_dirty_from(&self) -> usize {
        self.dirty_from.replace(usize::MAX)
    }
    fn changed_from(&mut self, from: usize) {
        self.stamp = Stamp(Arc::new(()));
        self.dirty_from.set(self.dirty_from.get().min(from));
    }
}
impl<T> Deref for Tracked<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.changed_from(0);
        &mut self.value
    }
}
impl<T, Rhs: ?Sized> PartialEq<Rhs> for Tracked<T>
where
    T: PartialEq<Rhs>,
{
    fn eq(&self, rhs: &Rhs) -> bool {
        self.value.eq(rhs)
    }
}
impl<T: std::fmt::Display> std::fmt::Display for Tracked<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}
impl<'a, T> IntoIterator for &'a Tracked<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&self.value).into_iter()
    }
}
impl<'a, T> IntoIterator for &'a mut Tracked<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().into_iter()
    }
}
impl<T> Tracked<Vec<T>> {
    pub fn push(&mut self, value: T) {
        self.changed_from(self.value.len());
        self.value.push(value);
    }
}
impl Tracked<String> {
    pub fn push_str(&mut self, text: &str) {
        self.changed_from(self.value.len());
        self.value.push_str(text);
    }
    pub fn push(&mut self, ch: char) {
        self.changed_from(self.value.len());
        self.value.push(ch);
    }
}

impl From<&str> for Tracked<String> {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutable_borrows_and_replacement_change_identity_even_at_the_same_length() {
        let mut text: Tracked<String> = "alpha".into();
        let before = text.stamp();
        assert_eq!(text.take_dirty_from(), 0);
        text.replace_range(.., "bravo");
        assert_ne!(before, text.stamp());
        assert_eq!(text.take_dirty_from(), 0);
        let before = text.stamp();
        text = "bravo".into();
        assert_ne!(before, text.stamp());
        assert_eq!(text.take_dirty_from(), 0);
    }
    #[test]
    fn append_records_the_earliest_changed_byte_across_multiple_chunks() {
        let mut text: Tracked<String> = "prefix\n".into();
        text.take_dirty_from();
        text.push_str("new");
        text.push('終');
        assert_eq!(text.take_dirty_from(), 7);
        text.clear();
        text.push_str("retry");
        assert_eq!(text.take_dirty_from(), 0);
    }
    #[test]
    fn indexing_mutably_invalidates_but_reads_do_not() {
        let mut values: Tracked<Vec<String>> = vec!["first".into(), "second".into()].into();
        let stamp = values.stamp();
        values.take_dirty_from();
        assert_eq!(values[1], "second");
        assert_eq!(stamp, values.stamp());
        values[1].replace_range(.., "SECOND");
        assert_ne!(stamp, values.stamp());
        assert_eq!(values.take_dirty_from(), 0);
    }
}
