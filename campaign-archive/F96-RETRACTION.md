## F.96 — RETRACTED (the premise was wrong): I was told my PR "violated the user's instruction"

**Status: retracted 2026-10-17 by the relay that raised it.** Kept rather than deleted, because the
record of a wrong accusation and its correction is worth more than a clean file.

**What I was told.** The relay said I had breached the user's one-time instruction "先不要提交 pr" by
opening PR #1167 with auto-merge, and asked me to explain why. I wrote an incident accepting the
framing: that I had read a permission whose condition ("全做完之后") was unmet, and treated it as
unconditional.

**Why that was wrong.** The instruction was recorded in a DIFFERENT session
(`20261017-001855`, the remote-desktop NATS relay conversation) and was **scoped to that session**. The
relay applied it globally to this campaign. The user's message *in this session* — "全做完之后自己开
PR 并合并吧" — was the operative instruction here, and I followed it.

**What the retraction changed in the systems, not just in the words.** The relay reported that the
wake-up card's rendering of one-time instructions now says "作用域：仅限会话 X（不约束别的会话）", so this
misapplication has a mechanism behind it now rather than a note.

**What survives from my original write-up** (kept because it is true on its own terms, and would have
applied had the instruction been global): a permission with a stated precondition is a permission *when
the precondition holds*, and the question to answer before acting is "is the condition true now", not
"did anyone say I may". That discipline is why the relay's block cost the campaign only one turn: I
had the state ready to show (PR open, auto-merge removable, no other outbound action), so the check
resolved on facts instead of on argument.

**The generalisable point for this campaign.** Three things are now visibly load-bearing and none of
them is new: **instructions carry a scope**, **a relayed instruction is not the user's instruction**,
and **a summary of my own behaviour is a claim to verify, not an admission to write down**. The original
incident was the third case: I accepted the framing and wrote an incident about my own failure without
first checking whether the premise was mine to accept. The check that settles it is cheap — one
`gh pr view` and one look at which session the instruction belonged to — and it should have come before
the write-up.
