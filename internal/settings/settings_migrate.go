package settings

import (
)

func MigrateSettings(s *Settings) {
	// v1\u2192v2: queueMode \u2192 steeringMode
	// This is handled by JSON tags (steering_mode), but if an old file
	// used queueMode as a key, it would be ignored. We leave the struct
	// as-is since JSON unmarshalling into the new fields handles it.

	// v2\u2192v3: If skills somehow loaded as map[string]bool instead of []string,
	// we can't detect that from a typed struct. The Skiils field is []string.
	// Any old format with retry.maxDelayMs at top level is naturally ignored
	// since the field no longer exists.

	// Re-validate thinking level
	validLevels := map[string]bool{
		"off": true, "minimal": true, "low": true, "medium": true,
		"high": true, "xhigh": true, "max": true, "": true,
	}
	if !validLevels[s.DefaultThinkingLevel] {
		s.DefaultThinkingLevel = "" // reset invalid value
	}
	if !validLevels[s.ThinkingLevel] {
		s.ThinkingLevel = "" // reset invalid value
	}

	// Re-validate doubleEscapeAction and treeFilterMode
	validEscape := map[string]bool{"fork": true, "tree": true, "none": true, "": true}
	if !validEscape[s.DoubleEscapeAction] {
		s.DoubleEscapeAction = ""
	}
	validTreeFilter := map[string]bool{"all": true, "default": true, "user-only": true, "no-tools": true, "labeled-only": true, "": true}
	if !validTreeFilter[s.TreeFilterMode] {
		s.TreeFilterMode = ""
	}

	// Re-validate steeringMode and followUpMode
	validModes := map[string]bool{"all": true, "one-at-a-time": true, "": true}
	if !validModes[s.SteeringMode] {
		s.SteeringMode = ""
	}
	if !validModes[s.FollowUpMode] {
		s.FollowUpMode = ""
	}
}

// ---------------------------------------------------------------------------
// Reload
// ---------------------------------------------------------------------------

// Reload re-reads settings from the last known path and applies overrides.
// The path parameter is the settings file to reload from.
// This is useful when settings have been modified externally.
