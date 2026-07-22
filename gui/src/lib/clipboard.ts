export async function copyText(value: string) {
  if (!value)
    return;

  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "true");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.appendChild(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  document.body.removeChild(textarea);
  // Surface the failure so callers can toast instead of silently believing the
  // copy succeeded.
  if (!copied)
    throw new Error("Copy command was rejected");
}
