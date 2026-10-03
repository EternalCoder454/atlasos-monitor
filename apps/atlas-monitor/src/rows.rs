//! Bringing a list model from the rows it shows to new ones in steps:
//! removals, moves and insertions, then one report of changed figures. The
//! view keeps its rows, scroll position and selection, which a reset would
//! lose. The Apps and Services tables both work this way.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// One step from the rows shown to the rows wanted.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Rows `first..=last` go.
    Remove(usize, usize),
    /// The row at `from` moves to `to` (its index once moved).
    Move(usize, usize),
    /// `n` new rows from `at`.
    Insert(usize, usize),
}

pub fn int(v: usize) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// Indices into `seq` of a longest strictly increasing run of its values.
fn increasing(seq: &[usize]) -> Vec<usize> {
    // tails[l]: the index ending the best run of length l + 1 found so far.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev = vec![usize::MAX; seq.len()];
    for (i, &v) in seq.iter().enumerate() {
        let l = tails.partition_point(|&t| seq[t] < v);
        if l > 0 {
            prev[i] = tails[l - 1];
        }
        if l == tails.len() {
            tails.push(i);
        } else {
            tails[l] = i;
        }
    }
    let mut run = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied().unwrap_or(usize::MAX);
    while at != usize::MAX {
        run.push(at);
        at = prev[at];
    }
    run.reverse();
    run
}

/// The steps that turn `old` into `new` (keys, each once): removals from
/// the bottom up, then moves, then the new rows. The longest run of rows
/// already in order stays put and every other row moves once, so one row
/// jumping is one step, not a step for every row it passes.
pub fn plan<K: Eq + Hash>(old: &[K], new: &[K]) -> Vec<Op> {
    let wanted: HashSet<&K> = new.iter().collect();
    let mut ops = Vec::new();
    let mut cur: Vec<&K> = old.iter().collect();
    let mut i = cur.len();
    while i > 0 {
        i -= 1;
        if !wanted.contains(cur[i]) {
            let last = i;
            while i > 0 && !wanted.contains(cur[i - 1]) {
                i -= 1;
            }
            ops.push(Op::Remove(i, last));
            cur.drain(i..=last);
        }
    }
    let place: HashMap<&K, usize> = new.iter().enumerate().map(|(i, k)| (k, i)).collect();
    let places: Vec<usize> = cur.iter().map(|k| place[k]).collect();
    let still: HashSet<&K> = increasing(&places).into_iter().map(|i| cur[i]).collect();
    // Every other row that stays moves once, in the new order, to just
    // after the row it follows there (or to the top): then the rows that
    // stay are in the new order, short only of the new ones.
    let staying: HashSet<&K> = cur.iter().copied().collect();
    let mut before: Option<&K> = None;
    for k in new {
        if !staying.contains(k) {
            continue;
        }
        if !still.contains(k) {
            let from = cur.iter().position(|c| *c == k).unwrap_or_default();
            let after = before.and_then(|b| cur.iter().position(|c| *c == b));
            let to = match after {
                None => 0,
                Some(a) if a < from => a + 1,
                Some(a) => a,
            };
            if to != from {
                ops.push(Op::Move(from, to));
                let moved = cur.remove(from);
                cur.insert(to, moved);
            }
        }
        before = Some(k);
    }
    for (i, k) in new.iter().enumerate() {
        if cur.get(i) != Some(&k) {
            match ops.last_mut() {
                Some(Op::Insert(at, n)) if *at + *n == i => *n += 1,
                _ => ops.push(Op::Insert(i, 1)),
            }
            cur.insert(i, k);
        }
    }
    ops
}

