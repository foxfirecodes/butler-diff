use crate::session::Selection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    pub selection: Selection,
    pub file: String,
}

#[derive(Default)]
pub struct History {
    entries: Vec<Visit>,
    cursor: Option<usize>,
}
impl History {
    pub fn record(&mut self, visit: Visit) {
        if self.cursor.and_then(|i| self.entries.get(i)) == Some(&visit) {
            return;
        }
        self.entries.truncate(self.cursor.map_or(0, |i| i + 1));
        self.entries.push(visit);
        if self.entries.len() > 200 {
            self.entries.remove(0);
        }
        self.cursor = Some(self.entries.len() - 1);
    }
    pub fn can_back(&self) -> bool {
        self.cursor.is_some_and(|i| i > 0)
    }
    pub fn can_forward(&self) -> bool {
        self.cursor.is_some_and(|i| i + 1 < self.entries.len())
    }
    pub fn step(&mut self, forward: bool) -> Option<Visit> {
        let i = self.cursor?;
        let next = if forward {
            i.checked_add(1)?
        } else {
            i.checked_sub(1)?
        };
        let visit = self.entries.get(next)?.clone();
        self.cursor = Some(next);
        Some(visit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn visit(branch: &str, file: &str) -> Visit {
        Visit {
            selection: Selection {
                stack: "s".into(),
                anchor: branch.into(),
                end: branch.into(),
            },
            file: file.into(),
        }
    }
    #[test]
    fn navigation_preserves_branch_identity_and_forks_forward_history() {
        let mut h = History::default();
        h.record(visit("a", "same.rs"));
        h.record(visit("a", "same.rs"));
        assert!(!h.can_back());
        h.record(visit("b", "same.rs"));
        assert_eq!(h.step(false), Some(visit("a", "same.rs")));
        assert!(!h.can_back());
        assert_eq!(h.step(true), Some(visit("b", "same.rs")));
        h.step(false);
        h.record(visit("a", "new.rs"));
        assert!(!h.can_forward());
        assert_eq!(h.step(false), Some(visit("a", "same.rs")));
    }
}
