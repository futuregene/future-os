package tools

import (
	"bytes"
	"fmt"
	"os"
)

// OutputAccumulator buffers tool output in memory, spilling to a temp file when
// the buffer exceeds a configurable size limit. It is safe for sequential use
// (Write calls should be serialized by the caller).
type OutputAccumulator struct {
	buffer    bytes.Buffer
	maxBytes  int
	tempFile  *os.File
	truncated bool
}

// NewOutputAccumulator creates an accumulator that spills to disk once the
// in-memory buffer exceeds maxBytes.
func NewOutputAccumulator(maxBytes int) *OutputAccumulator {
	return &OutputAccumulator{maxBytes: maxBytes}
}

// Write appends p to the accumulator. If the total buffered data exceeds
// maxBytes, excess data is written to a temporary file and truncated is set.
func (a *OutputAccumulator) Write(p []byte) (int, error) {
	if a.truncated {
		// Already spilling — write directly to temp file
		return a.tempFile.Write(p)
	}

	remaining := a.maxBytes - a.buffer.Len()
	if remaining >= len(p) {
		// Everything fits in memory
		return a.buffer.Write(p)
	}

	// Partial fit: write what we can to buffer, spill the rest
	if remaining > 0 {
		a.buffer.Write(p[:remaining])
	}
	a.truncated = true

	// Create temp file on first spill
	var err error
	a.tempFile, err = os.CreateTemp("", "pi-output-*.txt")
	if err != nil {
		return 0, fmt.Errorf("output accumulator: create temp file: %w", err)
	}

	// Write the portion that didn't fit (and any future writes go to disk)
	n, err := a.tempFile.Write(p[remaining:])
	return remaining + n, err
}

// Snapshot returns the current output. If the buffer fit entirely in memory,
// returns just the buffer contents. If truncated, returns the in-memory portion
// plus a note pointing to the temp file path.
func (a *OutputAccumulator) Snapshot() string {
	if !a.truncated {
		return a.buffer.String()
	}
	tempPath := ""
	if a.tempFile != nil {
		tempPath = a.tempFile.Name()
	}
	return a.buffer.String() + fmt.Sprintf("\n[truncated, full output at %s]", tempPath)
}

// Close cleans up the temporary file if one was created. After Close the
// accumulator should not be used.
func (a *OutputAccumulator) Close() error {
	if a.tempFile != nil {
		name := a.tempFile.Name()
		a.tempFile.Close()
		return os.Remove(name)
	}
	return nil
}
