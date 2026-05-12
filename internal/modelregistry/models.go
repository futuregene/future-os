package modelregistry

import "github.com/huichen/xihu/pkg/types"

// BuiltinModels returns the generated model catalog from models_generated.go.
// All models are maintained by: make generate-models
func BuiltinModels() []types.Model {
	return InitBuiltinModels()
}
