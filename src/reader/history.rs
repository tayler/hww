//! Back and forward, over a stack with a cursor.
//!
//! In memory only, and gone with the process. An on-disk history is the archive's problem and
//! arrives with it — reading history at rest is a whole second doctrine, and none of the
//! reading experience depends on it.

use url::Url;

#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Url>,
    /// Index of the current entry. Meaningless when `entries` is empty.
    cursor: usize,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Follow a link. Everything ahead of the cursor is discarded — the same rule every
    /// browser uses, and the reason forward is not a second stack.
    pub fn push(&mut self, url: Url) {
        // Re-navigating to where you already are is a reload, not a history entry; without
        // this, `r` on a page makes Back a no-op that looks broken.
        if self.current() == Some(&url) {
            return;
        }
        self.entries.truncate(self.cursor + 1);
        if !self.entries.is_empty() {
            self.cursor += 1;
        }
        self.entries.push(url);
        self.cursor = self.entries.len() - 1;
    }

    pub fn current(&self) -> Option<&Url> {
        self.entries.get(self.cursor)
    }

    pub fn can_go_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn back(&mut self) -> Option<&Url> {
        if !self.can_go_back() {
            return None;
        }
        self.cursor -= 1;
        self.current()
    }

    pub fn forward(&mut self) -> Option<&Url> {
        if !self.can_go_forward() {
            return None;
        }
        self.cursor += 1;
        self.current()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn empty_history_goes_nowhere() {
        let mut h = History::new();
        assert!(h.current().is_none());
        assert!(h.back().is_none());
        assert!(h.forward().is_none());
    }

    #[test]
    fn walks_back_and_forward_over_a_chain() {
        let mut h = History::new();
        for s in ["https://a/", "https://b/", "https://c/"] {
            h.push(u(s));
        }
        assert_eq!(h.current(), Some(&u("https://c/")));
        assert_eq!(h.back(), Some(&u("https://b/")));
        assert_eq!(h.back(), Some(&u("https://a/")));
        assert!(!h.can_go_back());
        assert_eq!(h.forward(), Some(&u("https://b/")));
        assert_eq!(h.forward(), Some(&u("https://c/")));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn following_a_link_after_going_back_discards_the_forward_branch() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        h.back();
        h.push(u("https://c/"));
        assert!(!h.can_go_forward());
        assert_eq!(h.len(), 2);
        assert_eq!(h.back(), Some(&u("https://a/")));
    }

    /// A reload must not make Back a no-op that looks broken.
    #[test]
    fn reloading_the_current_page_is_not_a_history_entry() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        h.push(u("https://b/"));
        assert_eq!(h.len(), 2);
        assert!(h.can_go_back());
    }

    /// Back, then forward to the same page, must not grow the stack either.
    #[test]
    fn revisiting_a_page_that_is_already_current_is_idempotent() {
        let mut h = History::new();
        h.push(u("https://a/"));
        h.push(u("https://b/"));
        h.back();
        h.push(u("https://a/"));
        assert_eq!(h.len(), 2);
        assert!(h.can_go_forward());
    }
}
