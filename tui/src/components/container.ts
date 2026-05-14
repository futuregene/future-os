/**
 * Container - a composable component that renders child components.
 * Used for building up the layout hierarchy.
 */

export interface Component {
  render(width: number, height: number): string[];
  getHeight(): number;
}

export class Container implements Component {
  private children: Component[] = [];

  addChild(child: Component): void {
    this.children.push(child);
  }

  removeChild(child: Component): void {
    const idx = this.children.indexOf(child);
    if (idx >= 0) this.children.splice(idx, 1);
  }

  clear(): void {
    this.children = [];
  }

  render(width: number, _height: number): string[] {
    const lines: string[] = [];
    for (const child of this.children) {
      lines.push(...child.render(width, 0));
    }
    return lines;
  }

  getHeight(): number {
    return this.children.reduce((sum, c) => sum + c.getHeight(), 0);
  }
}
