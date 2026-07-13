# 附件重构 —— "只给路径，让模型自己读" 实现计划

> 状态：重设计（替换旧的「GUI 端 pdfjs/文本抽取内联」方案）。核心思想：**不再由 GUI 预处理文件内容，只把文件路径结构化地交给 agent，让大模型用自己的 `read`/`bash`/`grep`/`ls` 工具按需读取**。图片仍走 base64 image 通道（模型支持时），否则降级为路径。
>
> **已锁定决策**：
> - ① **取消内联抽取**：不再用 pdfjs 抽 PDF、不再把文本内容塞进提示词。
> - ② **只给路径**：提示词/消息里只放文件的绝对路径，让模型自行决定怎么处理。
> - ③ **图片特殊处理**：模型支持图片模态 → 按图片（base64 `image_url`）发；否则降级为「填充文件路径」。
> - ④ **PDF / 二进制文件 → 完全交给模型跑 bash**（`read` 工具不改，仍只读 UTF-8 文本）。模型对 PDF/docx 等用 `pdftotext`/`python` 等 shell 工具处理。**注入文本块只列路径、不解释怎么读**——工具已在系统提示词别处描述，且不同平台方法不同，交给模型自行判断。
> - ⑤ **结构化存储 = agent JSONL `messageMeta`（方案 B）**：扩 `proto RpcCommand.attachments` + `SessionEntry.meta`，agent 统一组装（图片降级逻辑收敛到 agent 一处）。
> - ⑥ **不再拷贝到工作目录**：`messageMeta` 直接存原始绝对路径（都在本机，模型通过工具访问）。图片缩略图仍落盘供气泡渲染。
> - ⑦ **除图片外不限制**：非图片文件不限类型、大小、数量。图片保留大小 + 数量限制（vision 有 token/尺寸上限）。
> - ⑧ **本期范围 = 仅 GUI**（+ agent/proto）。TUI 不接线新字段（proto 加字段向后兼容，不传即可）；channels（飞书/钉钉）留后续。

---

## 1. 为什么这样做（相对旧方案）

旧方案（GUI 端抽取内联）为了把文件内容塞进提示词，背了大量复杂度：二进制嗅探、`TEXT_EXTENSIONS` 白名单、单文件 30KB/2000 行 + 全局 4 文件/60KB 三重截断、`inlineContext` 持久化、临时文件清理。

新方案把「读文件」还给 agent —— agent **本就有** `read`/`bash`/`grep`/`ls` 工具，模型能翻页读、grep 定位、按需读，比 GUI 一刀切 30KB 更强。

**地基事实（已核对源码）**：`agent/src/sandbox/rules.rs:484` 里读操作默认 `Decision::Allow` —— 模型 `read` 工作目录外的本机文件**不会每次弹审批**，只有敏感文件（`.env`/`.ssh`/`*.key`）才 `Ask`。所以「只给路径让模型自己读」不会变成审批地狱。这条是方案成立的前提。

---

## 2. 数据流

```
用户加附件（文件选择/拖拽/粘贴）
  └─ GUI 分类：image | file
      └─ 发送时（sendPipeline → persistImageAttachments）：
         ├─ image：生成缩略图落盘 images/<tid>/thumb（气泡渲染用）
         │         · 粘贴/下载图（临时目录）：额外拷到 images/<tid>/origin + 删临时 + rewrite path
         │         · 本机图（真实路径）：仅 thumb，path 不变
         │         → 附件 {path, kind:"image", name} 走 proto.attachments
         └─ file ：附件 {path, kind:"file", name} 走 proto.attachments（不拷贝、不落盘、不截断）
   ↓ proto RpcCommand.attachments
  agent prompt() 组装 user message content：
   ├─ image 且模型支持图片 → image_url（base64）
   ├─ image 但模型不支持    → 降级：并入下方「附件路径」文本块（带 (image) 标记）
   └─ file                  → 并入「附件路径」文本块
   ↓ 文本块格式（markdown 链接，尖括号包路径以兼容空格/特殊字符）：
      The user attached the following local files:
      - [report.pdf](</abs/report.pdf>)
      - [diagram.png](</abs/diagram.png>) (image)
   ↓
  同时把 attachments 写进该 user 条目的 SessionEntry.meta（结构化，供 UI/回放）
```

