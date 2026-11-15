// The chosen answer as a chip on a **queued** row, on the phone.
//
// Injected into the phone's real `[data-testid=send-queue]` panel after the real
// fixture seated eight waiting rows, so the frame, numbering, the row's own
// pencil (edit the words) and bin (remove the row) are the product's. Only what
// the design proposes is added.
//
// Why the chip is itself the button here, with no pencil of its own: the row
// already carries a pencil meaning "edit the words". A second pencil two inches
// away, meaning "change which option", is the ambiguity this design exists to
// remove — so the chip's own body re-opens the options (it looks pressable and
// says what it is to a screen reader), and the row's pencil keeps its one job.
(() => {
  const ACCENT = "#2563eb";
  const ACCENT_SOFT = "#e8f0ff";
  const LINE = "#d9dee7";
  const INK = "#172033";
  const INK_MUTED = "#606c82";
  const FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif";

  const row = document.querySelector("[data-testid=send-queue-item]");
  if (!row) throw new Error("the queue panel has no row");
  // The row's copy column: the flex child that holds the message text.
  const copy = row.children[1];
  if (!copy) throw new Error("the row has no copy column");

  const chip = document.createElement("div");
  chip.setAttribute("data-queue-choice", "接下来");
  chip.setAttribute("role", "button");
  chip.setAttribute("aria-label", "更换这条消息里的选择：接下来 · 重画森林图");
  Object.assign(chip.style, {
    display: "inline-flex", alignItems: "center", gap: "4px",
    alignSelf: "flex-start",
    marginBottom: "3px", padding: "2px 7px",
    borderRadius: "6px",
    background: ACCENT_SOFT, border: `1px solid ${ACCENT}33`,
    fontFamily: FONT, fontSize: "11px", lineHeight: "17px",
    maxWidth: "100%", boxSizing: "border-box", cursor: "pointer",
  });

  const title = document.createElement("span");
  title.textContent = "接下来";
  title.style.color = INK_MUTED;
  title.style.flexShrink = "0";
  chip.appendChild(title);

  const sep = document.createElement("span");
  sep.textContent = "·";
  sep.style.color = LINE;
  chip.appendChild(sep);

  const answer = document.createElement("span");
  answer.textContent = "重画森林图";
  answer.style.color = ACCENT;
  answer.style.fontWeight = "600";
  answer.style.overflow = "hidden";
  answer.style.textOverflow = "ellipsis";
  answer.style.whiteSpace = "nowrap";
  chip.appendChild(answer);

  // The affordance is a chevron, not a pencil: "this opens something", and it
  // cannot be confused with the row's edit-the-words pencil.
  const chev = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  for (const [k, v] of Object.entries({
    viewBox: "0 0 24 24", width: 10, height: 10, fill: "none",
    stroke: ACCENT, "stroke-width": 2.5, "stroke-linecap": "round",
    "stroke-linejoin": "round",
  })) chev.setAttribute(k, v);
  const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
  p.setAttribute("d", "m6 9 6 6 6-6");
  chev.appendChild(p);
  chev.style.flexShrink = "0";
  chip.appendChild(chev);

  copy.insertBefore(chip, copy.firstChild);

  // The row's text is the serialized form while the answer is only text; with a
  // real choice it becomes the words the reader added, which is what the row now
  // shows (the chip answers the rest).
  for (const el of copy.querySelectorAll("div")) {
    const text = (el.textContent || "").trim();
    if (text.startsWith("[next]") || text.startsWith("[future-goal]") || text.startsWith("[")) {
      el.textContent = "顺便把图注也改成中文";
      break;
    }
  }
  return "queue row carries a choice chip";
})();
