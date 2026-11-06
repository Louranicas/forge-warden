//! A list that cannot be empty, so code that needs "at least one" never needs a default.

/// One or more values, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T> {
    head: T,
    tail: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// The values, if there is at least one.
    #[must_use]
    pub fn from_vec(mut items: Vec<T>) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let head = items.remove(0);
        Some(Self { head, tail: items })
    }

    /// The first value.
    #[must_use]
    pub const fn first(&self) -> &T {
        &self.head
    }

    /// Every value, in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.head).chain(self.tail.iter())
    }

    /// How many values (at least 1).
    #[must_use]
    #[expect(
        clippy::len_without_is_empty,
        reason = "a NonEmpty is never empty; an is_empty that always says false would be noise"
    )]
    pub const fn len(&self) -> usize {
        self.tail.len().saturating_add(1)
    }

    /// The same list with `f` applied to each value, in order.
    pub fn map<U>(&self, mut f: impl FnMut(&T) -> U) -> NonEmpty<U> {
        let head = f(&self.head);
        NonEmpty {
            head,
            tail: self.tail.iter().map(f).collect(),
        }
    }

    /// The values as a plain vector (never empty).
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.len());
        out.push(self.head);
        out.extend(self.tail);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_refused_and_order_is_kept() {
        assert_eq!(NonEmpty::<u8>::from_vec(Vec::new()), None);
        let list = NonEmpty::from_vec(vec![3, 1, 2]).unwrap();
        assert_eq!(*list.first(), 3);
        assert_eq!(list.len(), 3);
        assert_eq!(list.iter().copied().collect::<Vec<_>>(), [3, 1, 2]);
        assert_eq!(list.map(|n| n * 10).into_vec(), [30, 10, 20]);
        assert_eq!(NonEmpty::from_vec(vec![7]).unwrap().len(), 1);
    }
}
