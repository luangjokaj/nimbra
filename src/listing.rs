//! A mailbox's list is replaced on every refresh, so a position in it means nothing
//! afterwards. This works out where everything kept by position (the open thread, the picked
//! rows, the undo history) is in the new list.

use std::collections::HashMap;
use std::hash::Hash;

/// The new list, and where the old one's rows are in it.
pub struct Relisted<T> {
    pub list: Vec<T>,
    /// For each old row, its row now.
    pub moved: Vec<Option<usize>>,
    /// Positions in the history of cleared threads that are back: someone wrote since.
    pub back: Vec<usize>,
}

/// A thread cleared here (archived, trashed) is gone from Gmail's next listing. It stays in
/// the list, hidden, where it was, so that Undo still has it and undoes that one, not the
/// action before it. With a newer message it is not cleared any more.
pub fn relist<T: Clone, K: Hash + Eq>(
    old: &[T],
    mut fresh: Vec<T>,
    history: &[usize],
    key: impl Fn(&T) -> K,
    at: impl Fn(&T) -> i64,
) -> Relisted<T> {
    let mut back = Vec::new();
    let mut absent = Vec::new();
    {
        let mut listed: HashMap<K, i64> = HashMap::new();
        for thread in &fresh {
            listed.entry(key(thread)).or_insert_with(|| at(thread));
        }
        for (position, &row) in history.iter().enumerate() {
            let Some(thread) = old.get(row) else { continue };
            match listed.get(&key(thread)) {
                Some(&newest) if newest > at(thread) => back.push(position),
                Some(_) => {}
                None => absent.push(row),
            }
        }
    }
    absent.sort_unstable();
    absent.dedup();
    for row in absent {
        fresh.insert(row.min(fresh.len()), old[row].clone());
    }
    let mut rows: HashMap<K, usize> = HashMap::new();
    for (row, thread) in fresh.iter().enumerate() {
        rows.entry(key(thread)).or_insert(row);
    }
    let moved = old.iter().map(|t| rows.get(&key(t)).copied()).collect();
    Relisted {
        list: fresh,
        moved,
        back,
    }
}

/// Takes one entry out of the undo history, and out of the batch it was cleared in.
/// `groups` are the batches' sizes, the last one ending where the history does.
pub fn forget(history: &mut Vec<usize>, groups: &mut Vec<usize>, position: usize) {
    if position >= history.len() {
        return;
    }
    let mut end = history.len();
    for group in (0..groups.len()).rev() {
        let start = end.saturating_sub(groups[group]);
        if position >= start {
            groups[group] = groups[group].saturating_sub(1);
            if groups[group] == 0 {
                groups.remove(group);
            }
            break;
        }
        end = start;
    }
    history.remove(position);
}

#[cfg(test)]
mod tests {
    use super::*;

    type Row = (&'static str, i64);

    fn relisted(old: &[Row], fresh: &[Row], history: &[usize]) -> Relisted<Row> {
        relist(old, fresh.to_vec(), history, |r| r.0, |r| r.1)
    }

    const OLD: [Row; 4] = [("a", 4), ("b", 3), ("c", 2), ("d", 1)];

    #[test]
    fn new_mail_moves_every_row_down_one() {
        let now = relisted(
            &OLD,
            &[("n", 9), ("a", 4), ("b", 3), ("c", 2), ("d", 1)],
            &[],
        );
        assert_eq!(now.moved, [Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn a_thread_that_is_gone_is_nowhere_not_its_neighbour() {
        let now = relisted(&OLD, &[("a", 4), ("c", 2), ("d", 1)], &[]);
        assert_eq!(now.moved, [Some(0), None, Some(1), Some(2)]);
    }

    #[test]
    fn cleared_threads_stay_for_undo_in_their_order() {
        // c was cleared, then a; neither is listed any more.
        let now = relisted(&OLD, &[("b", 3), ("d", 1)], &[2, 0]);
        assert_eq!(now.list, OLD);
        assert_eq!(now.moved, [Some(0), Some(1), Some(2), Some(3)]);
        assert!(now.back.is_empty());
    }

    #[test]
    fn a_newer_message_brings_a_cleared_thread_back() {
        let now = relisted(&OLD, &[("c", 9), ("a", 4), ("b", 3), ("d", 1)], &[0, 2]);
        assert_eq!(now.back, [1]);
        assert_eq!(now.moved[2], Some(0));
    }

    #[test]
    fn forgetting_keeps_the_batches_in_step() {
        let (mut history, mut groups) = (vec![5, 6, 7, 8], vec![1, 3]);
        forget(&mut history, &mut groups, 2);
        assert_eq!((&history[..], &groups[..]), (&[5, 6, 8][..], &[1, 2][..]));
        forget(&mut history, &mut groups, 0);
        assert_eq!((&history[..], &groups[..]), (&[6, 8][..], &[2][..]));
    }
}