/// Gives a list model `replace_rows(new)`, which plays [`plan`] on its
/// `rows` (each with a `key`) with Qt's begin and end calls, reports the
/// changed figures once, and keeps its `count` property; and
/// `update_count()`. The model inherits the calls `plan` needs
/// (`beginInsertRows` ... `endResetModel`, `index`, `dataChanged`).
macro_rules! row_model {
    ($model:ty, $row:ty, $what:literal) => {
        impl $model {
            fn replace_rows(mut self: ::std::pin::Pin<&mut Self>, new: Vec<$row>) {
                use $crate::rows::{Op, int};
                let old: Vec<_> = self.rust().rows.iter().map(|r| r.key.clone()).collect();
                let keys: Vec<_> = new.iter().map(|r| r.key.clone()).collect();
                let root = ::cxx_qt_lib::QModelIndex::default();
                for op in $crate::rows::plan(&old, &keys) {
                    match op {
                        Op::Remove(first, last) => {
                            self.as_mut()
                                .begin_remove_rows(&root, int(first), int(last));
                            self.as_mut().rust_mut().rows.drain(first..=last);
                            self.as_mut().end_remove_rows();
                        }
                        Op::Move(from, to) => {
                            // Qt's destination is the row it goes before,
                            // counted with it still in place.
                            let dest = if to > from { to + 1 } else { to };
                            if !self.as_mut().begin_move_rows(
                                &root,
                                int(from),
                                int(from),
                                &root,
                                int(dest),
                            ) {
                                // Can't happen for a real move; if Qt
                                // refuses one anyway, start the view over
                                // rather than drift.
                                log::error!(
                                    "the {} model refused a move from {from} to {to}",
                                    $what
                                );
                                self.as_mut().begin_reset_model();
                                self.as_mut().rust_mut().rows = new;
                                self.as_mut().end_reset_model();
                                self.as_mut().update_count();
                                return;
                            }
                            let rows = &mut self.as_mut().rust_mut().rows;
                            let row = rows.remove(from);
                            rows.insert(to, row);
                            self.as_mut().end_move_rows();
                        }
                        Op::Insert(at, n) => {
                            self.as_mut()
                                .begin_insert_rows(&root, int(at), int(at + n - 1));
                            self.as_mut()
                                .rust_mut()
                                .rows
                                .splice(at..at, new[at..at + n].iter().cloned());
                            self.as_mut().end_insert_rows();
                        }
                    }
                }
                // The figures: one report covering every row that changed.
                let mut changed: Option<(usize, usize)> = None;
                {
                    let mut r = self.as_mut().rust_mut();
                    for (i, (row, want)) in r.rows.iter_mut().zip(new).enumerate() {
                        if *row != want {
                            *row = want;
                            changed = Some(changed.map_or((i, i), |(a, _)| (a, i)));
                        }
                    }
                }
                if let Some((a, b)) = changed {
                    let top = self.index(int(a), 0, &root);
                    let bottom = self.index(int(b), 0, &root);
                    self.as_mut()
                        .data_changed(&top, &bottom, &::cxx_qt_lib::QList::default());
                }
                self.update_count();
            }

            fn update_count(self: ::std::pin::Pin<&mut Self>) {
                let count = $crate::rows::int(self.rust().rows.len());
                if count != *self.count() {
                    self.set_count(count);
                }
            }
        }
    };
}
pub(crate) use row_model;

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays a plan on `old`, as the model does.
    fn play(old: &[u32], new: &[u32]) -> Vec<u32> {
        let mut cur = old.to_vec();
        for op in plan(old, new) {
            match op {
                Op::Remove(a, b) => {
                    cur.drain(a..=b);
                }
                Op::Move(from, to) => {
                    assert_ne!(to, from);
                    let r = cur.remove(from);
                    cur.insert(to, r);
                }
                Op::Insert(at, n) => {
                    for (j, key) in new[at..at + n].iter().enumerate() {
                        cur.insert(at + j, *key);
                    }
                }
            }
        }
        cur
    }

    #[test]
    fn plans_reach_the_new_rows() {
        let cases: [(&[u32], &[u32]); 7] = [
            (&[], &[1, 2, 3]),
            (&[1, 2, 3], &[]),
            (&[1, 2, 3], &[1, 2, 3]),
            (&[1, 2, 3], &[3, 2, 1]),
            (&[1, 2, 3, 4, 5], &[2, 6, 4, 7, 8]),
            (&[1, 2, 3, 4], &[1, 3]),
            (&[5, 1, 2], &[1, 2, 5, 9]),
        ];
        for (old, new) in cases {
            assert_eq!(play(old, new), new, "{old:?} -> {new:?}");
        }
    }

    #[test]
    fn an_unchanged_order_is_no_step_and_new_rows_come_together() {
        assert!(plan(&[1, 2], &[1, 2]).is_empty());
        assert_eq!(plan(&[1, 2], &[1, 2, 3, 4]), vec![Op::Insert(2, 2)]);
        assert_eq!(plan(&[1, 2, 3], &[3]), vec![Op::Remove(0, 1)]);
    }

    #[test]
    fn one_row_moving_is_one_step() {
        let old: Vec<u32> = (1..=50).collect();
        let mut new = old.clone();
        let top = new.remove(0);
        new.push(top);
        assert_eq!(plan(&old, &new), vec![Op::Move(0, 49)]);
        assert_eq!(plan(&new, &old), vec![Op::Move(49, 0)]);
        let mut new = old.clone();
        new.swap(10, 30);
        assert_eq!(play(&old, &new), new);
        assert_eq!(plan(&old, &new).len(), 2);
    }

    #[test]
    fn random_plans_reach_the_new_rows_moving_each_row_at_most_once() {
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..500 {
            let old: Vec<u32> = (0..next(30) as u32).collect();
            let mut new: Vec<u32> = old.iter().copied().filter(|_| next(4) != 0).collect();
            for j in (1..new.len()).rev() {
                if next(3) == 0 {
                    new.swap(j, next(j as u64 + 1) as usize);
                }
            }
            for n in 100..100 + next(5) as u32 {
                new.insert(next(new.len() as u64 + 1) as usize, n);
            }
            assert_eq!(play(&old, &new), new, "{old:?} -> {new:?}");
            let moves = plan(&old, &new)
                .iter()
                .filter(|o| matches!(o, Op::Move(..)))
                .count();
            assert!(moves <= old.len(), "{old:?} -> {new:?}");
        }
    }
}
