// Prototype DOM for the phone (react-native-web) choice card.
//
// Same intent as `choice-card-lib.js` on the desktop: build the card and the
// composer chip out of the phone's own palette and metrics so the capture shows
// what the real RN component would look like. Values mirror
// `mobile/src/theme/tokens.ts` (light palette, spacing, radius).
(() => {
  const C = {
    surface: "#ffffff", surfaceSubtle: "#f1f4f8", line: "#d9dee7",
    lineSoft: "#e8edf4", ink: "#172033", inkSoft: "#5d687a", inkMuted: "#606c82",
    accent: "#2563eb", accentSoft: "#e8f0ff", focus: "#93c5fd",
  };
  const font = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif";

  const el = (style, text) => {
    const node = document.createElement("div");
    if (style) Object.assign(node.style, style);
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const svg = (paths, size = 14, color = C.accent) => {
    const root = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    for (const [k, v] of Object.entries({
      viewBox: "0 0 24 24", width: size, height: size, fill: "none", stroke: color,
      "stroke-width": 2, "stroke-linecap": "round", "stroke-linejoin": "round",
    })) root.setAttribute(k, v);
    root.style.flexShrink = "0";
    for (const d of [].concat(paths)) {
      const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
      p.setAttribute("d", d);
      root.appendChild(p);
    }
    return root;
  };
  const P = {
    check: "M20 6 9 17l-5-5",
    x: "M18 6 6 18M6 6l12 12",
    listChecks: ["m3 7 2 2 4-4", "m3 17 2 2 4-4", "M13 6h8", "M13 12h8", "M13 18h8"],
    pencil: "M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z",
  };

  const QUESTION = {
    title: "接下来做哪一件？",
    hint: "点一个填进输入框，也可以直接说别的",
    options: [
      { id: "forest", label: "重画森林图", desc: "按 Dreher 2025 的样本量重算置信区间" },
      { id: "flow", label: "补一张实验流程图", desc: "把两篇的被试流程画成对照图" },
      { id: "table", label: "表里加一列样本量", desc: "Frank 42 / Dreher 186" },
    ],
  };

  /** The inverted transcript's cell list, and where a new bottom cell goes. */
  const transcript = () => {
    const scroller = [...document.querySelectorAll("div")]
      .filter(d => d.scrollHeight > d.clientHeight + 20 && d.clientHeight > 200)[0];
    if (!scroller) throw new Error("no transcript scroller");
    const list = scroller.firstElementChild;
    if (!list) throw new Error("the scroller has no list");
    return { scroller, list };
  };

  /**
   * One option row (v2).
   *
   * v1 marked a chosen row three ways at once — accent fill, accent border and a
   * white check on a solid blue disc — which reads as a completed task, not as a
   * choice, and shouts on a 390pt screen. v2 keeps exactly one row-level cue (the
   * `accentSoft` tint the palette documents as the selected background, with no
   * border on top of it) and puts the selection where a single-select control
   * belongs: a ring with a dot inside it.
   */
  const row = (option, picked) => {
    const chosen = picked.includes(option.id);
    const box = el({
      display: "flex", flexDirection: "row", alignItems: "flex-start", gap: "10px",
      padding: "10px 12px", borderRadius: "10px",
      border: "1px solid transparent",
      background: chosen ? C.accentSoft : "transparent",
      minHeight: "44px", boxSizing: "border-box",
    });
    box.setAttribute("data-choice-option", option.id);
    if (chosen) box.setAttribute("data-choice-picked", "");
    // The ring: always drawn, so the row does not jump when it becomes chosen.
    const mark = el({
      width: "18px", height: "18px", marginTop: "1px", flexShrink: "0",
      borderRadius: "999px", display: "flex", alignItems: "center",
      justifyContent: "center", boxSizing: "border-box",
      border: `${chosen ? "2px" : "1.5px"} solid ${chosen ? C.accent : C.line}`,
      background: "transparent",
    });
    if (chosen) mark.appendChild(el({
      width: "8px", height: "8px", borderRadius: "999px", background: C.accent,
    }));
    box.appendChild(mark);
    const body = el({ display: "flex", flexDirection: "column", gap: "1px", minWidth: "0" });
    body.appendChild(el({
      fontSize: "14px", lineHeight: "20px", color: C.ink,
      fontWeight: chosen ? "500" : "400", fontFamily: font,
    }, option.label));
    body.appendChild(el({
      fontSize: "12px", lineHeight: "17px",
      color: chosen ? C.inkSoft : C.inkMuted, fontFamily: font,
    }, option.desc));
    box.appendChild(body);
    return box;
  };

  window.__mchoice = {
    /**
     * Append the card under the last reply.
     *
     * The transcript is an `inverted` list: react-native-web flips the scroll
     * view and flips each cell back, and it lays the cells out column-reverse.
     * So a cell that reads upright carries the flip itself and takes the *first*
     * DOM position to land at the visual bottom.
     */
    mount(picked, { withChip = false } = {}) {
      const { list } = transcript();
      const cell = el({
        transform: "matrix(1, 0, 0, -1, 0, 0)",
        display: "flex", flexDirection: "column", padding: "0 16px 12px",
      });
      cell.setAttribute("data-choice-card", QUESTION.title);

      const card = el({
        display: "flex", flexDirection: "column",
        background: C.surface, borderRadius: "12px",
        border: `1px solid ${C.line}`, overflow: "hidden",
        boxShadow: "0 1px 2px rgba(15,23,42,0.05)",
      });
      const head = el({ display: "flex", flexDirection: "row", alignItems: "center", gap: "6px", padding: "10px 12px 6px" });
      head.appendChild(svg(P.listChecks, 15));
      head.appendChild(el({ fontSize: "13px", fontWeight: "600", color: C.ink, fontFamily: font }, QUESTION.title));
      head.appendChild(el({ marginLeft: "auto", fontSize: "11px", color: C.inkMuted, fontFamily: font }, "单选"));
      card.appendChild(head);
      card.appendChild(el({ padding: "0 12px 8px", fontSize: "12px", lineHeight: "17px", color: C.inkMuted, fontFamily: font }, QUESTION.hint));
      const body = el({ display: "flex", flexDirection: "column", gap: "2px", padding: "0 6px 8px" });
      for (const option of QUESTION.options) body.appendChild(row(option, picked));
      card.appendChild(body);
      if (picked.length) {
        // v1 said "已填进输入框" here, but the chip it pointed at sat outside the
        // field — the sentence promised a place the answer was not. The field on a
        // phone is a plain textarea and cannot hold a pill, so the answer rides in
        // the dock's chip row; the footer names *that*, and "并" is the true
        // relationship rather than a guess about the field's contents.
        card.appendChild(el({
          padding: "8px 12px", borderTop: `1px solid ${C.lineSoft}`, background: C.surfaceSubtle,
          fontSize: "11px", lineHeight: "16px", color: C.inkSoft, fontFamily: font,
        }, "已选，答案在下面的输入框上方；可以直接发送，也可以补充文字"));
      }
      cell.appendChild(card);
      list.insertBefore(cell, list.firstChild);
      if (withChip) window.__mchoice.chip("接下来", QUESTION.options.find(o => o.id === picked[0]).label);
      return cell;
    },

    /**
     * The chosen answer, in the dock's chip row above the field.
     *
     * A phone field is a `textarea`, so unlike the desktop editor it cannot hold
     * an atomic pill; the row is therefore genuinely a second place the answer
     * lives, and the design says so instead of pretending otherwise. What the row
     * must do is count as content: `canSend`/`hasDraft` are
     * `!!message.trim() || attachments.length > 0`, so requirement (d) is met by
     * the answer being a draft item the dock counts — the same way an attachment
     * makes an empty field sendable.
     *
     * v1 differences this fixes: the chip wore a `focus`-blue border next to the
     * card's accent row (two different blues for one state), and its pencil was
     * accent while its × was grey — same weight of action, two weights of ink.
     */
    chip(label, value) {
      const ta = document.querySelector("textarea");
      if (!ta) throw new Error("no field in the dock");
      // The field's own row, one level up: the chip row sits above it, inside the
      // dock's padded surface.
      const fieldRow = ta.parentElement?.parentElement;
      if (!fieldRow) throw new Error("cannot find the dock's field row");
      const wrap = el({ display: "flex", flexDirection: "row", flexWrap: "wrap", gap: "6px", padding: "6px 4px 0" });
      wrap.setAttribute("data-choice-row", "");
      const chip = el({
        display: "inline-flex", flexDirection: "row", alignItems: "center", gap: "4px",
        borderRadius: "8px", border: `1px solid ${C.line}`, background: C.surfaceSubtle,
        padding: "4px 4px 4px 9px", fontSize: "12px", lineHeight: "18px", fontFamily: font,
        maxWidth: "100%", minHeight: "32px", boxSizing: "border-box",
      });
      chip.setAttribute("data-choice-chip", label);
      // The question stays, muted and allowed to truncate: a thread can hold
      // several cards, and without it the answer would not say what it answers.
      // The answer itself is the emphatic half, so it truncates last.
      chip.appendChild(el({ color: C.inkMuted, flexShrink: "0", maxWidth: "40%", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }, label));
      chip.appendChild(el({ color: C.line, flexShrink: "0" }, "·"));
      chip.appendChild(el({ color: C.ink, fontWeight: "600", minWidth: "0", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }, value));
      // Both actions carry the same ink. The × is a plain glyph rather than a
      // button-looking chip, so the pair reads as "edit / remove this answer".
      const edit = el({ display: "inline-flex", width: "24px", height: "24px", alignItems: "center", justifyContent: "center", flexShrink: "0" });
      edit.setAttribute("aria-label", `修改「${label}」的选择`);
      edit.appendChild(svg(P.pencil, 12, C.inkSoft));
      chip.appendChild(edit);
      const remove = el({ display: "inline-flex", width: "24px", height: "24px", alignItems: "center", justifyContent: "center", flexShrink: "0" });
      remove.setAttribute("aria-label", `移除「${label}」的选择`);
      remove.appendChild(svg(P.x, 13, C.inkSoft));
      chip.appendChild(remove);
      wrap.appendChild(chip);
      fieldRow.parentElement.insertBefore(wrap, fieldRow);
      return chip;
    },

    /** The bottom sheet the pencil opens on a phone. */
    sheet(picked) {
      const scrim = el({
        position: "fixed", inset: "0", background: C.overlay, zIndex: "40",
        display: "flex", flexDirection: "column", justifyContent: "flex-end",
      });
      scrim.style.background = "rgba(15,23,42,0.45)";
      const sheet = el({
        background: C.surface, borderTopLeftRadius: "16px", borderTopRightRadius: "16px",
        padding: "14px 12px 20px", display: "flex", flexDirection: "column", gap: "4px",
      });
      sheet.setAttribute("data-choice-sheet", QUESTION.title);
      const head = el({ display: "flex", flexDirection: "row", alignItems: "center", gap: "6px", padding: "0 4px 8px" });
      head.appendChild(svg(P.pencil, 14));
      head.appendChild(el({ fontSize: "14px", fontWeight: "600", color: C.ink, fontFamily: font }, `修改「${QUESTION.title}」`));
      sheet.appendChild(head);
      for (const option of QUESTION.options) sheet.appendChild(row(option, picked));
      sheet.appendChild(el({
        padding: "10px 4px 0", fontSize: "12px", lineHeight: "17px", color: C.inkSoft, fontFamily: font,
      }, "换个选项，输入框里的内容跟着变"));
      scrim.appendChild(sheet);
      document.body.appendChild(scrim);
      return sheet;
    },
  };
})();
