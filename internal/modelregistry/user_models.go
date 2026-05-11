package modelregistry

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/huichen/xihu/pkg/types"
)

// LoadUserModels reads a JSON file containing user-defined models and returns them.
// The file should contain an array of types.Model objects with the same JSON tags.
// Returns nil, nil if the file does not exist (non-error).
func LoadUserModels(path string) ([]types.Model, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("read models.json: %w", err)
	}

	var models []types.Model
	if err := json.Unmarshal(data, &models); err != nil {
		return nil, fmt.Errorf("parse models.json: %w", err)
	}

	return models, nil
}
