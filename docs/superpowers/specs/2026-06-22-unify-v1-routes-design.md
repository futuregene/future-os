# Design — 统一 `/v1/*` API 路由（合并 `/openai` 和 `/anthropic`）

**Date:** 2026-06-22
**Status:** Proposed
**Scope:** `future-server/api/`（主要）+ `future-os/agent/src/models/mod.rs` + smoke 脚本和文档

## 背景

`future-server/api` 当前把模型代理分成两个命名空间：
- `/openai/...` —— 走 `routes/openai.rs`，处理 OpenAI Chat Completions、Responses、Images、Models。
- `/anthropic/...`（以及为 Anthropic SDK 暴露的 `/v1/...`、`/api/anthropic/v1/...`）—— 走 `routes/anthropic.rs`，做 Anthropic↔OpenAI 协议转换。
- `routes/models.rs` 同时被 `/models`（legacy 直挂）和 `/openai/v1/models` 复用，返回 **OpenRouter 风格完整结构**。
- `anthropic.rs::list_models` 是 Anthropic SDK 用的精简结构（仅 `id/display_name/created_at`）。

这种双命名空间导致：
1. 同样的"列出模型"语义出现在两个路径上，且一个 rich、一个 minimal。
2. 调用方必须先选择协议再选路径；外部 SDK 集成时容易混用。
3. 部署侧配置和文档要在两个分支上维护。

本改动把对外路径统一到 `/v1/*`，**硬迁移**：删除 `/openai`、`/anthropic`、`/api/anthropic/v1`，不再保留别名。`/v1/models` 统一返回完整 OpenRouter 结构。

## 目标

- 对外只暴露一套模型代理路径：`/v1/chat/completions`、`/v1/responses`、`/v1/images/*`、`/v1/messages`、`/v1/models`。
- `/v1/models` 返回 OpenRouter 风格完整结构（`id/name/description/context_length/architecture/pricing/supported_parameters/knowledge_cutoff/provider`）。
- 保留 `routes/openai.rs` 和 `routes/anthropic.rs` 文件结构（用户确认）。文件内部的 `routes()` helper 如果不再被 nest，可以删除或标记 `#[allow(dead_code)]`。
- 调用方（agent、smoke 脚本、文档）同步更新到新路径。

## 非目标

- 不动 `routes/models.rs` 的 OpenRouter 结构本身 —— 它已经是 rich 版本。
- 不动 `routes/health.rs`、`routes/oauth.rs`、`routes/mcp/*`、`routes/auth.rs`、`routes/images.rs`（除非它的 endpoint 字符串需要改）。
- 不引入新的认证方案、计费逻辑或上游协议支持。
- 不改 agent 端的缓存逻辑、Registry 行为或 LLM provider 适配层。

## 新路由表

| Method | Path | Handler | 说明 |
|---|---|---|---|
| GET | `/healthz` | `health::healthz` | 不变 |
| GET | `/models` | `models::models` | legacy 顶层，保留 |
| GET | `/v1/models` | `models::models` | 完整 OpenRouter 结构 |
| POST | `/v1/chat/completions` | `openai::openai_chat_completions` | OpenAI 协议 |
| POST | `/v1/responses` | `openai::openai_responses` | OpenAI Responses API |
| POST | `/v1/images/generations` | `images::create_image` | OpenAI 图像生成 |
| POST | `/v1/images/edits` | `images::create_image_edit` | OpenAI 图像编辑 |
| POST | `/v1/messages` | `anthropic::messages` | Anthropic Messages API |
| POST | `/oauth/device/code` | `oauth::device_code` | 不变 |
| POST | `/oauth/device/token` | `oauth::device_token` | 不变 |
| * | `/mcp` | `mcp::service` | 不变 |

**删除**：
- `/openai/v1/models`、`/openai/models`、`/openai/v1/chat/completions`、`/openai/chat/completions`、`/openai/v1/responses`、`/openai/responses`、`/openai/v1/images/generations`、`/openai/images/generations`、`/openai/v1/images/edits`、`/openai/images/edits`
- `/anthropic/messages`、`/anthropic/models`、`/anthropic/v1/messages`、`/anthropic/v1/models`
- `/v1/messages`、`/v1/models`（Anthropic 风格精简结构版本）
- `/api/anthropic/v1/messages`、`/api/anthropic/v1/models`

## 改动清单

### 1. `future-server/api/service/src/routes.rs`（核心）

把当前的：

```rust
Router::new()
    .route("/healthz", get(health::healthz))
    .route("/models", get(models::models))
    .nest("/oauth/device", oauth::routes())
    .nest("/openai", openai::routes())
    .nest("/anthropic", anthropic::routes())
    .nest("/v1", anthropic::routes())
    .nest("/api/anthropic/v1", anthropic::routes())
    .nest_service("/mcp", mcp::service(state.clone()))
    .with_state(state)
```

