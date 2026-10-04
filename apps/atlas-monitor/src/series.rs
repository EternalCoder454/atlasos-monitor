//! A chart's history: the last 60 samples, oldest first, published to QML
//! as a `list<real>` once per tick (docs/DESIGN.md, Charts).

use std::collections::VecDeque;

use cxx_qt_lib::QList;

/// Samples a chart holds: a minute at the default interval.
pub const LEN: usize = 60;

#[derive(Debug, Default)]
pub struct Series {
    values: VecDeque<f64>,
}

impl Series {
    /// Adds a sample, dropping the oldest past [`LEN`]. A value that isn't
    /// a number is stored as 0, which the chart can draw.
    pub fn push(&mut self, v: f64) {
        if self.values.len() == LEN {
            self.values.pop_front();
        }
        self.values.push_back(if v.is_finite() { v } else { 0.0 });
    }

    /// Adds a sample, keeping one that isn't a number as a gap, which a
    /// chart leaves blank: a card left asleep, an adapter unplugged.
    pub fn record(&mut self, v: f64) {
        if self.values.len() == LEN {
            self.values.pop_front();
        }
        self.values.push_back(v);
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// The samples padded at the front with NaN to [`LEN`], which a chart
    /// leaves blank: lists of several series cut apart at multiples of it.
    pub fn padded(&self) -> impl Iterator<Item = f64> + '_ {
        std::iter::repeat_n(f64::NAN, LEN - self.values.len()).chain(self.values.iter().copied())
    }

    pub fn to_qlist(&self) -> QList<f64> {
        let mut list = QList::default();
        list.reserve(self.values.len() as isize);
        for &v in &self.values {
            list.append(v);
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_sixty_oldest_first() {
        let mut s = Series::default();
        for i in 0..100 {
            s.push(f64::from(i));
        }
        let v: Vec<f64> = s.values.iter().copied().collect();
        assert_eq!(v.len(), LEN);
        assert_eq!(v[0], 40.0);
        assert_eq!(v[LEN - 1], 99.0);
        s.push(f64::NAN);
        assert_eq!(s.values.back(), Some(&0.0));
        assert_eq!(s.padded().count(), LEN);
        s.clear();
        s.push(7.0);
        let p: Vec<f64> = s.padded().collect();
        assert_eq!(p.len(), LEN);
        assert!(p[..LEN - 1].iter().all(|v| v.is_nan()));
        assert_eq!(p[LEN - 1], 7.0);
        s.record(f64::NAN);
        assert!(s.values.back().is_some_and(|v| v.is_nan()));
        s.clear();
        assert!(s.values.is_empty());
    }
}
