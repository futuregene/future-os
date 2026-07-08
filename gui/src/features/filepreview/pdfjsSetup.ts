import * as pdfjs from "pdfjs-dist";

// Configure the PDF.js worker once for the whole app. Both the artifact preview
// (`features/artifacts/PdfPreview`) and the fullscreen file preview
// (`PdfScrollPreview`) import from here so the worker/WASM wiring lives in one place.
pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

// pdf.js v6 decodes JBIG2/JPEG2000/CCITT scanned images via WebAssembly loaded from
// this base dir (served by the `pdfjsWasm` Vite plugin). Without it, image-only PDFs
// render as a blank page. Absolute href so pdf.js's `new URL(name, wasmUrl)` resolves.
export const PDF_WASM_URL = new URL("pdfjs-wasm/", document.baseURI).href;

/** Decode a base64 string (from `read_file_base64`) into the byte buffer pdf.js wants. */
export function base64ToBytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

export { pdfjs };
