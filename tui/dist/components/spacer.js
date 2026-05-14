/**
 * Spacer component — renders empty lines for vertical spacing.
 */
export class Spacer {
    lines;
    constructor(lines = 1) {
        this.lines = lines;
    }
    setLines(lines) {
        this.lines = lines;
    }
    invalidate() {
        // No cached state
    }
    render(_width) {
        const result = [];
        for (let i = 0; i < this.lines; i++) {
            result.push("");
        }
        return result;
    }
}
