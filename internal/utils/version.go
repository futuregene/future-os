package utils

import (
	"fmt"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"
)

// Version is the current xihu version.
const Version = "0.3.0"

// VersionCheckResult holds the result of a version check.
type VersionCheckResult struct {
	Current string
	Latest  string
	Newer   bool
	URL     string
}

// CheckVersion compares the current version with the latest release from GitHub.
// If offline or check is skipped via XIHU_SKIP_VERSION_CHECK, returns nil.
func CheckVersion() *VersionCheckResult {
	if os.Getenv("XIHU_SKIP_VERSION_CHECK") != "" || os.Getenv("XIHU_OFFLINE") != "" {
		return nil
	}

	latest, url, err := fetchLatestVersion()
	if err != nil || latest == "" {
		return nil
	}

	current := strings.TrimPrefix(Version, "v")
	latestVer := strings.TrimPrefix(latest, "v")

	if compareSemver(current, latestVer) >= 0 {
		return nil // current >= latest
	}

	return &VersionCheckResult{
		Current: Version,
		Latest:  latest,
		Newer:   true,
		URL:     url,
	}
}

// fetchLatestVersion fetches the latest version tag from GitHub releases.
func fetchLatestVersion() (version, url string, err error) {
	client := &http.Client{Timeout: 5 * time.Second}

	// Use GitHub API to get latest release
	req, err := http.NewRequest("GET", "https://api.github.com/repos/huichen/xihu/releases/latest", nil)
	if err != nil {
		return "", "", err
	}
	req.Header.Set("Accept", "application/vnd.github.v3+json")
	req.Header.Set("User-Agent", "xihu-version-check")

	resp, err := client.Do(req)
	if err != nil {
		return "", "", err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", "", fmt.Errorf("status %d", resp.StatusCode)
	}

	// Parse simple response for tag_name
	buf := make([]byte, 2048)
	n, _ := resp.Body.Read(buf)
	body := string(buf[:n])

	// Extract "tag_name":"v1.2.3"
	tagIdx := strings.Index(body, `"tag_name"`)
	if tagIdx < 0 {
		return "", "", fmt.Errorf("no tag_name in response")
	}

	rest := body[tagIdx:]
	colonIdx := strings.Index(rest, ":")
	if colonIdx < 0 {
		return "", "", fmt.Errorf("malformed tag_name")
	}
	rest = rest[colonIdx+1:]

	// Find quoted value
	startQuote := strings.Index(rest, `"`)
	if startQuote < 0 {
		return "", "", fmt.Errorf("no quote in tag_name")
	}
	rest = rest[startQuote+1:]
	endQuote := strings.Index(rest, `"`)
	if endQuote < 0 {
		return "", "", fmt.Errorf("unterminated tag_name")
	}
	tag := rest[:endQuote]

	// Extract html_url
	urlIdx := strings.Index(body, `"html_url"`)
	htmlURL := ""
	if urlIdx >= 0 {
		urlRest := body[urlIdx:]
		urlColon := strings.Index(urlRest, ":")
		if urlColon >= 0 {
			urlRest = urlRest[urlColon+1:]
			urlStart := strings.Index(urlRest, `"`)
			if urlStart >= 0 {
				urlRest = urlRest[urlStart+1:]
				urlEnd := strings.Index(urlRest, `"`)
				if urlEnd >= 0 {
					htmlURL = urlRest[:urlEnd]
				}
			}
		}
	}

	return tag, htmlURL, nil
}

// compareSemver compares two semver strings (without "v" prefix).
// Returns -1 if a < b, 0 if a == b, 1 if a > b.
func compareSemver(a, b string) int {
	parseParts := func(s string) []int {
		parts := strings.Split(s, ".")
		nums := make([]int, len(parts))
		for i, p := range parts {
			// Strip any pre-release suffix
			if idx := strings.IndexAny(p, "-+"); idx >= 0 {
				p = p[:idx]
			}
			nums[i], _ = strconv.Atoi(p)
		}
		return nums
	}

	aParts := parseParts(a)
	bParts := parseParts(b)

	maxLen := len(aParts)
	if len(bParts) > maxLen {
		maxLen = len(bParts)
	}

	for i := 0; i < maxLen; i++ {
		av, bv := 0, 0
		if i < len(aParts) {
			av = aParts[i]
		}
		if i < len(bParts) {
			bv = bParts[i]
		}
		if av < bv {
			return -1
		}
		if av > bv {
			return 1
		}
	}
	return 0
}
