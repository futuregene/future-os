//! Follow-tail behavior independent of terminal and agent state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollState {
    pub offset: usize,
    following: bool,
    unread: bool,
    seen_revision: u64,
}
impl Default for ScrollState {
    fn default() -> Self {
        Self {
            offset: 0,
            following: true,
            unread: false,
            seen_revision: 0,
        }
    }
}
impl ScrollState {
    pub fn following(&self) -> bool {
        self.following
    }
    pub fn unread(&self) -> bool {
        self.unread
    }
    /// Called before composing a frame. New output only moves the viewport
    /// while following; otherwise it produces a return-to-latest affordance.
    pub fn observe(&mut self, revision: u64) {
        if revision == self.seen_revision {
            return;
        }
        self.seen_revision = revision;
        if self.following {
            self.offset = 0;
        } else {
            self.unread = true;
        }
    }
    pub fn scroll(&mut self, delta: i32, ceiling: usize) {
        self.offset = (self.offset as i32 - delta).clamp(0, ceiling as i32) as usize;
        self.following = self.offset == 0;
        if self.following {
            self.unread = false;
        }
    }
    pub fn latest(&mut self) {
        self.offset = 0;
        self.following = true;
        self.unread = false;
    }
    /// Adopt an offset that came from somewhere other than a scroll gesture —
    /// the anchor that holds the reader's rows across a row they opened. The
    /// tail is still the tail: an offset of `0` means following.
    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
        self.following = offset == 0;
        if self.following {
            self.unread = false;
        }
    }
    /// Session switches start at the new tail, not the old conversation's offset.
    pub fn reset(&mut self) {
        *self = Self {
            seen_revision: self.seen_revision,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_output_follows_only_at_the_tail() {
        let mut scroll = ScrollState::default();
        scroll.observe(1);
        assert_eq!(scroll.offset, 0);
        scroll.scroll(-4, 20);
        scroll.observe(2);
        assert_eq!(scroll.offset, 4);
        assert!(scroll.unread());
        scroll.observe(3);
        assert_eq!(scroll.offset, 4);
        scroll.latest();
        scroll.observe(4);
        assert_eq!((scroll.offset, scroll.unread()), (0, false));
    }
    #[test]
    fn returning_by_scrolling_restores_follow() {
        let mut scroll = ScrollState {
            offset: 2,
            following: false,
            unread: true,
            seen_revision: 1,
        };
        scroll.scroll(2, 5);
        assert!(scroll.following());
        assert!(!scroll.unread());
    }
}
