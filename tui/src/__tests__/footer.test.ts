/**
 * Unit tests for the Footer component.
 * Tests rendering with various data combinations and width constraints.
 */
import { describe, test, expect } from "bun:test";
import { Footer } from "../components/footer.js";
import { stripAnsiCodes, visibleWidth } from "../utils.js";

describe("Footer", () => {
  test("renders with minimal data", () => {
    const footer = new Footer(80);
    footer.setData({});
    const lines = footer.render(80);
    expect(lines).toHaveLength(1);
    expect(visibleWidth(lines[0])).toBeLessThanOrEqual(80);
  });

  test("renders cwd", () => {
    const footer = new Footer(80);
    footer.setData({ cwd: "/home/user/project" });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("/home/user/project");
  });

  test("renders home-relative cwd with ~", () => {
    const home = process.env.HOME || "";
    if (!home) return; // skip if no HOME set
    const footer = new Footer(80);
    footer.setData({ cwd: home + "/projects/foo" });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("~/projects/foo");
  });

  test("renders model name (shortened)", () => {
    const footer = new Footer(80);
    footer.setData({ model: "anthropic/claude-sonnet-4" });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("claude-sonnet-4");
    expect(text).not.toContain("anthropic/");
  });

  test("renders thinking level when not off", () => {
    const footer = new Footer(80);
    footer.setData({ model: "openai/gpt-4o", thinking: "high" });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("high");
  });

  test("does not render thinking level when off", () => {
    const footer = new Footer(80);
    footer.setData({ model: "openai/gpt-4o", thinking: "off" });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).not.toContain("off");
  });

  test("renders token stats", () => {
    const footer = new Footer(80);
    footer.setData({ tokensIn: 5000, tokensOut: 12000 });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("↑5k");
    expect(text).toContain("↓12k");
  });

  test("renders cost", () => {
    const footer = new Footer(80);
    footer.setData({ totalCost: 0.1234 });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("¥0.123");
  });

  test("renders context usage", () => {
    const footer = new Footer(80);
    footer.setData({ contextTokens: 50000, contextWindow: 128000, contextPercent: 39 });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("50k");
    expect(text).toContain("128k");
  });

  test("renders auto compaction indicator", () => {
    const footer = new Footer(80);
    footer.setData({
      contextTokens: 50000,
      contextWindow: 128000,
      contextPercent: 39,
      autoCompactionEnabled: true,
    });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("(auto)");
  });

  test("renders spinner when streaming", () => {
    const footer = new Footer(80);
    footer.setData({ streaming: true, spinnerFrame: 0 });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("⠋");
  });

  test("renders tool elapsed time", () => {
    const footer = new Footer(80);
    footer.setData({ toolElapsed: 5 });
    const text = stripAnsiCodes(footer.render(80)[0]);
    expect(text).toContain("5s");
  });

  test("never exceeds terminal width", () => {
    const footer = new Footer(40);
    footer.setData({
      cwd: "/very/long/path/to/a/deeply/nested/directory/structure/that/keeps/going",
      model: "anthropic/claude-sonnet-4-20250514",
      thinking: "xhigh",
      streaming: true,
      spinnerFrame: 0,
      tokensIn: 999000,
      tokensOut: 999000,
      totalCost: 12.345,
      contextTokens: 99000,
      contextWindow: 128000,
      contextPercent: 77,
      autoCompactionEnabled: true,
    });
    const lines = footer.render(40);
    expect(visibleWidth(lines[0])).toBeLessThanOrEqual(40);
  });

  test("right side stays visible with long cwd", () => {
    const footer = new Footer(60);
    footer.setData({
      cwd: "/extremely/deeply/nested/path/that/goes/on/and/on/forever/and/ever",
      model: "openai/gpt-4o",
      contextTokens: 50000,
      contextWindow: 128000,
      contextPercent: 39,
    });
    const text = stripAnsiCodes(footer.render(60)[0]);
    expect(text).toContain("50k");
    expect(text).toContain("128k");
  });

  test("fmtTokens formats large numbers", () => {
    const footer = new Footer(120);
    footer.setData({ tokensIn: 1500000, tokensOut: 500 });
    const text = stripAnsiCodes(footer.render(120)[0]);
    expect(text).toContain("1.5M");
    expect(text).toContain("500");
  });

  test("fmtTokens formats small numbers", () => {
    const footer = new Footer(120);
    footer.setData({ tokensIn: 42, tokensOut: 999 });
    const text = stripAnsiCodes(footer.render(120)[0]);
    expect(text).toContain("42");
    expect(text).toContain("999");
  });

  test("getHeight is always 1", () => {
    const footer = new Footer(80);
    expect(footer.getHeight()).toBe(1);
  });

  test("cache token stats render", () => {
    const footer = new Footer(120);
    footer.setData({ tokensCacheR: 3000, tokensCacheW: 2000 });
    const text = stripAnsiCodes(footer.render(120)[0]);
    expect(text).toContain("R3k");
    expect(text).toContain("W2k");
  });
});
