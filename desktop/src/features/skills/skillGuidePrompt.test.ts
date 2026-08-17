import { describe, expect, it } from "vitest";
import { buildCoachPrompt } from "./skillGuidePrompt";

function guide(overrides: {
  coachZh?: string;
  coachEn?: string;
  manualZh?: string;
  manualEn?: string;
} = {}): Parameters<typeof buildCoachPrompt>[0] {
  return {
    links: { help: "https://example.com/help" },
    skills: {
      coachPrompt: {
        zh: overrides.coachZh ?? "你是教练，参考手册：@技能使用手册",
        en: overrides.coachEn ?? "You are the coach. Manual: @Skill User Manual",
      },
      manual: {
        zh: overrides.manualZh ?? "https://example.com/manual-zh",
        en: overrides.manualEn ?? "https://example.com/manual-en",
      },
    },
  };
}

describe("buildCoachPrompt", () => {
  it("replaces the zh manual token with a markdown link", () => {
    const prompt = buildCoachPrompt(guide(), "zh");
    expect(prompt).toBe("你是教练，参考手册：[技能使用手册](https://example.com/manual-zh)");
  });

  it("replaces the en manual token with a markdown link", () => {
    const prompt = buildCoachPrompt(guide(), "en");
    expect(prompt).toBe("You are the coach. Manual: [Skill User Manual](https://example.com/manual-en)");
  });

  it("returns the prompt unchanged when the manual is not a URL", () => {
    const prompt = buildCoachPrompt(guide({ manualZh: "技能" }), "zh");
    expect(prompt).toBe("你是教练，参考手册：@技能使用手册");
  });

  it("returns the prompt unchanged when the token is absent", () => {
    const prompt = buildCoachPrompt(guide({ coachZh: "你是教练，开始学习吧" }), "zh");
    expect(prompt).toBe("你是教练，开始学习吧");
  });

  it("falls back to the zh prompt when the language prompt is empty", () => {
    const prompt = buildCoachPrompt(guide({ coachEn: "" }), "en");
    expect(prompt).toBe("你是教练，参考手册：[技能使用手册](https://example.com/manual-zh)");
  });

  it("returns an empty string when both prompts are empty", () => {
    const prompt = buildCoachPrompt(guide({ coachZh: "", coachEn: "" }), "zh");
    expect(prompt).toBe("");
  });
});
