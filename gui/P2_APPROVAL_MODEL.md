# P2 Approval Model Design

更新时间：2026-06-18

## 概述

P2 升级了审批模型，引入结构化 action payload 和沙盒边界信息，为未来的沙盒执行、自动审批策略和规则引擎预留了数据模型。

## 核心变更

### 1. 结构化 Action Payload

每个审批请求现在包含结构化的 `action` 字段，替代了原来的纯文本 `requested_action`。

**Action 结构：**

```typescript
interface ApprovalAction {
  tool: string;           // 工具名称（bash, write, edit 等）
  category: string;       // 操作类别
  summary?: string;       // 操作摘要
  command?: string;       // shell 命令（仅 shell_command 类别）
  paths?: string[];       // 文件路径列表（仅文件操作类别）
  writes?: Array<{        // 写入操作详情
    path: string;
    preview?: string;     // 内容预览（最多 200 字符）
  }>;
  deletes?: Array<{       // 删除操作详情
    path: string;
  }>;
  scope?: {
    cwd: string;
    insideWorkspace: boolean;
    estimatedBlastRadius: "low" | "medium" | "high";
  };
}
```

**Action 类别：**

- `shell_command` - Shell 命令执行
- `file_write` - 文件写入
- `file_delete` - 文件删除
- `outside_workspace_read` - 工作区外读取
- `outside_workspace_write` - 工作区外写入
- `network_access` - 网络访问
- `data_access` - 数据访问
- `batch_operation` - 批量操作

### 2. 沙盒边界信息

每个审批请求包含 `sandbox_boundary` 字段，描述操作与沙盒边界的关系。

**Sandbox Boundary 结构：**

```typescript
interface SandboxBoundary {
  mode: string;                    // 当前沙盒模式
  insideSandbox: boolean;          // 是否在沙盒内
  violation?: string | null;       // 违反的边界类型
  cwd: string;                     // 当前工作目录
  writableRoots?: string[];        // 可写根目录列表
}
```

**沙盒模式（预留）：**

- `read-only` - 只读模式
- `workspace-write` - 工作区写入模式（当前默认）
- `danger-full-access` - 完全访问模式

**违反类型：**

- `shell_command_not_in_allowlist` - Shell 命令不在白名单
- `outside_workspace_write` - 工作区外写入
- `outside_workspace_read` - 工作区外读取
- `network_access` - 网络访问

### 3. 决策范围和来源

新增字段追踪审批决策的范围和来源：

- `decisionScope`: `once` | `session` | `always`（当前仅支持 `once`）
- `decisionSource`: `user` | `rule` | `sandbox`（当前仅支持 `user`）
- `reviewer`: `user` | `auto_review`（当前仅支持 `user`）

## 数据库 Schema 变更

### Migration 003: approval_model_v2

```sql
-- 新增列
ALTER TABLE approval_requests ADD COLUMN action_category TEXT;
ALTER TABLE approval_requests ADD COLUMN action_payload TEXT;
ALTER TABLE approval_requests ADD COLUMN sandbox_boundary TEXT;
ALTER TABLE approval_requests ADD COLUMN reviewer TEXT NOT NULL DEFAULT 'user';
ALTER TABLE approval_requests ADD COLUMN decision_scope TEXT NOT NULL DEFAULT 'once';
ALTER TABLE approval_requests ADD COLUMN decision_source TEXT NOT NULL DEFAULT 'user';

-- 预留表（当前未使用）
CREATE TABLE IF NOT EXISTS sandbox_config (
  id TEXT PRIMARY KEY,
  workspace_id TEXT REFERENCES workspaces(id),
  mode TEXT NOT NULL DEFAULT 'workspace-write',
  writable_roots TEXT,
  network_access INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS approval_policy_config (
  id TEXT PRIMARY KEY,
  workspace_id TEXT REFERENCES workspaces(id),
  policy TEXT NOT NULL DEFAULT 'on-request',
  reviewer TEXT NOT NULL DEFAULT 'user',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS approval_rules (
  id TEXT PRIMARY KEY,
  workspace_id TEXT REFERENCES workspaces(id),
  scope TEXT NOT NULL,
  match_kind TEXT NOT NULL,
  match_value TEXT NOT NULL,
  decision TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  expires_at INTEGER
);
```

## Agent 侧变更

### 1. 结构化 Action 提取

`agent/src/rpc/approval.rs` 中的 `approval_shape()` 函数现在返回结构化的 `ApprovalShape`，包含：

- `action`: 结构化的操作描述
- `sandbox_boundary`: 沙盒边界信息

**示例：**

```rust
// Shell 命令
ApprovalShape {
    kind: "shell_command",
    risk_level: "high",
    action: json!({
        "tool": "bash",
        "category": "shell_command",
        "command": "rm -rf node_modules",
        "scope": {
            "cwd": "/workspace",
            "inside_workspace": true,
            "estimated_blast_radius": "high"
        }
    }),
    sandbox_boundary: json!({
        "mode": "workspace-write",
        "inside_sandbox": false,
        "violation": "shell_command_not_in_allowlist",
        "cwd": "/workspace",
        "writable_roots": ["/workspace"]
    })
}
```

