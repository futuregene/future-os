//go:build linux || darwin

package extensions

import (
	"fmt"
	"plugin"
)

// loadGoPlugin opens a Go plugin (.so) file and extracts the Extension symbol.
func loadGoPlugin(path string, _ Logger) (Extension, error) {
	p, err := plugin.Open(path)
	if err != nil {
		return nil, fmt.Errorf("open plugin: %w", err)
	}

	// Look up the "Extension" symbol (must be of type ExtensionFactory)
	sym, err := p.Lookup("Extension")
	if err != nil {
		return nil, fmt.Errorf("plugin %q does not export 'Extension' symbol: %w", path, err)
	}

	factory, ok := sym.(*ExtensionFactory)
	if !ok {
		// Try dereferencing (some plugins export a value, not a pointer)
		if factoryVal, ok2 := sym.(ExtensionFactory); ok2 {
			factory = &factoryVal
		} else {
			return nil, fmt.Errorf("plugin %q symbol 'Extension' is not of type ExtensionFactory (got %T)", path, sym)
		}
	}

	ext := (*factory)()
	if ext == nil {
		return nil, fmt.Errorf("plugin %q factory returned nil extension", path)
	}

	return ext, nil
}
