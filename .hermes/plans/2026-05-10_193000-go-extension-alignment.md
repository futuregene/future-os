# Go Extension 系统完善计划：与 TypeScript pi-mono 对齐

## 目标
完善 xihu Go 版本的 extension 系统，修复关键缺陷，与 TypeScript pi-mono 的 extension 功能保持一致。

## 现状与差距

| 分类 | TS pi-mono | Go xihu | 差距 |
|------|-----------|---------|------|
| 工具合并 | 注册后自动合并到 agent loop | 注册到 globalRegistry 但**从未加入 loop.Tools** | 🔴 CRITICAL |
| Prompt 注入 | 通过 `resources_discover` 注入 | 注册到 registry 但**未被读取注入** | 🔴 CRITICAL |
| 扩展发现 | `~/.pi/extensions/`、`.pi/extensions/` 自动扫描 | 仅通过 `--extensions` 命令行指定 | 🟠 HIGH |
| 事件钩子 | 27 个事件类型，覆盖全生命周期 | 仅有 EventBus pub/sub | 🟠 HIGH |
| RPC get_commands | 返回 extension/skill/prompt 命令 | 返回空列表 | 🟠 HIGH |
| Extension API | 40+ 方法（sendMessage, setModel, setSessionName, etc.） | ~12 个注册方法 | 🟡 MEDIUM |

## 实现步骤（按优先级）

### Step 1: 工具合并到 Agent Loop

**文件**: `internal/engine/engine.go` (line 265-279)

在扩展加载完成后，将 `globalRegistry.GetAllTools()` 合并到 `loop.Tools`：

```go
// After extension loading (line 279):
if extRunner != nil && len(extRunner.Initialized()) > 0 {
    extTools := extensions.GetAllTools()
    toolList = append(toolList, extTools...)
    loop.Tools = toolList
}
```

同时支持 `AgentConfig.NoTools = "builtin"` 模式（仅内置工具，排除扩展工具）。

### Step 2: Prompt 注入到系统提示词

**文件**: `internal/prompt/prompt.go` 或 `internal/engine/engine.go`

在构建系统提示词时，读取 `globalRegistry.GetAllPrompts()`，将注册的 prompt 模板注入到系统提示词末尾：

```go
// 在 system prompt 构建后
for name, tmpl := range extensions.GetAllPrompts() {
    systemPrompt += fmt.Sprintf("\n\n[Extension Prompt: %s]\n%s", name, tmpl)
}
```

同时在 `resources_discover` 逻辑中添加对 skills/prompts 路径的发现。

### Step 3: 扩展自动发现

**文件**: `internal/extensions/loader.go`

添加 `DiscoverExtensionPaths()` 函数，自动扫描：
1. `~/.xihu/extensions/`（全局）
2. `{cwd}/.xihu/extensions/`（项目本地）
3. 递归扫描子目录中的 `extension.json`、`index.ts`（如支持）、`*.so`

在 `engine.NewEngine()` 中，如果 `opts.ExtensionPaths` 为空且 `!opts.NoExtensions`，自动调用发现。

### Step 4: RPC get_commands 完善

**文件**: `internal/rpc/server.go`、`internal/agentsession/agent_session.go`

让 `get_commands` 返回真实的命令列表：
- Extension slash commands（从 `extensions.GetAllSlashCommands()`）
- Prompt templates（从 `extensions.GetAllPrompts()`）
- Skills（从 skills 包）

类型对齐 TS 的 `RpcSlashCommand`：
```go
type RpcSlashCommand struct {
    Name        string    `json:"name"`
    Description string    `json:"description,omitempty"`
    Source      string    `json:"source"` // "extension"|"prompt"|"skill"
    SourceInfo  SourceInfo `json:"sourceInfo"`
}
```

### Step 5: 事件钩子系统

**文件**: `internal/extensions/events.go`（新建）

在 `ExtensionRunner` 上添加事件发射方法，对齐 TS 的关键事件：

| Go 方法 | TS 事件 | 优先级 |
|---------|---------|--------|
| `EmitBeforeAgentStart()` | `before_agent_start` | 🔴 |
| `EmitAgentStart()` | `agent_start` | 🔴 |
| `EmitAgentEnd()` | `agent_end` | 🔴 |
| `EmitContext()` | `context` | 🟠 |
| `EmitBeforeProviderRequest()` | `before_provider_request` | 🟠 |
| `EmitAfterProviderResponse()` | `after_provider_response` | 🟠 |
| `EmitToolCall()` | `tool_call` | 🟡 |
| `EmitToolResult()` | `tool_result` | 🟡 |
| `EmitUserBash()` | `user_bash` | 🟡 |
| `EmitInput()` | `input` | 🟡 |
| `EmitSessionStart()` | `session_start` | 🟢 |
| `EmitSessionShutdown()` | `session_shutdown` | 🟢 |

在 Engine/Agent/Loop 的关键路径插入事件调用。

### Step 6: Extension API 扩展

**文件**: `internal/extensions/extensions.go`

为 `ExtensionContext` 添加方法对齐 TS `ExtensionAPI`：

| 方法 | 说明 |
|------|------|
| `SetSessionName(name)` | 设置会话名称 |
| `GetSessionName()` | 获取会话名称 |
| `SendUserMessage(content, opts)` | 发送用户消息（steer/followUp） |
| `SetModel(provider, modelID)` | 切换模型 |
| `GetThinkingLevel()` | 获取当前思维级别 |
| `SetThinkingLevel(level)` | 设置思维级别 |
| `GetActiveTools()` | 获取当前激活工具列表 |
| `Abort()` | 中断当前操作 |
| `IsIdle()` | 检查 agent 是否空闲 |

### Step 7: 测试

| 测试文件 | 测试内容 |
|----------|----------|
| `internal/extensions/extensions_test.go` | Tool 合并、Prompt 注入、事件发射 |
| `internal/extensions/loader_test.go` | 自动发现、扫描逻辑 |
| `internal/engine/engine_test.go` | 扩展工具合并到 loop |
| `internal/rpc/rpc_test.go` | get_commands 返回正确分类 |

## 涉及文件

| 文件 | 变更类型 |
|------|----------|
| `internal/engine/engine.go` | 修改：工具合并、提示词注入、自动发现 |
| `internal/extensions/extensions.go` | 修改：Extension API 扩展 |
| `internal/extensions/runner.go` | 修改：事件发射方法 |
| `internal/extensions/loader.go` | 修改：自动发现 |
| `internal/extensions/events.go` | **新建**：事件系统 |
| `internal/agent/loop.go` | 修改：事件钩子插入点 |
| `internal/rpc/server.go` | 修改：get_commands 实现 |
| `internal/agentsession/agent_session.go` | 修改：GetCommands |
| `pkg/rpcclient/types.go` | 修改：SlashCommandInfo 对齐 TS |
| `internal/extensions/*_test.go` | **新建**：测试 |

## 风险

- **循环导入**: `engine` ↔ `extensions` 已有正常依赖，添加事件发射不应导致循环
- **线程安全**: 事件发射在 agent loop 的热路径上，需保证不阻塞
- **TS 对齐度**: Go 的 interface 模式 vs TS 的 event-driven 模式有本质差异，对齐到"功能等价"而非"API 同构"