**要点**：模型必须「看得见」路径才能去读，所以路径既进 `meta`（结构化存储），也由 agent 渲染进**模型可见的 message content**（文本块，markdown 链接形式）。`meta` 是输入 + 存档，content 文本块是模型实际读到的东西。文本块只列路径、不解释读法（工具已在系统提示词别处描述、平台相关）。

---

## 3. 关键取舍与风险（已知、已接受）

| 项 | 取舍 | 处理 |
|---|---|---|
| **原始路径失效** | 不拷贝 → 源文件被移动/删除后，历史 chip 失效、retry 读不到 | 单机工具可接受；`<img> onError` 兜底成文件名 chip；图片缩略图落盘不依赖源文件 |
| **PDF/二进制** | `read` 只读 UTF-8，PDF 报错 | 交给模型跑 bash（`pdftotext`/`python`）；注入文本块只列路径、不解释读法；需实机验 seatbelt tier 下 bash 能跑通 |
| **Artifact 面板** | chat 附件不再登记 Artifact | 不再进 Artifacts 面板；预览改走 `filepreview` 直接读原始路径 |
| **图片降级判定** | `supportsImages` 标记不可靠（Future provider Qwen-VL 曾被误标 text-only，见 `sendPipeline.ts` 注释） | 沿用现状「总是发图 + API 报错兜底」，或用更可靠能力信号；避免误降级能看图的模型 |
| **channels** | 「都在本机」前提对飞书/钉钉不成立（附件从对方服务器下载） | 本期不做；后续各 channel 把下载的临时路径传入 |

---

## 4. 改动清单（按层）

### 4.1 proto（`proto/future.proto`）
- `RpcCommand` 新增 `repeated Attachment attachments = <n>;`
- 新增 `message Attachment { string path = 1; string kind = 2; string name = 3; }`（`kind` = `"image"` | `"file"`）
- `make generate-proto`

### 4.2 agent（Rust）
- `agent/src/rpc/session_prompt.rs` `prompt()`：
  - 签名接收 `attachments`；组装 content 时按上面数据流分流（image_url / 注入路径文本块）。
  - 把 attachments 写进 user `SessionEntry.meta`。
- `agent/src/session/mod.rs`：`SessionEntry` 加 `meta: Option<serde_json::Value>`；`agent_message_to_entry` 与反序列化往返；向后兼容（老 session 无此字段）。
- `agent/src/rpc/mod.rs`：`prompt` 命令处理透传 `attachments`。
- `agent/src/prompt/mod.rs`（可选）：附件 bash 提示的措辞（或直接在文本块里写死）。
- **`read` 工具不改**。

### 4.3 GUI 前端
- `features/agent/attachments.ts`：删 `TEXT_EXTENSIONS` 白名单 / 二进制嗅探依赖；分类简化为 `image` | `file`；非图片不限类型/大小/数量；图片保留限制。
- `features/agent/attachmentContext.ts`：**删除** `buildInlineAttachmentContext`（pdfjs 抽取）。
- `features/agent/sendPipeline.ts`：删内联抽取 / `inlineContext` / 非图片落盘；附件透传 `{path, kind, name}`；图片仍生成落盘缩略图。
- `features/agent/agentThreadTypes.ts`：`MessageAttachment` 精简为 `{path, kind, name, thumbnail?}`；去掉 `inlineContext`。
- `features/agent/threadAttachments.ts`：删 `importChatAttachments`（非图片）/ `importWorkspaceImages`；只保留图片缩略图生成。
- `integrations/agent/agentClient.ts`：接线 proto `attachments` 字段。
- 消息气泡（`MessageBlock.tsx`）：chip 从 `path` + 缩略图渲染；`onError` 兜底。
- 预览：`filepreview` 读原始路径（image + 文本；PDF/其它走 OS 默认处理器，现状已如此）。

### 4.4 Tauri（src-tauri）
- 保留 `write_thumbnail`（图片缩略图落盘）、`read_file_base64`（图片 base64）。
- `inspect_attachment`：可简化（不再需要二进制嗅探；仅目录判定可留可删）。
- `delete_temp_attachment`：仅保留清理粘贴图片临时文件。

---

