/**
 * Container - a composable component that renders child components.
 * Used for building up the layout hierarchy.
 */
export class Container {
    children = [];
    addChild(child) {
        this.children.push(child);
    }
    removeChild(child) {
        const idx = this.children.indexOf(child);
        if (idx >= 0)
            this.children.splice(idx, 1);
    }
    clear() {
        this.children = [];
    }
    render(width, _height) {
        const lines = [];
        for (const child of this.children) {
            lines.push(...child.render(width, 0));
        }
        return lines;
    }
    getHeight() {
        return this.children.reduce((sum, c) => sum + c.getHeight(), 0);
    }
}
