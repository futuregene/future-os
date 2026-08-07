export async function copyText(value: string) {
  if (!value)
    return;

  // Try execCommand first — more reliable in Tauri WKWebView than the
  // async clipboard API, which requires a secure context and explicit
  // clipboard-write permissions.
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "true");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.appendChild(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  document.body.removeChild(textarea);
  if (copied)
    return;

  // Fallback: async clipboard API (works in modern browsers + secure contexts).
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }

  throw new Error("Copy failed");
}
