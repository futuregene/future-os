import { createContext, use } from "react";

/**
 * Whether `futureos-choice` blocks are drawn as cards.
 *
 * A setting, not a rendering detail: when the user turns choice cards off the
 * block must draw **nothing** — neither the card nor the fenced source it would
 * otherwise degrade to (design `docs/design/choice-cards.zh-CN.md` §3). The
 * parser still produces the node, so this one flag is what a reader sees; the
 * document itself is never rewritten, which is what keeps an old conversation's
 * cards renderable again once the switch goes back on.
 *
 * Default `true` because most callers (previews, artifacts, tests) draw a
 * document with no settings around them; the app itself provides the persisted
 * value from `AppShell`. It is `false` for a fresh install, which is the product
 * default.
 */
const ChoiceCardsContext = createContext(true);

export const ChoiceCardsProvider = ChoiceCardsContext.Provider;

/** True when a `futureos-choice` block may be drawn for this reader. */
export function useChoiceCardsEnabled(): boolean {
  return use(ChoiceCardsContext);
}
