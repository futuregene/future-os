package types

import (
	"encoding/json"
	"fmt"
	"reflect"
	"strings"
)

// SchemaOf generates a JSON Schema from a Go struct using reflection.
// It mirrors TypeScript's TypeBox pattern: the struct definition IS the schema.
//
// Tag conventions:
//   json:"field_name"          — property name (required)
//   json:"field_name,omitempty" — optional property
//   jsonschema:"description=..." — human-readable description
//   jsonschema:"required"      — force this field as required even with omitempty
//
// Type mapping:
//   string   → {"type":"string"}
//   int      → {"type":"integer"}
//   float64  → {"type":"number"}
//   bool     → {"type":"boolean"}
//   []T      → {"type":"array","items":...}
//   struct   → {"type":"object","properties":...}
//   map[string]T → {"type":"object","additionalProperties":...}
//
// Use SchemaOf to replace hand-written JSON Schema strings in tool definitions.
// The schema is always in sync with the struct, and IDE autocomplete works on
// the struct definition.
func SchemaOf[T any]() json.RawMessage {
	var zero T
	t := reflect.TypeOf(zero)
	schema := buildSchema(t)
	b, _ := json.Marshal(schema)
	return b
}

func buildSchema(t reflect.Type) map[string]interface{} {
	// Dereference pointer
	if t.Kind() == reflect.Ptr {
		t = t.Elem()
	}

	switch t.Kind() {
	case reflect.String:
		return map[string]interface{}{"type": "string"}
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64,
		reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return map[string]interface{}{"type": "integer"}
	case reflect.Float32, reflect.Float64:
		return map[string]interface{}{"type": "number"}
	case reflect.Bool:
		return map[string]interface{}{"type": "boolean"}
	case reflect.Slice:
		if t.Elem().Kind() == reflect.Uint8 {
			return map[string]interface{}{"type": "string"}
		}
		return map[string]interface{}{
			"type":  "array",
			"items": buildSchema(t.Elem()),
		}
	case reflect.Map:
		return map[string]interface{}{
			"type":                 "object",
			"additionalProperties": buildSchema(t.Elem()),
		}
	case reflect.Struct:
		props := make(map[string]interface{})
		var required []string

		for i := 0; i < t.NumField(); i++ {
			f := t.Field(i)

			// Skip unexported fields
			if !f.IsExported() {
				continue
			}

			// Get property name from json tag
			jsonTag := f.Tag.Get("json")
			if jsonTag == "-" {
				continue
			}
			propName := f.Name
			omitempty := false
			if jsonTag != "" {
				parts := strings.Split(jsonTag, ",")
				propName = parts[0]
				for _, p := range parts[1:] {
					if p == "omitempty" {
						omitempty = true
					}
				}
			}

			// Build property schema
			propSchema := buildSchema(f.Type)

			// Add description from jsonschema tag
			jsTag := f.Tag.Get("jsonschema")
			if jsTag != "" {
				parts := strings.Split(jsTag, ",")
				for _, p := range parts {
					if strings.HasPrefix(p, "description=") {
						propSchema["description"] = p[len("description="):]
					}
					if p == "required" {
						omitempty = false // force required
					}
				}
			}

			props[propName] = propSchema

			// Determine if required
			if !omitempty {
				required = append(required, propName)
			}
		}

		result := map[string]interface{}{
			"type":       "object",
			"properties": props,
		}
		if len(required) > 0 {
			result["required"] = required
			// Ensure required is stable in JSON output
		}
		return result
	default:
		return map[string]interface{}{"type": "string"}
	}
}

// SchemaOfExample demonstrates SchemaOf usage. Remove after migration.
func SchemaOfExample() json.RawMessage {
	type BashParams struct {
		Command string `json:"command" jsonschema:"required,description=The shell command to execute"`
		Timeout int    `json:"timeout,omitempty" jsonschema:"description=Optional timeout in seconds"`
	}
	return SchemaOf[BashParams]()
}

// MustSchemaOf is like SchemaOf but panics on error (for init-time use).
// Since SchemaOf uses reflection which can't fail at the type level,
// this is identical to SchemaOf but signals intent for top-level var declarations.
func MustSchemaOf[T any]() json.RawMessage {
	return SchemaOf[T]()
}

// FormatSchema pretty-prints a json.RawMessage schema for debugging.
func FormatSchema(schema json.RawMessage) string {
	var m map[string]interface{}
	if err := json.Unmarshal(schema, &m); err != nil {
		return fmt.Sprintf("<invalid schema: %v>", err)
	}
	b, _ := json.MarshalIndent(m, "", "  ")
	return string(b)
}