### 2. Policy Evaluator 桩

新增 `agent/src/rpc/approval_policy.rs` 模块，定义策略评估接口：

```rust
pub enum PolicyDecision {
    AskUser,        // 询问用户
    AutoApprove,    // 自动批准（预留）
    AutoReject(String),  // 自动拒绝（预留）
}

pub fn evaluate_policy(...) -> PolicyDecision {
    // 当前桩：始终返回 AskUser
    // 未来：加载规则、匹配策略、返回自动决策
    PolicyDecision::AskUser
}
```

## GUI 侧变更

### 1. 类型定义

`gui/src/integrations/storage/types.ts` 新增：

```typescript
interface StoredApprovalRequest {
  // ... 原有字段
  actionCategory?: string | null;
  actionPayload?: string | null;      // JSON 字符串
  sandboxBoundary?: string | null;    // JSON 字符串
  reviewer: string;
  decisionScope: string;
  decisionSource: string;
}

interface ApprovalAction { ... }
interface SandboxBoundary { ... }
```

### 2. UI 结构化展示

`gui/src/features/agent/ApprovalPrompt.tsx` 重构为：

- 优先使用 `action` 字段渲染结构化卡片
- Shell 命令：显示命令代码块
- 文件写入：显示路径列表和内容预览
- 文件删除：显示路径列表
- 沙盒违反：显示警告徽章和沙盒模式
- 无 `action` 时 fallback 到旧 `requested_action` JSON 显示

### 3. CRUD 桩函数

`gui/src-tauri/src/store/approval_config.rs` 新增（未暴露 Tauri commands）：

- `get_sandbox_config()` / `upsert_sandbox_config()`
- `get_approval_policy_config()` / `upsert_approval_policy_config()`
- `list_approval_rules()` / `insert_approval_rule()` / `delete_approval_rule()`

## 未来扩展点

### 沙盒执行（P2-FUTURE）

**当前状态：** 数据模型已就位，执行逻辑未实现

**未来工作：**

1. Agent 启动时读取 `sandbox_config` 表
2. 实现沙盒边界检查（文件系统、网络）
3. 沙盒内操作自动通过，不触发审批
4. 沙盒外操作触发审批流程

**涉及模块：**

- `agent/src/rpc/sandbox.rs`（新模块）
- `gui/src/features/settings/SandboxSettings.tsx`（新组件）

### 自动审批策略（P2-FUTURE）

**当前状态：** 数据模型已就位，策略引擎未实现

**未来工作：**

1. 实现 `evaluate_policy()` 规则匹配逻辑
2. 支持 `approval_policy_config` 配置
3. 支持 `approval_rules` 规则匹配
4. 实现 `auto_review` reviewer（审查 agent）

**涉及模块：**

- `agent/src/rpc/approval_policy.rs`（扩展）
- `gui/src/features/settings/ApprovalPolicySettings.tsx`（新组件）

### 决策范围扩展（P2-FUTURE）

**当前状态：** 仅支持 `once`

**未来工作：**

1. UI 添加 "Allow for this session" 按钮
2. UI 添加 "Always allow this command" 按钮
3. 实现 session 级别规则缓存
4. 实现全局规则持久化

**涉及模块：**

- `gui/src/features/agent/ApprovalPrompt.tsx`（UI 扩展）
- `agent/src/rpc/approval_policy.rs`（规则缓存）

### Settings UI（P2-FUTURE）

**当前状态：** 无 UI 入口

**未来工作：**

1. Settings 页面添加 "Sandbox" 配置面板
2. Settings 页面添加 "Approval Policy" 配置面板
3. Settings 页面添加 "Approval Rules" 管理面板
4. 暴露 Tauri commands 连接 CRUD 桩函数

**涉及模块：**

- `gui/src/features/settings/SandboxSettings.tsx`
- `gui/src/features/settings/ApprovalPolicySettings.tsx`
- `gui/src/features/settings/ApprovalRulesSettings.tsx`
- `gui/src-tauri/src/commands/approval_config.rs`（新模块）

## 测试覆盖

### Agent 侧测试

`agent/src/rpc/approval.rs` 新增测试：

- `shell_command_action_is_structured` - 验证 shell 命令 action 结构化
- `shell_command_sandbox_boundary_is_structured` - 验证沙盒边界结构化
- `file_write_action_is_structured` - 验证文件写入 action 结构化
- `file_write_sandbox_boundary_is_structured` - 验证沙盒边界结构化
- `write_preview_truncates_long_content` - 验证长内容截断
- `policy_evaluator_returns_ask_user_by_default` - 验证策略评估桩

所有测试通过 ✅

## 向后兼容性

- 旧 GUI 版本忽略新字段，不影响现有流程
- `requested_action` 字段保留，作为 fallback
- Migration 使用 `ALTER TABLE ADD COLUMN`，对现有数据无破坏性
- 预留表使用 `CREATE TABLE IF NOT EXISTS`，幂等安全

## 总结

P2 完成了审批模型的数据结构升级，为未来的沙盒执行、自动审批和规则引擎预留了完整的扩展点。当前所有功能保持向后兼容，UI 提供结构化展示，Agent 侧保留策略评估桩点。
