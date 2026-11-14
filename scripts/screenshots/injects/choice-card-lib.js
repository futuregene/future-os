// Prototype DOM for the "choice card" design (docs/design/choice-cards.zh-CN.md).
//
// Nothing here is product code: it builds the card and the composer chips from
// the app's own design tokens so the screenshots show the real palette, type
// scale and surface treatment. A state file (choice-card-*.js) calls into these
// helpers; nothing here runs on its own.
(() => {
  const token = (name, fallback) =>
    getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
  const T = {
    line: token("--color-line", "#d9dee7"),
    lineSoft: token("--color-line-soft", "#e8edf4"),
    surface: token("--color-surface", "#ffffff"),
    surfaceSubtle: token("--color-surface-subtle", "#f1f4f8"),
    ink: token("--color-ink", "#172033"),
    inkSoft: token("--color-ink-soft", "#5d687a"),
    inkMuted: token("--color-ink-muted", "#818c9f"),
    accent: token("--color-accent", "#2563eb"),
    accentSoft: token("--color-accent-soft", "#e8f0ff"),
    success: token("--color-success", "#15803d"),
  };
  const font = getComputedStyle(document.body).fontFamily;

  const el = (tag, style, props = {}) => {
    const node = document.createElement(tag);
    Object.assign(node, props);
    if (style) Object.assign(node.style, style);
    return node;
  };

  const svg = (paths, { size = 14, stroke = 2 } = {}) => {
    const root = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    for (const [k, v] of Object.entries({
      viewBox: "0 0 24 24", width: size, height: size, fill: "none",
      stroke: "currentColor", "stroke-width": stroke, "stroke-linecap": "round",
      "stroke-linejoin": "round", "aria-hidden": "true",
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
    pencil: "M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z",
    listChecks: ["m3 7 2 2 4-4", "m3 17 2 2 4-4", "M13 6h8", "M13 12h8", "M13 18h8"],
    corner: "M9 10l-5 5 5 5M20 4v7a4 4 0 0 1-4 4H4",
    send: "M22 2 11 13M22 2l-7 20-4-9-9-4Z",
    plus: "M12 5v14M5 12h14",
  };

  // The fixture conversation: two papers and their conclusions, so the options
  // read as the reply's own next steps rather than as a generic menu.
  const QUESTION = {
    title: "接下来做哪一件？",
    hint: "选一个就放进输入框；也可以直接在输入框里写别的",
    multiple: false,
    options: [
      { id: "forest", label: "重画森林图", desc: "按 Dreher 2025 的样本量重算两篇的置信区间" },
      { id: "flow", label: "补一张实验流程图", desc: "把两篇的被试流程画成一张对照图" },
      { id: "table", label: "表里加一列样本量", desc: "Frank 42 / Dreher 186，标出测量手段" },
    ],
    free: "自己写…",
  };

  /** One inline pill (variant C): a whole option as a single tag. */
  const optionPill = (question, option, state) => {
    const chosen = state.picked.includes(option.id);
    const pill = el("div", {
      display: "inline-flex", alignItems: "center", gap: "6px", cursor: "pointer",
      padding: "5px 11px", borderRadius: "999px",
      border: `1px solid ${chosen ? T.accent : T.line}`,
      background: chosen ? T.accentSoft : T.surface,
      fontSize: "13px", lineHeight: "20px", fontFamily: font,
      color: chosen ? T.ink : T.inkSoft,
      fontWeight: chosen ? "500" : "400",
    });
    pill.setAttribute("data-choice-option", option.id);
    if (chosen) {
      const mark = svg(P.check, { size: 12, stroke: 3 });
      mark.style.color = T.accent;
      pill.appendChild(mark);
    }
    pill.appendChild(el("span", null, { textContent: option.label }));
    return pill;
  };

  /**
   * One option row. `style` picks the visual variant (see the variant sheet).
   * `numbers` adds the 1/2/3 keyboard hint; off by default because digits typed
   * in the composer are free text, not a menu key.
   */
  const optionRow = (question, option, state, style, { numbers = false } = {}) => {
    if (style === "pills") return optionPill(question, option, state);
    const chosen = state.picked.includes(option.id);
    const row = el("div", {
      display: "flex", alignItems: "flex-start", gap: "10px", cursor: "pointer",
      padding: style === "compact" ? "7px 10px" : "9px 11px",
      borderRadius: "8px",
      border: `1px solid ${chosen ? T.accent : "transparent"}`,
      background: chosen ? T.accentSoft : "transparent",
      transition: "background 120ms, border-color 120ms",
    });
    row.setAttribute("data-choice-option", option.id);

    // Marker: radio for single-select, checkbox for multi.
    const mark = el("span", {
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      width: "16px", height: "16px", marginTop: "2px", flexShrink: "0",
      borderRadius: question.multiple ? "4px" : "999px",
      border: `1.5px solid ${chosen ? T.accent : T.line}`,
      background: chosen ? T.accent : T.surface,
      color: "#fff",
    });
    if (chosen) mark.appendChild(svg(P.check, { size: 11, stroke: 3 }));
    row.appendChild(mark);

    const body = el("div", { minWidth: "0", flex: "1" });
    const label = el("div", {
      fontSize: "13px", lineHeight: "20px", fontWeight: chosen ? "500" : "400",
      color: chosen ? T.ink : T.ink, fontFamily: font,
    }, { textContent: option.label });
    body.appendChild(label);
    if (option.desc && style !== "compact") {
      body.appendChild(el("div", {
        fontSize: "12px", lineHeight: "18px", color: chosen ? T.inkSoft : T.inkMuted,
        fontFamily: font, marginTop: "1px",
      }, { textContent: option.desc }));
    }
    row.appendChild(body);

    // Optional keyboard affordance: the number a user types instead of clicking.
    if (style === "rows" && numbers) {
      row.appendChild(el("span", {
        fontSize: "11px", lineHeight: "16px", fontFamily: "ui-monospace, monospace",
        color: T.inkMuted, padding: "0 4px", borderRadius: "4px",
        background: T.surfaceSubtle, flexShrink: "0", marginTop: "2px",
      }, { textContent: String(question.options.indexOf(option) + 1) }));
    }
    return row;
  };

  /** The free-text row: it turns the card into the composer's own input. */
  const freeRow = (question, state, style) => {
    const row = el("div", {
      display: "flex", alignItems: "center", gap: "10px", padding: "9px 11px",
      borderRadius: "8px", cursor: "text",
      border: `1px dashed ${T.line}`,
      background: "transparent",
    });
    row.appendChild(svg(P.pencil, { size: 13 }));
    row.firstChild.style.color = T.inkMuted;
    row.appendChild(el("span", {
      fontSize: "13px", lineHeight: "20px", color: T.inkMuted, fontFamily: font,
    }, { textContent: question.free }));
    return row;
  };

  window.__choice = { T, font, el, svg, P, QUESTION, optionRow, freeRow };

  /**
   * Append the card to the last assistant turn; returns the card element.
   *
   * `free` draws the extra "write your own" row. Off in the final design: the
   * hint line already points at the composer, and a dashed row inside the card
   * reads as a field that is not one.
   */
  window.__choiceMount = (state, style, { free = false, numbers = false } = {}) => {
    const msgs = [...document.querySelectorAll("[data-message-id]")];
    const last = msgs[msgs.length - 1];
    const stack = last?.querySelector(".space-y-3");
    if (!stack) throw new Error("the last turn has no block stack to append to");

    const question = window.__choice.QUESTION;
    const card = el("div", {
      marginTop: "4px", borderRadius: "10px", overflow: "hidden",
      border: `1px solid ${T.line}`, background: T.surface,
      boxShadow: "0 1px 2px rgba(15, 23, 42, 0.04)",
      fontFamily: font,
    });
    card.setAttribute("data-choice-card", question.title);

    // Header: what is being asked, and how the answer is entered.
    const head = el("div", {
      display: "flex", alignItems: "center", gap: "6px",
      padding: "10px 12px 8px",
    });
    const glyph = svg(P.listChecks, { size: 14 });
    glyph.style.color = T.accent;
    head.appendChild(glyph);
    head.appendChild(el("span", {
      fontSize: "12px", fontWeight: "600", color: T.ink, letterSpacing: "0.01em",
    }, { textContent: question.title }));
    head.appendChild(el("span", {
      marginLeft: "auto", fontSize: "11px", color: T.inkMuted,
    }, { textContent: question.multiple ? "可多选" : "单选" }));
    card.appendChild(head);

    if (question.hint) {
      card.appendChild(el("div", {
        padding: "0 12px 8px", fontSize: "11px", lineHeight: "16px", color: T.inkMuted,
      }, { textContent: question.hint }));
    }

    const body = el("div", style === "pills"
      // Variant C: one wrapped row of tags. The cheapest option to scan, and the
      // only one that survives a narrow phone width without a second column.
      ? { display: "flex", flexWrap: "wrap", gap: "6px", padding: "0 12px 10px" }
      : { display: "flex", flexDirection: "column", gap: "2px", padding: "0 6px 6px" });
    for (const option of question.options)
      body.appendChild(optionRow(question, option, state, style, { numbers }));
    if (free && style !== "pills") body.appendChild(freeRow(question, state, style));
    card.appendChild(body);

    // Footer: the answer's destination, said before the click rather than after.
    if (state.picked.length) {
      const foot = el("div", {
        display: "flex", alignItems: "center", gap: "6px",
        padding: "8px 12px", borderTop: `1px solid ${T.lineSoft}`,
        background: T.surfaceSubtle,
      });
      foot.appendChild(svg(P.corner, { size: 12 })).style.color = T.inkMuted;
      foot.appendChild(el("span", {
        fontSize: "11px", color: T.inkSoft,
      }, { textContent: "已放进输入框，可以继续补充，也可以直接发送" }));
      card.appendChild(foot);
    }

    stack.appendChild(card);
    return card;
  };

  /**
   * The composer's chosen-answer chip.
   *
   * Mirrors the attachment chip's slot (same row, same height) because it is the
   * same kind of thing: something already decided that the message carries. It
   * differs in being editable in place — the pencil re-opens the question.
   */
  window.__choiceChip = (label, value, { editing = false } = {}) => {
    const chip = el("span", {
      display: "inline-flex", alignItems: "center", gap: "6px",
      maxWidth: "420px", padding: "3px 4px 3px 8px", borderRadius: "6px",
      fontSize: "12px", lineHeight: "18px", fontFamily: font,
      border: `1px solid ${editing ? T.accent : `${T.accent}40`}`,
      background: T.accentSoft, color: T.ink,
    });
    chip.setAttribute("data-choice-chip", label);
    chip.appendChild(el("span", { color: T.inkMuted, flexShrink: "0" }, { textContent: label }));
    chip.appendChild(el("span", {
      color: T.inkMuted, flexShrink: "0",
    }, { textContent: "·" }));
    chip.appendChild(el("span", {
      fontWeight: "500", overflow: "hidden", textOverflow: "ellipsis",
      whiteSpace: "nowrap",
    }, { textContent: value }));

    const edit = el("button", {
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      width: "18px", height: "18px", borderRadius: "4px", border: "none",
      background: editing ? T.surface : "transparent", cursor: "pointer",
      color: editing ? T.accent : T.inkMuted, padding: "0",
    }, { type: "button" });
    edit.setAttribute("aria-label", `修改「${label}」的选择`);
    edit.appendChild(svg(P.pencil, { size: 11 }));
    chip.appendChild(edit);

    const remove = el("button", {
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      width: "18px", height: "18px", borderRadius: "4px", border: "none",
      background: "transparent", cursor: "pointer", color: T.inkMuted, padding: "0",
    }, { type: "button" });
    remove.setAttribute("aria-label", `移除「${label}」的选择`);
    remove.appendChild(svg(P.x, { size: 12 }));
    chip.appendChild(remove);
    return chip;
  };

  /** Put a chip row above the composer's field, in the attachments' slot. */
  window.__choiceComposerRow = (chips) => {
    const form = document.querySelector("form");
    if (!form) throw new Error("no composer on screen");
    const field = form.querySelector('[contenteditable="true"], textarea');
    if (!field) throw new Error("the composer has no field");
    // The attachments row sits directly above the field; the chip row takes the
    // same slot, and the two stack when both exist. Find the form's own child
    // that holds the field — the local composer wraps it in a placeholder
    // overlay, the remote one keeps the textarea as a direct child — and insert
    // before that, so both composers take the row in the same place.
    let anchor = field;
    while (anchor.parentElement && anchor.parentElement !== form) anchor = anchor.parentElement;
    if (anchor.parentElement !== form) throw new Error("cannot find the composer's content column");
    const row = el("div", {
      display: "flex", flexWrap: "wrap", gap: "6px",
      padding: "0 4px 8px",
    });
    row.setAttribute("data-choice-row", "");
    for (const chip of chips) row.appendChild(chip);
    form.insertBefore(row, anchor);
    return row;
  };

  /** The re-open popover: the same question, anchored above the chip row. */
  window.__choicePopover = (state) => {
    const question = window.__choice.QUESTION;
    const form = document.querySelector("form");
    const row = document.querySelector("[data-choice-row]");
    if (!form || !row) throw new Error("no chip row to anchor the popover");

    const pop = el("div", {
      position: "absolute", zIndex: "30", width: "360px",
      borderRadius: "10px", border: `1px solid ${T.line}`, background: T.surface,
      boxShadow: "0 12px 28px rgba(15, 23, 42, 0.16)", overflow: "hidden",
      fontFamily: font,
    });
    pop.setAttribute("data-choice-popover", question.title);
    const head = el("div", {
      display: "flex", alignItems: "center", gap: "6px", padding: "9px 11px 6px",
    });
    const glyph = svg(P.pencil, { size: 12 });
    glyph.style.color = T.accent;
    head.appendChild(glyph);
    head.appendChild(el("span", { fontSize: "12px", fontWeight: "600", color: T.ink },
      { textContent: `修改「${question.title}」` }));
    head.appendChild(el("span", { marginLeft: "auto", fontSize: "11px", color: T.inkMuted },
      { textContent: "Esc 关闭" }));
    pop.appendChild(head);
    const body = el("div", { display: "flex", flexDirection: "column", gap: "2px", padding: "0 6px 6px" });
    for (const option of question.options) body.appendChild(window.__choice.optionRow(question, option, state, "compact"));
    pop.appendChild(body);
    const foot = el("div", {
      display: "flex", alignItems: "center", gap: "6px", padding: "8px 11px",
      borderTop: `1px solid ${T.lineSoft}`, background: T.surfaceSubtle,
    });
    foot.appendChild(el("span", { fontSize: "11px", color: T.inkSoft },
      { textContent: "换个选项，输入框里的内容跟着变" }));
    pop.appendChild(foot);

    document.body.appendChild(pop);
    const chipBox = row.getBoundingClientRect();
    pop.style.left = `${Math.round(chipBox.left + 4)}px`;
    pop.style.top = `${Math.round(chipBox.top - pop.getBoundingClientRect().height - 10)}px`;
    return pop;
  };
})();
