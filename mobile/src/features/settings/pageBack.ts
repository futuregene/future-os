/**
 * One step back inside a settings page.
 *
 * The settings stack owns its routes, but a page can have levels of its own
 * (the Tasks page opens a task over its list). Such a page registers this with
 * the stack, so the system back gesture and the header arrow leave that level
 * first — a swipe out of a task returns to the task list, not to the settings
 * home. `goBack` answers whether it consumed the step.
 *
 * Its own module because both sides need the type and one of them imports the
 * other: the stack renders the page, and the page must not import the stack.
 */
export type SettingsPageBack = { goBack(): boolean };
