/**
 * Probe: are the machine icons adjacent to their titles, or pushed to the right
 * edge? Measured in the rendered app — the class list says what was intended,
 * the boxes say what the browser did.
 */
export async function run(): Promise<unknown> {
  const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

  // Open the remote page, then the machine list, so the merged list is drawn.
  const rail = document.querySelectorAll("[data-activity-rail-scroll='true']");
  if (rail.length === 0)
    return { error: "no rail" };
  await wait(600);

  const out: Record<string, unknown> = {};

  // The device selector starts on "all devices" (the merged list).
  const rows = [...document.querySelectorAll("[data-testid='merged-conversations'] > *")];
  out.rowCount = rows.length;
  out.rows = rows.slice(0, 6).map((row) => {
    const title = row.querySelector("span");
    const icon = row.querySelector("[data-testid='remote-source-icon']");
    const titleBox = title?.getBoundingClientRect();
    const iconBox = icon?.getBoundingClientRect();
    const rowBox = row.getBoundingClientRect();
    return {
      text: (title?.textContent ?? "").slice(0, 22),
      titleRight: titleBox ? Math.round(titleBox.right) : null,
      iconLeft: iconBox ? Math.round(iconBox.left) : null,
      gapToIcon: titleBox && iconBox ? Math.round(iconBox.left - titleBox.right) : null,
      rowRight: Math.round(rowBox.right),
      iconToRowRight: iconBox ? Math.round(rowBox.right - iconBox.right) : null,
    };
  });

  // Remote workspace group headings: name then machine icon.
  const headings = [...document.querySelectorAll("[data-testid='merged-group-heading']")];
  out.headingCount = headings.length;
  out.headings = headings.slice(0, 4).map((name) => {
    const header = name.closest("div")!;
    const icon = [...header.querySelectorAll("span[aria-hidden]")]
      .find(span => (span.textContent ?? "").trim() !== "");
    const nameBox = name.getBoundingClientRect();
    const iconBox = icon?.getBoundingClientRect();
    const headerBox = header.getBoundingClientRect();
    return {
      name: (name.textContent ?? "").slice(0, 20),
      nameRight: Math.round(nameBox.right),
      iconLeft: iconBox ? Math.round(iconBox.left) : null,
      gapToIcon: iconBox ? Math.round(iconBox.left - nameBox.right) : null,
      headerRight: Math.round(headerBox.right),
      iconToHeaderRight: iconBox ? Math.round(headerBox.right - iconBox.right) : null,
    };
  });

  return out;
}