## 5. 分阶段

- **P0**：proto `Attachment` + `SessionEntry.meta` + agent `prompt()` 组装/降级/写 meta。（跑通 agent 侧，TUI 不传即向后兼容）
- **P1**：GUI 发送路径瘦身 —— 删内联抽取/落盘/截断/白名单；附件透传新 proto 字段；图片保留缩略图 + 限制。
- **P2**：存储与 UI —— chip 从 path+缩略图重建、`onError` 兜底、预览走 filepreview；确认 Artifacts 面板不再收 chat 附件。
- **P3**：清理 —— `delete_temp_attachment` 收窄、图片降级判定定稿。

## 6. 验证
```bash
cd gui && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run
cd gui/src-tauri && cargo fmt --check && cargo clippy   # 若动 Tauri
cd agent && cargo test && cargo clippy
make run-gui   # 实机跑通 图片 / PDF(走 bash) / 文本 三类附件端到端
```

## 6.5 落地状态

- ✅ **P0（agent + proto，已验证）**：`proto` 加 `Attachment` + `RpcCommand.attachments`；`Attachment` Rust 类型；`session_prompt.rs` 抽出纯函数 `build_user_message`（图片支持→`image_url`／否则路径文本块 + bash 提示；写 `AgentMessage.metadata`）；`SessionEntry.meta` 往返（`agent_message_to_entry` 写、`entries_to_agent_messages` 读，reload 不丢）。`cargo build/clippy/test` 全过（106 tests，含 4 个 `build_user_message` 单测）。
- ✅ **P1（GUI，已静态验证）**：`attachments.ts` 分类简化为 `image|file`、非图片不限类型/大小/数量、仅图片限数量（`MAX_IMAGES_PER_TURN`）；删 `attachmentContext.ts`（pdfjs 抽取）、`inlineContext`、`importChatAttachments`/`importWorkspaceImages`（不再拷贝）；`sendPipeline` 只保留图片缩略图，附件按 `{path,kind,name}` 透传；`agentClient` 改传 `attachments`；Tauri `agent_prompt`/`prompt_command`/`encode_attachments` 按 attachments 读 base64（仅图片）构造 proto；Composer 接受任意文件、picker 无扩展过滤；`MessageBlock` chip 按 `image|file` 渲染。`tsc`/`eslint`/`vitest`(153)/`cargo check`/`clippy`/`fmt` 全过。
- **图片持久化（按来源区分）**：`persistImageAttachments`——粘贴/下载图（临时目录、无用户原始路径）拷进 `images/<tid>/origin` + 删临时 + rewrite path（重发/大图预览可用）；本机图（真实路径）仅落 thumb、path 不变；两者都不写工作目录。非图片文件完全不落盘。
- **注入文本简化 + markdown 链接**：附件文本块**只列路径**，不再解释 read/bash/pdftotext（工具已在系统提示词别处描述，且平台相关）。每行用 markdown 链接、尖括号包路径 `- [name](</abs/path>)`（兼容空格/特殊字符），降级图片带 ` (image)` 标记。
- ⏳ **待办**：`make run-gui` 实机端到端验证（图片 / PDF 走 bash / 文本 三类）；未验证前视为"静态通过、运行未验"。旧 `attachDialogFilter*` / `attachLimitReached` i18n key 已不再引用（无害，可后续清理）。

## 7. 关键源码索引
- proto：`proto/future.proto`（`RpcCommand` / 新增 `Attachment`）
- agent 组装：`agent/src/rpc/session_prompt.rs` `prompt()`（图片 image_url / 注入路径 / 写 meta）
- session 条目：`agent/src/session/mod.rs`（`SessionEntry.meta`、`agent_message_to_entry`）
- 读默认放行：`agent/src/sandbox/rules.rs:484`（`Op::Read => Decision::Allow`）
- read 工具（不改，只读文本）：`agent/src/tools/mod.rs:618 run_read`
- GUI 发送：`gui/src/features/agent/sendPipeline.ts`、`attachments.ts`、`attachmentContext.ts`、`threadAttachments.ts`
- 消息类型：`gui/src/features/agent/agentThreadTypes.ts`
- 缩略图/字节 Tauri 命令：`gui/src-tauri/src/commands/files.rs`
