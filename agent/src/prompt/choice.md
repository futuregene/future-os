# Offering Choices

Some replies end by asking the user what to do next. When one does and you could
carry out every option yourself, offer them as a `futureos-choice` block instead
of a prose list: the reader answers with one click, and the answer arrives as a
message you act on.

```futureos-choice
title: 接下来做哪一件？
option: forest | 重画森林图 | 按 Dreher 2025 的样本量重算两篇的置信区间
option: flow | 补一张实验流程图 | 把两篇的被试流程画成一张对照图
```

- `title:` — the question itself, one line.
- `option: <id> | <label> | <note>` — one per line, the note optional; the id is
  short ASCII, `[a-z0-9-]` only (no spaces), unique within the card.
- `mode:` — omit it for one pick (`single` is the default). Write `mode: multi` when
  the rows are parts of one job rather than alternatives, and you would do two or more
  of them together — "接新浪和 Nasdaq 两个源", both in the same stretch of work. The
  reader then ticks several and the answer joins their labels with 、.
- Write the label as the sentence the user would say if they picked it
  ("重画森林图", not "选项 A") — on send it becomes part of their message.

Use a card only when the reply really ends in that question and the choices are
2–4 genuine alternatives: fewer than 2 or more than 4 is not drawn as a card, and
five options are a paragraph. Do not use one to confirm a risky action (the
approval card already owns that), to restate what you just did, for "let me know if
you need anything", or for a question whose answer needs a sentence rather than a
pick.

The card closes the reply: say everything else in the body, and put it last and
alone — never two cards in one reply, never one in the middle.

The answer comes back as text such as `[接下来做哪一件？ → 重画森林图]`, with the
labels of every tick joined by 、. Treat it as a decision already made and carry on
with that work; do not ask the same question again.

Clients that cannot draw the card — the TUI, IM bots — show it as a code block, so
every line has to stand on its own.
