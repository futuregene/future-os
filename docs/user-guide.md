# Future OS 使用指南

## 1. 编译

### 环境要求

- Rust（agent 后端）
- Node.js / bun（TUI 前端）

### 编译命令

```bash
# 编译全部（agent + TUI）
make build

# 只编译 agent
make build-agent          # 等价于 cd agent && cargo build

# 只编译 TUI
make build-tui            # 等价于 cd tui && npm run build

# 运行测试
make test                 # 等价于 cd agent && cargo test

# 代码检查
make lint                 # Rust fmt+clippy + TypeScript tsc --noEmit

# 格式化 Rust 代码
make fmt                  # 等价于 cd agent && cargo fmt

# 清理编译产物
make clean                # 删除 target/, dist/, node_modules/
```

编译产物：
- Agent: `agent/target/debug/future-agent`
- TUI: `tui/dist/`

---

## 2. Agent 使用

Agent 只提供 gRPC 服务模式，启动后供 TUI 连接。

```bash
future-agent
# 输出: gRPC server listening on 127.0.0.1:50051
```

### 2.1 命令行参数

```
future-agent [OPTIONS]
```

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `-v, --verbose` | 打印模型名称等详细信息 | false |
| `--grpc-addr` | gRPC 监听地址 | `127.0.0.1:50051` |

### 2.2 模型与 API Key 来源

所有模型相关配置均从配置文件读取：

- **模型**: `~/.future/agent/settings.json` 中 `default_model` → `~/.future/agent/models.json` → 内置默认 `deepseek-v4-flash`
- **API Key**: `LLM_API_KEY` 环境变量 → `ANTHROPIC_API_KEY` 环境变量 → `OPENAI_API_KEY` 环境变量 → `~/.future/agent/auth.json`（按 provider） → 模型内置 key
- **API 地址**: `LLM_BASE_URL` 环境变量 → 模型配置中的 `base_url` → `https://api.openai.com/v1`
- **思考级别**: `~/.future/agent/settings.json` 中 `default_thinking_level`

---

## 3. TUI 使用

### 3.1 启动

```bash
# 先启动 agent（终端1）
future-agent

# 再启动 TUI（终端2）
cd tui && npm run dev
# 或者
make run-tui
```

TUI 通过 gRPC 连接 agent（默认 `127.0.0.1:50051`）。

### 3.2 快捷键

| 快捷键 | 功能 |
|--------|------|
| `ctrl+c` | 中断当前操作 |
| `ctrl+p` | 切换模型 |
| `ctrl+r` | 浏览会话列表 |
| `ctrl+s` | 打开设置 |
| `ctrl+t` | 切换思考级别 |
| `shift+tab` | 切换思考级别 |
| `tab` | 自动补全 |
| `↑/↓` | 滚动 / 导航（输入框中为历史输入） |
| `enter` | 提交 / 确认 |
| `escape` | 关闭弹窗 |

### 3.3 斜杠命令

| 命令 | 功能 |
|------|------|
| `/model [模型名]` | 选择模型 |
| `/sessions` | 浏览会话列表 |
| `/new` | 新建会话 |
| `/clone` | 克隆当前会话 |
| `/fork` | 从历史消息分叉新会话 |
| `/tree` | 会话树视图 |
| `/name [名称]` | 设置会话名称 |
| `/scoped-models` | 配置模型范围（多选） |
| `/compact` | 压缩上下文 |
| `/help` | 显示帮助 |

### 3.4 配置文件

Agent 配置：`~/.future/agent/settings.json`
- 模型默认值、思考级别、自动压缩、重试策略等

TUI 配置：`~/.future/tui/settings.json`
- 主题、终端图片显示、快捷键覆盖等

API 密钥：`~/.future/agent/auth.json`
- 按 provider 存储 API key

---

## 4. 项目结构

```
future-os/
├── agent/          # Rust 后端（gRPC 服务）
│   └── src/
├── tui/            # TypeScript 终端 UI
│   └── src/
├── proto/          # gRPC 协议定义
│   └── proto/future.proto
├── docs/           # 设计文档
└── Makefile        # 构建入口
```