改为：

```rust
Router::new()
    .route("/healthz", get(health::healthz))
    .route("/models", get(models::models))
    .route("/v1/models", get(models::models))
    .route("/v1/chat/completions", post(openai::openai_chat_completions))
    .route("/v1/responses", post(openai::openai_responses))
    .route("/v1/images/generations", post(images::create_image))
    .route("/v1/images/edits", post(images::create_image_edit))
    .route("/v1/messages", post(anthropic::messages))
    .nest("/oauth/device", oauth::routes())
    .nest_service("/mcp", mcp::service(state.clone()))
    .with_state(state)
```

注意：保留 `mod openai;`、`mod anthropic;` 声明，handler 仍从这里引用。

### 2. `routes/openai.rs`

- 内部 endpoint 字符串：
  - L169 `/openai/v1/chat/completions` → `/v1/chat/completions`
  - L196 `/openai/v1/chat/completions` → `/v1/chat/completions`
  - L338 `/openai/v1/responses` → `/v1/responses`
  - L365 `/openai/v1/responses` → `/v1/responses`
- `routes()` helper（L28–40）：现在不再被 nest。**直接删除**整个 helper。OpenAI handler 通过 `routes.rs` 直接挂载到 `/v1/...`，不依赖这个 helper。

### 3. `routes/openai/streaming.rs`

- L39 `endpoint: "/openai/v1/chat/completions"` → `endpoint: "/v1/chat/completions"`。

### 4. `routes/anthropic.rs`

- `list_models` handler（L754–786）：不再被 nest，**删除**（handler 内部 `AnthropicModelInfo` struct 一并删除）。
- `routes()` helper（L586–592）：现在不再被 nest。**直接删除**整个 helper。Anthropic handler 通过 `routes.rs` 直接挂载到 `/v1/messages`。
- 内部 endpoint 字符串 L702：`/anthropic/v1/messages` → `/v1/messages`。
- 顶部注释块（L1–13）更新路径说明。

### 5. `routes/anthropic/sse.rs`

- L297 `"/anthropic/v1/messages"` → `"/v1/messages"`。

### 6. `scripts/smoke-api.sh`

- L127–128：`GET /openai/v1/models` → `GET /v1/models`，变量名同步。
- L152–153：`POST /openai/v1/chat/completions` → `POST /v1/chat/completions`。
- L163–164：`POST /openai/v1/chat/completions`（stream）→ `POST /v1/chat/completions`。
- 顶部注释或文件中其他 `/openai` 字样（如有）一并更新。

### 7. `future-server/api/README.md`

- L31：`GET /openai/v1/models` → `GET /v1/models`。
- L32：`POST /openai/v1/chat/completions` → `POST /v1/chat/completions`。
- L33：删除 `POST /openai/chat/completions`（无 v1 别名）一行，或改写为说明"无 v1 别名"。
- 顶部 L7 `/openai` 描述更新为 `/v1`。

### 8. `future-os/agent/src/models/mod.rs`

- L138 注释：`/openai/v1/models endpoint` → `/v1/models endpoint`。
- L183 `format!("{}/openai/v1/models", ...)` → `format!("{}/v1/models", ...)`。

### 9. `future-os/cli/README.md`

- L24 step 5 `Fetches GET /openai/v1/models ...` —— commit `71a8ef9` 之后 CLI 不再 fetch models。整段重写为："Saves the returned API Key to `~/.future/agent/auth.json` only. Models are loaded dynamically by the agent on startup."。

## 数据流

### GET `/v1/models`

1. `routes.rs` 把请求路由到 `models::models`。
2. 鉴权 `authenticate_with_apikey_or_bearer`，失败 401。
3. 从 `state.models` 取所有启用模型，映射成 `Vec<OpenRouterModel>`（id, name, description, context_length, architecture, pricing, supported_parameters, knowledge_cutoff, provider），按 id 排序，返回 JSON 数组。

### POST `/v1/chat/completions`（OpenAI）

1. 鉴权 + 计费前置检查（余额）。
2. 读取 `payload.model`，从 `state.models` 查配置。
3. 重写 `payload.model` 为 `model.upstream_model`，加 `stream_options.include_usage = true`，处理 Azure `max_completion_tokens` 重命名。
4. `proxy_chat_upstream` 把请求发到上游 `model.base_url/chat/completions`，传 `model.api_key`。
5. 流式走 `proxy_streaming_chat_completion`，记录 `endpoint = "/v1/chat/completions"`。
6. 非流式记录 `usage_events.endpoint = "/v1/chat/completions"`，返回上游 body。

### POST `/v1/messages`（Anthropic）

