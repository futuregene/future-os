package extensions

// ---------------------------------------------------------------------------
// Inline extension loading — mirrors pi-mono loadExtensionFromFactory
// ---------------------------------------------------------------------------

// LoadExtensionFromFactory loads an extension from a factory function.
// This allows extensions to be defined inline (in code) without file I/O.
// Mirrors pi-mono's loadExtensionFromFactory.
func LoadExtensionFromFactory(factory ExtensionFactory, name string) Extension {
	return &factoryExtension{
		name:    name,
		factory: factory,
	}
}

// factoryExtension wraps an ExtensionFactory as an Extension.
type factoryExtension struct {
	name    string
	factory ExtensionFactory
	ext     Extension
}

func (f *factoryExtension) Name() string { return f.name }

func (f *factoryExtension) Init(ctx ExtensionContext) error {
	f.ext = f.factory()
	return f.ext.Init(ctx)
}

func (f *factoryExtension) Deinit() error {
	if f.ext != nil {
		return f.ext.Deinit()
	}
	return nil
}
