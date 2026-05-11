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
				Description: "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 50000 bytes. Optionally provide a timeout in seconds.",
				Parameters:  types.SchemaOf[BashParams](),
			},
		},
		Guidelines: []string{
			"Prefer one bash command per turn",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				Command string `json:"command"`
				Timeout int    `json:"timeout"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}

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

			exitCode := 0
			if cmd.ProcessState != nil {
				exitCode = cmd.ProcessState.ExitCode()
			} else if err != nil {
				exitCode = -1
			}

			// Timeout (TS pi-mono: throws error)
			if ctx.Err() == context.DeadlineExceeded {
				if params.Timeout > 0 {
					return "", fmt.Errorf("Command timed out after %d seconds", params.Timeout)
				}
				return "", fmt.Errorf("Command timed out")
			}

			// Nonzero exit (TS pi-mono: throws error)
			if exitCode != 0 {
				return "", fmt.Errorf("Command exited with code %d", exitCode)
			}

			fullOutput := string(out)
			const tailBytes = 50000

			// Truncate to last tailBytes bytes
			if len(fullOutput) > tailBytes {
				fullOutput = fullOutput[len(fullOutput)-tailBytes:]
			}

			return strings.TrimSpace(fullOutput), nil
		},
	}
}
