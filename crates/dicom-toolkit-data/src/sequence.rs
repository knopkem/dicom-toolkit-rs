//! Helper functions for working with DICOM sequences.
//!
//! A DICOM sequence is stored as `Vec<DataSet>` inside `Value::Sequence`.
//! This module provides ergonomic helpers so callers don't have to import
//! slice methods directly.

use crate::dataset::DataSet;

/// Return the sequence item at `index`, or `None` if out of bounds.
pub fn get_item(items: &[DataSet], index: usize) -> Option<&DataSet> {
    items.get(index)
}

/// Return the number of items in a sequence.
pub fn item_count(items: &[DataSet]) -> usize {
    items.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_item_in_bounds() {
        let items = vec![DataSet::new(), DataSet::new()];
        assert!(get_item(&items, 0).is_some());
        assert!(get_item(&items, 1).is_some());
    }

    #[test]
    fn get_item_out_of_bounds() {
        let items = vec![DataSet::new()];
        assert!(get_item(&items, 1).is_none());
    }

    #[test]
    fn item_count_empty() {
        assert_eq!(item_count(&[]), 0);
    }

    #[test]
    fn item_count_nonempty() {
        let items = vec![DataSet::new(); 3];
        assert_eq!(item_count(&items), 3);
    }
}
