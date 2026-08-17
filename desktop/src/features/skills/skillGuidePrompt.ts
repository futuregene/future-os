import type { Language } from "../../i18n";
import type { SkillGuide } from "../../integrations/skills/skillsClient";
import { getLanguage } from "../../i18n";
import { getSkillGuide } from "../../integrations/skills/skillsClient";

/**
 * The coach prompt references the skill manual with a bare `@name` token that
 * only the platform can author (the name differs per language). Replacing it
 * with a markdown link does two jobs at once: the chat renders a clickable
 * manual link (SafeLink), and the model receives the manual URL so it can
 * actually teach from it.
 */
const MANUAL_PLACEHOLDER: Record<Language, string> = {
  zh: "@技能使用手册",
  en: "@Skill User Manual",
};

const MANUAL_LINK_TEXT: Record<Language, string> = {
  zh: "技能使用手册",
  en: "Skill User Manual",
};

/** True for a plausible web URL; anything else is treated as "no link". */
function isWebUrl(value: string): boolean {
  return /^https?:\/\//i.test(value);
}

/**
 * Build the first message for a skill-coaching conversation. Takes the coach
 * prompt for the UI language (falling back to zh) and, when the platform
 * supplies an http(s) manual link, turns the prompt's `@manual` token into a
 * clickable markdown link. Returns the prompt unchanged when the manual value
 * is missing or not a URL, or when the prompt no longer carries the token.
 */
export function buildCoachPrompt(guide: SkillGuide, language: Language): string {
  const coach = guide.skills.coachPrompt;
  const promptLanguage: Language = coach[language] ? language : "zh";
  const prompt = coach[promptLanguage] || "";
  const manual = guide.skills.manual[promptLanguage] || "";
  if (!isWebUrl(manual))
    return prompt;
  const placeholder = MANUAL_PLACEHOLDER[promptLanguage];
  if (!prompt.includes(placeholder))
    return prompt;
  return prompt.replace(placeholder, `[${MANUAL_LINK_TEXT[promptLanguage]}](${manual})`);
}

/** Fetch the platform coach prompt for the UI language, ready to send. */
export async function fetchCoachPrompt(): Promise<string> {
  const guide = await getSkillGuide();
  return buildCoachPrompt(guide, getLanguage());
}

/** The skill manual URL for the UI language, or null when unset/non-URL. */
export async function fetchSkillManualUrl(): Promise<string | null> {
  const guide = await getSkillGuide();
  const manual = guide.skills.manual[getLanguage()];
  return isWebUrl(manual) ? manual : null;
}
