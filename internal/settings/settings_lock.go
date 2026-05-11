package settings

import (
	"fmt"
	"os"
	"time"
)

func lockPath(path string) string {
	return path + ".lock"
}

// LockSettings acquires an exclusive lock on a settings file.
// The lock is a best-effort mechanism: it creates a .lock file using O_EXCL.
// Returns an error if already locked by another process.
func LockSettings(path string) error {
	lockFile := lockPath(path)
	f, err := os.OpenFile(lockFile, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0644)
	if err != nil {
		if os.IsExist(err) {
			return fmt.Errorf("settings file is locked: %s", path)
		}
		return fmt.Errorf("lock settings: %w", err)
	}
	// Write lock metadata
	fmt.Fprintf(f, "%d\n%s\n", os.Getpid(), time.Now().Format(time.RFC3339))
	f.Close()
	return nil
}

// UnlockSettings releases the lock on a settings file.
func UnlockSettings(path string) error {
	lockFile := lockPath(path)
	if err := os.Remove(lockFile); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("unlock settings: %w", err)
	}
	return nil
}

// IsLocked checks if a settings file is currently locked.
func IsLocked(path string) bool {
	_, err := os.Stat(lockPath(path))
	return err == nil
}

// ---------------------------------------------------------------------------
// Settings migration
// ---------------------------------------------------------------------------

// MigrateSettings applies field name and format migrations from older settings formats.
//
// Migration history:
//   v1\u2192v2: queueMode \u2192 steeringMode, websockets transport value \u2192 sse
//   v2\u2192v3: skills object notation ({"file1": true}) \u2192 array notation (["file1"]),
//              retry.maxDelayMs field removed (moved to retry.provider.maxRetryDelayMs)
