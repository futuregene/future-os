// v3 · the choice answer as a chip on a **queued** message's own row (desktop).
//
// Injected into the real `[data-send-queue]` panel after enqueuing a real message
// through the real flow (start a run → type → send → 加入发送队列), so the row's
// frame, numbering and controls are the product's, and only what the design
// proposes — the answer as a chip instead of flattened text — is added.
//
// Two corrections over the v2 sketch of this row:
//
// 1. **The chip is the button.** v2 gave the chip its own pencil, and the row
//    already has a pencil meaning "edit the words" (`composer.queue.edit`). Two
//    pencils a few pixels apart meaning different things is exactly the ambiguity
//    this design is supposed to remove. So the chip's body re-opens the options
//    and carries a chevron saying "this opens something"; the row's pencil keeps
//    its one job.
// 2. **Removal is already the row's.** The chip offers no × either: dropping the
//    whole message is the row's bin, and dropping only the choice is the options'
//    own "no choice" entry. Nothing is drawn twice.
//
// Why the answer needs to be structured here at all: `queueDraft()` hands the
// panel `editor.getContent()`, which is text, so today the answer is already the
// string `[接下来做哪一件？ → 重画森林图]` and the row's edit box can only edit
// that string. The reader can already delete the row; what the design adds is
// being able to *change the choice* without retyping the serialized form.
(() => {
  const T = window.__choiceTokens();
  const font = getComputedStyle(document.body).fontFamily;

  const row = document.querySelector("[data-send-queue] [data-queue-item]");
  if (!row) throw new Error("the queue panel has no row to attach a choice to");
  const column = row.querySelector("div.min-w-0.flex-1, div[class*='flex-1']");
  if (!column) throw new Error("cannot find the row's content column");

  const title = "接下来";
  const label = "重画森林图";

  const chip = document.createElement("button");
  chip.type = "button";
  chip.setAttribute("data-queue-choice", title);
  chip.setAttribute("aria-label", `更换这条消息里的选择：${title} · ${label}`);
  chip.title = `更换这条消息里的选择：${title} · ${label}`;
  // A button, so it must not inherit the row's text styles.
  Object.assign(chip.style, {
    display: "inline-flex", alignItems: "center", gap: "4px",
    padding: "1px 6px", marginBottom: "3px", borderRadius: "6px",
    background: T.accentSoft, color: T.accent,
    border: `1px solid ${T.accent}33`,
    fontSize: "12px", lineHeight: "18px", fontFamily: font,
    maxWidth: "100%", boxSizing: "border-box",
    cursor: "pointer", textAlign: "left",
  });

  const titleEl = document.createElement("span");
  titleEl.style.color = T.inkMuted;
  titleEl.style.flexShrink = "0";
  titleEl.textContent = title;
  chip.appendChild(titleEl);

  const sep = document.createElement("span");
  sep.style.color = T.inkMuted;
  sep.textContent = "·";
  chip.appendChild(sep);

  const answer = document.createElement("span");
  answer.style.fontWeight = "600";
  answer.style.overflow = "hidden";
  answer.style.textOverflow = "ellipsis";
  answer.style.whiteSpace = "nowrap";
  answer.textContent = label;
  chip.appendChild(answer);

  // A chevron, not a pencil: "opens something", and unmistakably not the row's
  // edit-the-words control.
  const chev = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  for (const [k, v] of Object.entries({
    viewBox: "0 0 24 24", width: 10, height: 10, fill: "none",
    stroke: "currentColor", "stroke-width": 2.5, "stroke-linecap": "round",
    "stroke-linejoin": "round", "aria-hidden": "true",
  })) chev.setAttribute(k, v);
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "m6 9 6 6 6-6");
  chev.appendChild(path);
  chev.style.flexShrink = "0";
  chip.appendChild(chev);

  column.insertBefore(chip, column.firstChild);

  // The row's own text becomes the words the reader added, since the answer is no
  // longer words — the serialized form is what the Agent receives, not what the
  // panel shows.
  const text = column.querySelector("p");
  if (text && text.textContent.trim().startsWith("[")) {
    text.textContent = "顺便把图注也改成中文";
  }
  return "queue row carries a choice chip";
})();
