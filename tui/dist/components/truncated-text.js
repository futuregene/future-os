/**
 * TruncatedText component — single-line text truncated to fit width with ellipsis.
 */
import { truncateToWidth, visibleWidth } from "../utils.js";
export class TruncatedText {
    text;
    paddingX;
    paddingY;
    constructor(text, paddingX = 0, paddingY = 0) {
        this.text = text;
        this.paddingX = paddingX;
        this.paddingY = paddingY;
    }
    setText(text) {
        this.text = text;
    }
    invalidate() {
        // No cached state
    }
    render(width) {
        const result = [];
        const emptyLine = " ".repeat(width);
        for (let i = 0; i < this.paddingY; i++) {
            result.push(emptyLine);
        }
        const avail = Math.max(1, width - this.paddingX * 2);
        let singleLine = this.text;
        const nl = this.text.indexOf("\n");
        if (nl !== -1)
            singleLine = this.text.substring(0, nl);
        const displayText = truncateToWidth(singleLine, avail);
        const leftPad = " ".repeat(this.paddingX);
        const rightPad = " ".repeat(this.paddingX);
        const line = leftPad + displayText + rightPad;
        const pad = Math.max(0, width - visibleWidth(line));
        result.push(line + " ".repeat(pad));
        for (let i = 0; i < this.paddingY; i++) {
            result.push(emptyLine);
        }
        return result;
    }
}
