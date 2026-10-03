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

    pub fn clear(&mut self) {
        self.values.clear();
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
        s.clear();
        assert!(s.values.is_empty());
    }
}
