//! Where an entry's summary stops. No egui types, so the fast CI job tests it.
//!
//! # Why the cut is here and not in the producer
//!
//! A feed entry shows a lead-in, not the whole post. Putting that cut in `feed.rs` would make
//! it a lossy extractor, which `reader::title` rules out in as many words: *"`html::extract`
//! reports what the page contains, and [this] is a reading decision; a different renderer is
//! entitled to a different answer, and a lossy extractor is not recoverable."* It would also
//! delete the text from the plain renderer, which has no width pressure and no reason to lose
//! it. So the producer emits the whole summary and the reader decides how much of it to draw.
//!
//! # Why it is a shared function and not a loop inside `entries_ui`
//!
//! `ui::app::walk_blocks` walks `Entry::summary` in full to collect image sources. A budget
//! applied only where entries *draw* would leave every figure in the cut tail collected, named
//! in the load-images disclosure, and fetched: third-party requests for pictures the reader
//! cannot see, and a count in page info that does not match the screen. One function, two
//! callers, so the draw and the image walk cannot disagree about what the page contains.
//!
//! # Bytes
//!
//! [`ir::blocks_text_len`] sums `s.len()`, so this budget is bytes and is named bytes. Written
//! as "characters" it would cut a CJK or Cyrillic feed at roughly a third of its visible
//! length, and text length in this crate has one currency.

use crate::ir::Block;

/// The leading blocks of `summary` worth about `budget` bytes of text.
///
/// The budget is a **floor**, not a ceiling: blocks are taken until the running total reaches
/// it, and the block that reaches it is kept whole. Overshooting by one block is the intent —
/// the cut is always at a block boundary, so a paragraph is never left half-drawn and a list
/// never loses its last item.
///
/// `budget` of 0 is the whole summary. A reader who wants full entries has to be able to say
/// so: while an entry is cut its tail is not findable, because find-in-page walks widgets and
/// what is not drawn is not walked.
pub fn lead_in(summary: &[Block], budget: u16) -> &[Block] {
    if budget == 0 {
        return summary;
    }
    let budget = usize::from(budget);
    let mut spent = 0;
    for (i, b) in summary.iter().enumerate() {
        spent += crate::ir::blocks_text_len(std::slice::from_ref(b));
        if spent >= budget {
            return &summary[..=i];
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Inline;

    fn para(n: usize) -> Block {
        Block::Paragraph(vec![Inline::Text("x".repeat(n))])
    }

    #[test]
    fn an_entry_under_budget_is_untouched() {
        let s = [para(10), para(10), para(10)];
        assert_eq!(lead_in(&s, 1000).len(), 3);
        // And exactly at the budget, the last block still lands whole.
        assert_eq!(lead_in(&s, 30).len(), 3);
    }

    /// The cut is at a block boundary and never inside one, so a paragraph is never left
    /// half-drawn. The budget is a floor, which is why this is two blocks and not one.
    #[test]
    fn an_entry_over_budget_is_cut_at_a_block_boundary() {
        let s = [para(100), para(100), para(100), para(100)];
        assert_eq!(lead_in(&s, 150).len(), 2);
        assert_eq!(lead_in(&s, 100).len(), 1);
        assert_eq!(lead_in(&s, 1).len(), 1);
    }

    /// A reader who wants whole entries has to be able to ask for them: while an entry is cut,
    /// find-in-page cannot see its tail, because find walks widgets and the tail is not drawn.
    #[test]
    fn a_budget_of_zero_shows_the_whole_entry() {
        let s = [para(5_000), para(5_000)];
        assert_eq!(lead_in(&s, 0).len(), 2);
    }

    /// A block that carries no text of its own — a rule, a picture with no caption — does not
    /// spend the budget, so a summary made of them is not cut to its first one.
    #[test]
    fn a_textless_block_does_not_spend_the_budget() {
        let pics = [
            Block::Rule,
            Block::Figure {
                image: crate::ir::Image {
                    src: "a.png".into(),
                    alt: None,
                },
                caption: None,
            },
            Block::Rule,
        ];
        assert_eq!(lead_in(&pics, 100).len(), 3);
    }

    #[test]
    fn an_empty_summary_stays_empty() {
        assert!(lead_in(&[], 100).is_empty());
        assert!(lead_in(&[], 0).is_empty());
    }
}
