//go:build !linux && !darwin

package extensions

// loadGoPlugin is a no-op on unsupported platforms.
func loadGoPlugin(path string, logger Logger) (Extension, error) {
	logger.Warn("Go plugins are not supported on this platform (skipping %q)", path)
	return nil, nil
}
