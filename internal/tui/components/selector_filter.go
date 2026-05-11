package components

import (
	"strings"
)

// fuzzyScore computes a match score for pattern in s.
// Returns (match bool, score int). Lower score = better match.
// Rewards word-boundary matches, consecutive chars, exact match.
// Penalizes gaps between matches (TS pi-mono: fuzzyMatch).
func fuzzyScore(pattern, s string) (bool, int) {
	if pattern == "" {
		return true, 0
	}
	if len(pattern) > len(s) {
		return false, 0
	}

	score := 0
	queryIdx := 0
	lastMatchIdx := -1
	consecutive := 0

	for i := 0; i < len(s) && queryIdx < len(pattern); i++ {
		if s[i] == pattern[queryIdx] {
			// Word boundary check
			isWordBoundary := i == 0 || s[i-1] == ' ' || s[i-1] == '-' || s[i-1] == '_' || s[i-1] == '.' || s[i-1] == '/' || s[i-1] == ':'
			if isWordBoundary {
				score -= 10
			}

			// Consecutive match bonus
			if lastMatchIdx == i-1 {
				consecutive++
				score -= consecutive * 5
			} else {
				consecutive = 0
				if lastMatchIdx >= 0 {
					score += (i - lastMatchIdx - 1) * 2
				}
			}

			// Slight penalty for later matches
			score += i / 10

			lastMatchIdx = i
			queryIdx++
		}
	}

	if queryIdx < len(pattern) {
		return false, 0
	}

	// Exact match bonus
	if pattern == s {
		score -= 100
	}

	return true, score
}

// trySwappedAlphaNum tries alpha-numeric swap (e.g. "haiku3" → "3haiku").
func trySwappedAlphaNum(pattern string) string {
	// Split into letters then digits
	split := -1
	for i := 0; i < len(pattern)-1; i++ {
		if isLetter(pattern[i]) && isDigit(pattern[i+1]) {
			split = i + 1
			break
		}
		if isDigit(pattern[i]) && isLetter(pattern[i+1]) {
			split = i + 1
			break
		}
	}
	if split < 0 {
		return ""
	}
	return pattern[split:] + pattern[:split]
}

func isLetter(c byte) bool {
	return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

func isDigit(c byte) bool {
	return c >= '0' && c <= '9'
}

// fuzzyMatchItem tries to match a pattern against an item's label and description.
// Returns (match bool, score int). Lower score = better.
func fuzzyMatchItem(pattern, label, description string) (bool, int) {
	lower := strings.ToLower(pattern)

	// Try without tokenization first for simple queries
	if ok, score := fuzzyScore(lower, strings.ToLower(label)); ok {
		return true, score
	}
	if description != "" {
		if ok, score := fuzzyScore(lower, strings.ToLower(description)); ok {
			return true, score + 20 // slight penalty for description matches
		}
	}

	// Try alpha-num swap (e.g., "haiku3" matches "claude-haiku-3.5")
	if swapped := trySwappedAlphaNum(lower); swapped != "" {
		if ok, score := fuzzyScore(swapped, strings.ToLower(label)); ok {
			return true, score + 5
		}
	}

	return false, 0
}

type scoredItem struct {
	item  SelectorItem
	score int
}

// filteredItems returns items matching the filter, sorted by match quality (TS pi-mono: fuzzyFilter).
func (s ListSelector) filteredItems() []SelectorItem {
	if s.Filter == "" {
		return s.Items
	}
	lower := strings.ToLower(strings.TrimSpace(s.Filter))
	if lower == "" {
		return s.Items
	}

	// Split into space-separated tokens
	tokens := strings.Fields(lower)
	if len(tokens) == 0 {
		return s.Items
	}

	var results []scoredItem
	for _, item := range s.Items {
		totalScore := 0
		allMatch := true
		for _, token := range tokens {
			ok, score := fuzzyMatchItem(token, item.Label, item.Description)
			if !ok {
				allMatch = false
				break
			}
			totalScore += score
		}
		if allMatch {
			results = append(results, scoredItem{item, totalScore})
		}
	}

	// Sort by score (lower is better)
	sortResults(results)

	var items []SelectorItem
	for _, r := range results {
		items = append(items, r.item)
	}
	return items
}

func sortResults(results []scoredItem) {
	for i := 0; i < len(results); i++ {
		for j := i + 1; j < len(results); j++ {
			if results[i].score > results[j].score {
				results[i], results[j] = results[j], results[i]
			}
		}
	}
}
