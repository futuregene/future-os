package tools

import (
"context"
"encoding/json"
"fmt"
"os"
"os/exec"
"strings"
"time"

"github.com/huichen/xihu/pkg/types"
)

func BashTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "bash",
				Description: "Execute a shell command in the project directory",
				Parameters: types.SchemaOf[BashParams](),
			},
		},
		Guidelines: []string{
			"Prefer one bash command per turn",
			"Check exit codes",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				Command string `json:"command"`
				Timeout int    `json:"timeout"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}

			// Timeout is optional; if unspecified or <= 0, run without timeout.
			// The context serves as an AbortSignal that can cancel the command.
			var ctx context.Context
			var cancel context.CancelFunc
			if params.Timeout > 0 {
				ctx, cancel = context.WithTimeout(context.Background(), time.Duration(params.Timeout)*time.Second)
			} else {
				ctx, cancel = context.WithCancel(context.Background())
			}
			defer cancel()

			cmd := exec.CommandContext(ctx, "bash", "-c", params.Command)
			cmd.Dir, _ = os.Getwd()
			out, err := cmd.CombinedOutput()

			// Determine exit code
			exitCode := 0
			if cmd.ProcessState != nil {
				exitCode = cmd.ProcessState.ExitCode()
			} else if err != nil {
				exitCode = -1
			}

			// Handle timeout case
			if ctx.Err() == context.DeadlineExceeded {
				if exitCode == 0 {
					exitCode = -1
				}
			}

			fullOutput := string(out)
			const spillThreshold = 10000
			const tailBytes = 50000

			var result strings.Builder

			// Line 1: exit code
			fmt.Fprintf(&result, "exit code: %d\n", exitCode)

			// Line 2 (optional): temp file path if output exceeds spill threshold
			if len(fullOutput) > spillThreshold {
				tmpFile, tmpErr := os.CreateTemp("", "pi-bash-*.txt")
				if tmpErr != nil {
					fmt.Fprintf(&result, "[failed to spill output: %v]\n", tmpErr)
				} else {
					if _, tmpErr := tmpFile.WriteString(fullOutput); tmpErr != nil {
						tmpFile.Close()
						os.Remove(tmpFile.Name())
						fmt.Fprintf(&result, "[failed to spill output: %v]\n", tmpErr)
					} else {
						tmpFile.Close()
						fmt.Fprintf(&result, "[full output at %s]\n", tmpFile.Name())
					}
				}
			}

			// Line 3+: output — truncated to last tailBytes bytes
			if len(fullOutput) > tailBytes {
				fullOutput = fullOutput[len(fullOutput)-tailBytes:]
			}
			result.WriteString(fullOutput)

			return result.String(), nil
		},
	}
}

