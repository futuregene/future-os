// v2 · the choice answer as a chip on a **queued** message's own row.
//
// Injected into the real `[data-send-queue]` panel after enqueuing a real message
// through the real flow (start a run → type → send → 加入发送队列), so the row's
// frame, numbering and controls are the product's, and only the part the design
// proposes — the answer chip in place of flattened text — is added.
//
// Why the answer needs to be structured here at all: `queueDraft()` hands the
// panel `editor.getContent()`, which is text, so today the answer is already the
// string `[接下来做哪一件？ → 重画森林图]` and the row's edit box can only edit
// that string. The reader can still delete the row (the × and the row removal
// are real); what the design adds is being able to *change the choice* without
// retyping the serialized form.
(() => {
  const T = window.__choiceTokens();
  const svg = window.__choiceSvg;
  const font = getComputedStyle(document.body).fontFamily;

  const row = document.querySelector("[data-send-queue] [data-queue-item]");
  if (!row) throw new Error("the queue panel has no row to attach a choice to");
  const column = row.querySelector("div.min-w-0.flex-1, div[class*='flex-1']");
  if (!column) throw new Error("cannot find the row's content column");

  const chip = document.createElement("span");
  chip.setAttribute("data-queue-choice", "接下来");
  Object.assign(chip.style, {
    display: "inline-flex", alignItems: "center", gap: "4px",
    padding: "1px 2px 1px 7px", marginBottom: "3px", borderRadius: "6px",
    background: T.accentSoft, color: T.accent,
    border: `1px solid ${T.accent}33`,
    fontSize: "12px", lineHeight: "18px", fontFamily: font,
    maxWidth: "100%", boxSizing: "border-box",
  });

  const title = document.createElement("span");
  title.style.color = T.inkMuted;
  title.style.flexShrink = "0";
  title.textContent = "接下来";
  chip.appendChild(title);

  const sep = document.createElement("span");
  sep.style.color = T.inkMuted;
  sep.textContent = "·";
  chip.appendChild(sep);

  const answer = document.createElement("span");
  answer.style.fontWeight = "500";
  answer.style.overflow = "hidden";
  answer.style.textOverflow = "ellipsis";
  answer.style.whiteSpace = "nowrap";
  answer.textContent = "重画森林图";
  chip.appendChild(answer);

  // Same two actions, same ink, as the composer's pill: one is "change which
  // answer this message carries", the other is already the row's own removal.
  const edit = document.createElement("button");
  edit.type = "button";
  edit.setAttribute("aria-label", "更换这条消息里的选择");
  Object.assign(edit.style, {
    display: "inline-flex", alignItems: "center", justifyContent: "center",
    width: "16px", height: "16px", border: "none", background: "transparent",
    padding: "0", color: T.accent, cursor: "pointer", flexShrink: "0",
  });
  edit.appendChild(svg("M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z", { size: 10 }));
  chip.appendChild(edit);

  column.insertBefore(chip, column.firstChild);

  // The row's own text becomes the words the reader added, since the answer is
  // no longer words — the serialized form is what the Agent receives, not what
  // the panel shows.
  const text = column.querySelector("p");
  if (text && text.textContent.trim().startsWith("[")) {
    text.textContent = "顺便把图注也改成中文";
  }
  return "queue row carries a choice chip";
})();