1. 鉴权 + 计费前置检查。
2. 读取 `req.model`，从 `state.models` 查配置。
3. `to_openai_payload(&req, &model)` 把 Anthropic request 转 OpenAI 协议（system / 文本 / 图像 / 工具 / thinking）。
4. `proxy_chat_upstream` 发上游。
5. 流式走 `AnthropicStreamProxy::stream`：把 OpenAI SSE 转 Anthropic SSE（`message_start` / `content_block_start` / `content_block_delta` / `content_block_stop` / `message_delta` / `message_stop`）。
6. 非流式 `from_openai_response` 转 `MessagesResponse`，记录 `endpoint = "/v1/messages"`。

## 错误处理

- 鉴权失败 → 401（`auth_required`）。
- 余额不足 → 402（`insufficient_credit`）。
- 模型未找到 → 404（`model_not_found`）。
- provider 未配置 → 502/503（`provider_not_configured`）。
- 上游失败 → 透传 status 和 body（`transform_upstream_error`）。
- 路径不存在 → 404（axum 默认行为，符合硬迁移预期：旧路径不再可达）。

本次改动不引入新的错误码或错误响应格式。

## 测试与验证

### 单元/集成测试

- `cd future-server && make test` —— 跑全 workspace 单元测试，确认无回归。
- `cd future-server && make lint` —— 确认 clippy 不报 dead code 警告（`routes()` helper 已删除）。

### Smoke 测试

- 更新后的 `scripts/smoke-api.sh` 跑过：`make smoke-api`，期望所有 step 200/2xx。

### 手工 curl 验证

启动本地 server (`docker compose up` 或 `make run-api`)，确认：

```bash
# 新路径
curl -s http://localhost:7003/v1/models -H "Authorization: Bearer <test-key>" | jq '.[0]'
# 期望：单个 OpenRouterModel 对象（含 context_length, architecture, pricing）

curl -s -X POST http://localhost:7003/v1/chat/completions \
  -H "Authorization: Bearer <test-key>" \
  -H "Content-Type: application/json" \
  -d '{"model":"deepseek-v4-flash","messages":[{"role":"user","content":"hi"}]}' | jq .
# 期望：OpenAI 风格 chat completion 响应

curl -s -X POST http://localhost:7003/v1/messages \
  -H "Authorization: Bearer <test-key>" \
  -H "Content-Type: application/json" \
  -d '{"model":"deepseek-v4-flash","max_tokens":64,"messages":[{"role":"user","content":"hi"}]}' | jq .
# 期望：Anthropic 风格 messages 响应（content[0].text）

# 旧路径确认 404
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:7003/openai/v1/models
# 期望：404
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:7003/anthropic/v1/models
# 期望：404
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:7003/v1/messages  # POST only
# 期望：405（路由存在但方法不允许）
```

### Agent 端验证

- `cd future-os && make build-agent` —— 重新编译。
- 重启 agent（停掉旧 PID 41530，启动新二进制）。
- `~/.future/agent/.future-models-cache.json` 应该被创建并填充 server 返回的 Future provider 模型。
- `list_models` RPC 返回的 JSON 应包含 `provider: "future"` 的条目。

## 风险与回退

- **风险**：硬迁移会让所有还在用 `/openai` 或 `/anthropic` 旧路径的外部调用方 404。需要在 README/CHANGELOG 明确标注 break change。
- **回退**：如果上线后某个调用方没准备好，可以临时在 `routes.rs` 加回 `.nest("/openai", openai::legacy_routes())`，但本次 spec 不保留此兼容层。
- **数据**：历史 `usage_events.endpoint` 仍是旧路径字符串（`/openai/v1/chat/completions` 等），不影响功能，只影响报表查询。新事件用新路径字符串写入。

## 验证清单

- [ ] `cd future-server && make lint` 通过，无 dead code 警告（除非显式保留）
- [ ] `cd future-server && make test` 通过
- [ ] `cd future-server && make smoke-api` 通过
- [ ] `cd future-os && make build-agent` 通过
- [ ] curl 验证 `/v1/models`、`/v1/chat/completions`、`/v1/messages` 全部 2xx
- [ ] curl 验证 `/openai/v1/models`、`/anthropic/v1/models` 404
- [ ] 重启 agent，`~/.future/agent/.future-models-cache.json` 被填充
- [ ] agent `list_models` 响应包含 `provider: "future"` 模型

## 不在本次范围

- agent 端缓存策略改进、错误日志细化（之前讨论的 stale binary 问题已在 `71a8ef9` 后通过本次重启解决；如有更多问题另开 spec）。
- agent 端 client 解析 OpenRouter 完整结构的字段扩展（description、created_at 等）。
- mcp、oauth、health 路由的任何改动。
- 鉴权、计费、模型配置的 schema 变更。
