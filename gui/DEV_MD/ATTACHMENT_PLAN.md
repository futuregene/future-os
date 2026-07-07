# 非图片附件支持（PDF / 文本 / 源码）— 实现计划

> 状态：计划稿（待评审，先看方案再开工）。核心实现**在 GUI 端**（TypeScript/React + 少量 Tauri 命令复用）；**agent / proto 不改**。
>
> **已锁定决策**：①图片缩略图 = **持久化到 `~/.future/app/images/<threadId>/thumb/<stamp>.jpg`**（后端 `write_thumbnail(threadId, base64Jpeg)` 落盘并分配唯一文件名，`MessageAttachment.thumbnail` 存该路径，`convertFileSrc` 渲染；不再塞 base64 进 JSONL）。**原方案 A′（写 `appCacheDir/thumbnails`）已废弃**：`appCacheDir` 在 macOS = `~/Library/Caches/<id>`，属 APFS 可回收空间，系统会静默清理，导致消息里的缩略图路径失效、图片渲染成空白框。②PDF = **pdfjs 抽文本内联**（非原生 file part）；③内联上限 = **单文件 30KB/2000 行；可内联（文本+PDF）附件最多 4 个、合计 ≤60KB**（图片走独立通道、不计入 60KB）；④本计划落 `gui/ATTACHMENT_PLAN.md`。
>
> **图片原图落盘**：**chat** 附件照旧存进临时 workspace（`~/.future/app/workspaces/chat/<tid>/attachments`，登记 Artifact）；**workspace** 图片原图经 `import_workspace_image(threadId, sourcePath, name)` 拷进 `~/.future/app/images/<threadId>/origin/<stamp>_<name>`（持久、在 asset scope 内，供重发/查看），不写用户项目目录、不建 Artifact。
>
> **临时文件清理 = 方案 A（已定）**：发送后显式删除，**仅限我们自己的 `futureos-attachments` 临时子目录**，绝不碰用户选择/拖拽的原文件。`TEXT_EXTENSIONS` 白名单先按初版。
>
> **图片目录回收**：`images/<tid>` 没有逐删执行器,统一靠**启动时孤儿清扫** `reconcile_orphan_images`——`threads` 表里 `status='deleted'` 或无行的 tid,其 `images/<tid>` 目录被 `remove_dir_all`(方案 A,无软删撤销)。覆盖 GUI 删、CLI/TUI 外部删 session、整库 reset(`clear_all_data` 额外清 `images/` 整棵)三种来源。渲染侧 `AttachmentChip` 的 `<img>` 加了 `onError` 兜底:文件缺失时退成文件名 chip 而非空白框。
>
> ⚠️ **前置任务（已完成）**：`tauri.conf.json` 的 `app.security.assetProtocol` 已启用，scope 含 `$HOME/.future/**`——`~/.future/app/images/**` 天然在内,`convertFileSrc` 直接可用,无需额外配置。

---

## 1. 目标

- 新增支持附件类型：**PDF、纯文本、源码**（图片维持现状）。
- **不支持**：目录、MCP resource、二进制（含 docx/xlsx 等 Office 二进制）。
- 喂模型方式：**GUI 端抽取文本 → 内联进 user 消息**（PDF 用 pdfjs，文本用现有 `read_text_file_preview`）；图片仍走 base64 → `image_url`。
- **前置检测**：不支持的类型在 GUI 入口当场拒绝（toast），绝不入队、绝不发模型。
- **Chat thread**：附件照旧存 artifact（落盘 + 预览）；**Workspace thread**：不存 artifact、不碰 workspace tree，内容内联即用。
- **预览**：artifact viewer 支持 image / pdf / text 预览（**已基本现成**）。
- **Thread 内展示**：图片缩略图；PDF / 文本只显示图标 + 文件名 chip。

---

## 2. 现状（含「已具备」部分，避免重复造轮子）

| 能力 | 现状 | 位置 |
|---|---|---|
| 协议附件 | **仅图片**（`RpcCommand.images: ImageContent`） | `proto/future.proto:21-36`、`agent/src/types/mod.rs` |
| agent 组装 | 只取 images → `image_url`，非图片静默丢 | `agent/src/rpc/session_prompt.rs:62-80` |
| GUI 入口 | 选择/拖拽/粘贴**全部按 `IMAGE_EXTENSIONS` 过滤** | `features/agent/Composer.tsx`、`attachments.ts:5` |
| 图片能力门控 | 发前查 `modelSupportsImages`，否则不发图 | `useAgentThreadState.ts:246`、`agentClient.ts:65` |
| chat→artifact | `import_attachment_artifact` 仅 `mode==="chat"` 存；**workspace 已跳过** | `useAgentThreadState.ts importChatAttachments`、`store/artifacts.rs:68-109` |
| 文本读取命令 | **已存在** `read_text_file_preview(path, max_bytes?)`→`{content,size,truncated}` | `src-tauri/src/commands/files.rs:32` |
| PDF 渲染 | `PdfPreview`（pdfjs，仅渲染页） | `features/artifacts/PdfPreview.tsx` |
| **artifact 预览** | **image + pdf + text 三种已内置**（`imageSrc`/`PdfPreview`/`readTextFilePreview`） | `features/artifacts/ArtifactDetailPanel.tsx:31-60,174-214` |
| artifact 类型分类 | 已分 image/pdf/document/spreadsheet/data/code/file | `store/artifacts.rs:159-178 artifact_type_from_path` |
| 消息附件类型 | `MessageAttachment { artifactId?, name, path }`（**无缩略图字段**） | `features/agent/agentThreadTypes.ts:34` |

**结论**：需求④（预览）几乎现成；要做的主要是「入口检测/分类 → 抽取内联 → 消息渲染缩略图/图标 → workspace 不落盘」。

---

## 3. 核心数据流

发送前在 `useAgentThreadState` 的发送路径里按 thread 模式分流：

```
用户加附件
  └─ 入口分类(§4)：image | pdf | text | ❌unsupported(toast 拒绝)
      └─ 发送时：
         ├─ 图片：
         │   ├─ 生成小缩略图(canvas, ~96px) → write_thumbnail(threadId) 落盘 images/<tid>/thumb → MessageAttachment.thumbnail
         │   ├─ chat: import_attachment_artifact（原图落临时 workspace/attachments，登记 Artifact）
         │   ├─ workspace: import_workspace_image（原图拷进 images/<tid>/origin，rewrite path）
         │   └─ base64 全图 → RpcCommand.images（受 modelSupportsImages 门控）
         ├─ PDF：pdfjs 抽文本 →(30KB/2000行截断)→ 内联进消息
         │   └─ chat: import_attachment_artifact（落盘+PdfPreview）
         └─ 文本/源码：read_text_file_preview(30KB) →(2000行截断)→ 内联进消息
             └─ chat: import_attachment_artifact（落盘+文本预览）
   workspace thread：以上 import_* 全部跳过；不写 workspace tree；
                     粘贴临时文件发送后清理；缩略图写 app 缓存目录(决策A′)
```

**⚠️ 内联只进「模型侧」`promptContent`，不进可见气泡**（代码已验证有此 seam，见 §10）：
- 可见气泡 = `messageContent`（`stringifyMessageContent` 存 `{text, attachments}`，只展示 chip）。
- 模型侧 = `promptContent = buildReferencePrompt(..., buildPromptWithAttachments(content, attachments))`。
- 抽取的 30–60KB 文本拼进 **`buildPromptWithAttachments` 的输出（promptContent）**，气泡仍只显示文件名 chip。

模型侧 `promptContent` 内联结构（示意）：
```
<用户输入>

附带文件内容（已为你读取，超出部分已截断）：

===== report.pdf (PDF, 12 页, 已截断到 30KB) =====
<抽取文本…>

===== src/main.rs =====
<文件内容…>
```

---

## 4. 前置检测 / 分类（需求③）

> **核心原则（修正自代码核对）**：webview **不能直接读任意本地 path 的字节**，且 asset scope 只覆盖 artifact 目录 + appCache（**不含 workspace 用户原文件**）。因此：
> - **所有「读字节」**（分类嗅探 / 文本 / PDF 字节 / 缩略图源图）一律走 **Tauri 命令按绝对路径读**（Rust 有完整 fs 权限，无 scope 限制）。
> - **asset:// / `convertFileSrc` 仅用于 webview 渲染**（artifact 预览、appCache 里的缩略图——这两处在 scope 内）；**绝不**用它去读用户原文件做分类/抽取/缩略图。

分类在 **Rust 侧**完成（前端只拿到 path）：新增 `inspect_attachment(path) -> { isDir, size, kind }`，Rust 读文件头判定（目录、magic byte、NUL/控制字符嗅探）：

| 类别 | 判定 | 动作 |
|---|---|---|
| 图片 png/jpg/jpeg/gif/webp/bmp/svg | 扩展名 + magic byte | 入队，标 `kind: "image"` |
| PDF | 扩展名 `.pdf` + `%PDF` magic | 入队，标 `kind: "pdf"` |
| 文本/源码 | 扩展名在 `TEXT_EXTENSIONS` 白名单 **且** 首 1KB 非二进制嗅探（无 NUL、控制字符占比低） | 入队，标 `kind: "text"` |
| 目录 | 路径为 dir | ❌ toast：「不支持文件夹」 |
| 二进制/未知（docx/xlsx/zip…） | 不在白名单 / magic 命中已知二进制 / NUL 嗅探 | ❌ toast：「不支持该文件类型」 |

`TEXT_EXTENSIONS`（初版）：`txt md markdown json jsonl yaml yml toml xml csv tsv ini cfg conf log sql sh bash zsh py rs ts tsx js jsx mjs cjs go java c h cpp hpp cc cs rb php swift kt scala r lua pl dart vue svelte css scss less html htm`，外加无扩展名的 `Dockerfile / Makefile`。（白名单可后续扩。`.env` 等敏感文件默认不纳入。）

**内联上限（决策③）**：
- 单文件：**30KB 或 2000 行**，先到者截断（`read_text_file_preview` 传 `max_bytes: 30*1024`，再在 TS 按 2000 行二次截断）。
- 全局：**可内联（文本+PDF）附件最多 4 个；内联文本合计 ≤60KB**，累计到 60KB 即停止再内联并提示。第 5 个及以上、或会超 60KB 的文件 → 入口 toast 拒绝/提示。
- 图片**不计入** 60KB（走独立 base64 通道，仍受 `modelSupportsImages` 门控）。
- 所有截断/超额都在内联文本里明确标注「已截断 / 已省略」。

---

## 5. 各需求落地 + 改动点

### 5.1 PDF/文本/源码 → 内联（需求①，核心）
- `features/agent/attachments.ts`
  - 新增 `TEXT_EXTENSIONS`；分类调 Rust `inspect_attachment(path)`（前端不读 path 字节）。
  - 新增 `extractPdfText(path)`：**先 `read_file_bytes(path)` 取字节 → `pdfjs.getDocument({ data })`**（不要用 path/asset URL——workspace 原文件不在 asset scope）→ 逐页 `getTextContent()` 拼接（复用 `PdfPreview.tsx` worker 配置）。
  - 新增 `extractTextFile(path)`：调 `read_text_file_preview({path, maxBytes: 30720})`（Rust 按 path 读，无 scope 问题）。
  - 新增 `capInline(text)`：30KB/2000 行截断 + 截断标注。
  - 新增 `buildInlineAttachmentBlock(items)`：拼装内联结构（替代/扩展现有 `buildPromptWithAttachments` 的「只列路径」）。
- `features/agent/Composer.tsx`：file picker filters 增加 pdf + text 扩展；拖拽/粘贴接受这些类型，其余 toast 拒绝（替换现有 `isImagePath` 单一过滤为 `classifyAttachment`）。
- `useAgentThreadState.ts`（发送路径 `:168-248`）：在 `buildReferencePrompt` 之前**先 await 抽取** pdf/text 内容，再喂给（升级为 async 的）`buildPromptWithAttachments` 拼进 **`promptContent`**；**不动 `messageContent`**。图片仍走 `imageAttachmentPaths` + `modelSupportsImages`。
  - 注：`buildPromptWithAttachments` 目前只列路径（`attachments.ts:59`），改为内联抽取文本（同步签名→async，或先抽取再传入已抽好的内容）。
  - 注：`MAX_ATTACHMENTS_PER_TURN = 4` **已存在**（`attachments.ts:3`），复用；新增 60KB 合计预算。
- 粘贴只产生图片临时文件；**pdf/文本经文件选择器/拖拽传入，是用户真实路径，无需落临时文件**（抽取直接就地读）。

### 5.2 Workspace thread 不落盘 + 图片缩略图（需求②，决策 A′）
- `useAgentThreadState.ts` `importChatAttachments`：维持「`mode!=="chat"` 跳过 artifact」；workspace 仅做抽取内联，不 import。
- 图片缩略图（决策 A′：app 缓存目录）：源图同样不能让 canvas 直接读（workspace 原图不在 scope）。流程：**`read_file_bytes(path)` 取原图字节 → 前端 `Blob`/`createImageBitmap` → canvas 下采样 ~96px → `toBlob("image/jpeg",0.6)` → `write_thumbnail(bytes) -> cachePath`**（Rust 写 `appCache/thumbnails/<key>.jpg`，`key`=源路径+thread/msg id 哈希）→ `MessageAttachment.thumbnail` 存 cachePath → 渲染用 `convertFileSrc`（appCache 在 scope）。**chat 与 workspace 都生成**。缓存被清后退化为图标（workspace 无源不可重生，可接受）。
  - 备选：纯 Rust 侧用 `image` crate 直接读原图→缩放→写 appCache（省去字节往返，但加一个 Rust 依赖）。
- 粘贴临时文件清理（待确认，推荐方案 A）：发送后调 `delete_temp_attachment` 命令删除粘贴产生的临时文件——**护栏：仅允许删 `<temp>/futureos-attachments/` 下的文件**，绝不碰文件选择器/拖拽的用户原文件。

### 5.3 前置检测/拒绝（需求③）
- 全部在 §4 的 `classifyAttachment` + Composer 入口完成；不支持类型**不进 attachments 数组**，自然不会发模型。

### 5.4 Artifact 预览（需求④，**多数现成**）
- `ArtifactDetailPanel.tsx` 已支持 image/pdf/text 预览；**只需确认** `artifact_type_from_path` 的 `code/data/document(文本类)` 都能落到「文本预览」分支（而非被当作不可预览）。必要时小调类型→预览映射，使源码/文本统一走 `readTextFilePreview` 文本预览。
- docx/xlsx 等二进制：本就不支持、不入库，无需预览。

### 5.5 Thread 内缩略图 / 图标（需求⑤）
- `MessageAttachment` 扩字段：`kind?: "image"|"pdf"|"text"`、`thumbnail?: string`（**app 缓存目录里的缩略图路径**，非 base64）。
  - 同步：`agentThreadTypes.ts:34`、`store` 持久化结构、proto 无关（GUI 本地）。
- 消息气泡附件渲染（`AgentThread.tsx` 及其消息子组件）：
  - `image` → `<img src={convertFileSrc(thumbnail)}>` 缩略图（缺失则回退 artifact/源 path，再不行显示图标）。
  - `pdf` / `text` → 文件图标 + 文件名 chip（chat 下可点开走 ArtifactDetailPanel 预览）。

### 5.6 内联块的持久化 / retry 复用（修正 P2）
**问题**：抽取的内联文本只拼进 `promptContent`（不进可见气泡），但若不持久化，retry/重发（GUI 侧从已存消息重建）就拿不到这份上下文——尤其 workspace 无 artifact、临时文件已删时无法重建。

**方案**：把抽取后的「内联块」**持久化进已存的 `mixed` 用户消息 JSON 里一个不渲染的字段**（如 `inlineContext?: string`，与 `text`/`attachments` 并列），气泡照旧只显示 `text`+chip。
- 首次发送：`promptContent = buildReferencePrompt(... 把 inlineContext 拼进去)`。
- **retry/重发**：直接复用已存的 `inlineContext` 重建 `promptContent`，不依赖原文件/临时文件是否还在。
- agent 侧 session 本就保留首次发送的完整 `promptContent`，故 agent 自身的续跑也不丢；本方案保证 **GUI 侧重建**也一致。
- 体积：内联块上限 60KB（§4），随消息进 session 存储可接受；若担心，可只在「文件已不可读」时回退到 `inlineContext`。

> 需核对 retry/重发的实际代码路径（它如何重建发给 agent 的内容），确保读取 `inlineContext`。

---

## 6. 改动清单（按文件）

**GUI 前端**
- `features/agent/attachments.ts` — 分类、白名单、二进制嗅探、pdf/文本抽取、截断、内联拼装、缩略图生成。
- `features/agent/Composer.tsx` — 入口接受 pdf/text、拒绝其余。
- `features/agent/useAgentThreadState.ts` — 发送分流（抽取内联 / chat 存 artifact / workspace 不存 / 图片门控）。
- `features/agent/agentThreadTypes.ts` — `MessageAttachment` 加 `kind` / `thumbnail`；mixed 消息 JSON 加不渲染的 `inlineContext`（§5.6）。
- `features/agent/attachments.ts` — 分类/抽取/缩略图相关 helper **均通过 Tauri 命令读字节**（`inspect_attachment`/`read_file_bytes`/`read_text_file_preview`），前端不直读 path。
- `features/agent/AgentThread.tsx`（+ 消息附件渲染子组件）— 缩略图/图标 chip。
- `features/artifacts/ArtifactDetailPanel.tsx` — 确认/微调文本预览覆盖源码类（多数已成）。

**Tauri（src-tauri）— 附件 IO 基础设施（P0）**
- **`tauri.conf.json`**：启用 `app.security.assetProtocol`（`enable: true` + scope 覆盖 chat artifact 目录与 `appCacheDir`）；capabilities 加对应 asset 权限。
- 修 `PdfPreview.tsx:31` 裸 `asset://` → `convertFileSrc`（Windows 兼容）。
- 新增 `inspect_attachment(path) -> { isDir, size, kind }`：Rust 读文件头做目录/magic/NUL 判定（分类）。
- 新增 `read_file_bytes(path, maxBytes?) -> bytes`：供 PDF 抽取（`getDocument({data})`）与缩略图源图字节读取。
- 新增 `write_thumbnail(bytes) -> cachePath`：写 `app_cache_dir()/thumbnails`（用 `app.path().app_cache_dir()`，无需 fs 插件）。
- 新增 `delete_temp_attachment(path)`：仅限 `futureos-attachments` 临时子目录（护栏校验）。
- 复用既有 `read_text_file_preview`（文本，传 30KB）。

**Agent（Rust）/ proto**：**不改**。

---

## 7. 分阶段
- **P0（前置：本地附件 IO 基础设施）**：① 启用 `assetProtocol` + scope（artifact 目录 + `appCacheDir`）+ capabilities；② `PdfPreview` 裸 `asset://` → `convertFileSrc`；③ 新增 Tauri 命令 `inspect_attachment` / `read_file_bytes` / `write_thumbnail` / `delete_temp_attachment`；④ 实机确认现有 image/pdf 预览能渲染。**后续分类/抽取/缩略图都依赖这层。**
- **P1（核心）**：§4 检测/拒绝 + §5.1 PDF/文本/源码抽取内联 + 单文件 30KB/2000 行 + 全局 4 文件/60KB。让模型真正用上非图片内容。
- **P2**：§5.5 thread 缩略图（A′：app 缓存目录）/ 图标 + §5.4 确认 artifact 文本预览覆盖源码。
- **P3**：粘贴临时文件清理（方案 A）、白名单扩充、内联结构打磨。

---

## 8. 风险 / 盲区
- **扫描版 PDF（无文本层）**抽不出文字 → 内联为空。处理：抽取为空时在 UI 提示「该 PDF 无可提取文本」，不发空内容。（如需扫描件，未来才考虑原生 PDF→vision 模型，需改 proto+agent，本期不做。）
- **大文件**：单文件 30KB/2000 行 + 全局 4 文件/60KB，均标注「已截断/已省略」。
- **缩略图缓存被清**：app 缓存目录可被系统/用户清理；chat 可从 artifact 重生成，workspace 无源 → 退化为图标（可接受）。
- **assetProtocol 未启用**：现有 image/pdf 预览很可能现在就渲染不出 → P0 修复。
- **二进制误判**：白名单 + NUL 嗅探双保险；少数含大量控制字符的文本可能被误拒（罕见，可改扩展名规避）。

---

## 9. 决策结论（全部已定）
- ✅ ①缩略图 → **app 缓存目录**（A′）。 ✅ ②PDF → **pdfjs 抽文本**。 ✅ ③上限 → **单文件 30KB/2000 行 + 全局 4 文件/60KB**（图片不计入）。 ✅ ④临时文件清理 → **方案 A**（显式删除，限 `futureos-attachments` 子目录）。 ✅ ⑤白名单先按初版。

---

## 10. 代码核对结论（review，结合源码逐条验证）

| 假设 | 结论 | 证据 / 修正 |
|---|---|---|
| 存在「显示 vs 模型」分离的 seam | ✅ 成立 | `useAgentThreadState.ts:170-177`：`messageContent`(显示) 与 `promptContent`(模型) 是两个变量。**内联只改 `promptContent`** |
| 内联点 = `buildPromptWithAttachments` | ✅ 但需改造 | `attachments.ts:59-69` 现仅列路径；改为内联抽取文本，签名转 async（或先 await 抽取再传入） |
| 最多 4 附件 | ✅ 已有常量 | `MAX_ATTACHMENTS_PER_TURN = 4`（`attachments.ts:3`）复用；新增 60KB 合计 |
| 消息附件可加 `kind`/`thumbnail` 无需迁移 | ✅ | 附件随 `stringifyMessageContent` 以 JSON 存在消息 content（`mixed` 类型），加可选字段向后兼容 |
| 气泡附件渲染可扩展 | ✅ | `MessageBlock.tsx:47-61` 现为 Paperclip+name，可加缩略图/图标 |
| artifact 文本/代码预览已成 | ✅ | `ArtifactDetailPanel.tsx` 的 `isTextPreviewArtifact` 覆盖 `code/data/document/text`；`isImageArtifact`/`isPdfArtifact` 齐全 |
| pdfjs 可复用抽文本 | ✅ | `PdfPreview.tsx:1-8` worker 已配；`getDocument().getTextContent()` 可用；桌面端无 SSR 坑 |
| 文本读取命令可用 | ✅ | `read_text_file_preview(path, max_bytes?)`（`files.rs:32`），传 30KB |
| **assetProtocol 已配置** | ❌ **未配置（P0 必修）** | `tauri.conf.json` 无 `assetProtocol`、capabilities 无 asset scope。`PdfPreview.tsx:31` 用裸 `asset://` 在 Windows 必失败；`convertFileSrc` 也需 assetProtocol+scope 才生效。**现有 image/pdf 预览很可能现在就不工作 → 实机确认 + P0 修复** |
| 缩略图写盘 | ✅ 用 Rust 命令 | GUI 无 fs 插件、JS `appCacheDir()` 不可用；新增 Rust `write_thumbnail`，用 `app.path().app_cache_dir()` 解析缓存目录 |
| 粘贴非图片需落临时文件 | ❌ 不需要 | 粘贴仅处理图片；pdf/文本经选择器/拖拽是真实路径，就地读取，无临时文件 |
| **前端能读任意 path 的字节做分类/嗅探** | ❌ **不能** | webview 无任意 fs 访问；分类/PDF/缩略图字节**必须走 Rust 命令**按 path 读 |
| **pdfjs 用 asset URL 抽 workspace PDF** | ❌ **会失败** | workspace 原文件不在 asset scope；改 `read_file_bytes` → `getDocument({data})` |
| **canvas 直接读源图做缩略图** | ❌ **会失败** | 同上 scope 问题；改 `read_file_bytes` → blob → canvas → `write_thumbnail` |
| 内联块 retry 可复用 | ⚠️ 需持久化 | 仅进 `promptContent` 不存则 retry/重发丢上下文 → 存入 mixed 消息隐藏字段 `inlineContext`（§5.6） |

**核对后对计划的净修正**（含第三方报告有效项）：
1. 内联文本进 `promptContent`（经 `buildPromptWithAttachments`），**不碰** `messageContent`/可见气泡；并**持久化 `inlineContext`** 供 retry（§5.6）。
2. `buildPromptWithAttachments` 改 async 内联（或发送路径先 await 抽取再传入）。
3. **所有读字节走 Tauri 命令**（`inspect_attachment` / `read_file_bytes` / `read_text_file_preview`）；asset:// 仅渲染。
4. **PDF 抽取**：`read_file_bytes` → `pdfjs.getDocument({data})`，不用 path/asset（修 workspace 失败）。
5. **缩略图**：`read_file_bytes` → canvas 下采样 → `write_thumbnail`（`app_cache_dir`）；不依赖 asset scope/canvas 直读源文件。
6. **P0 扩成「附件 IO 基础设施」**：assetProtocol+scope + `PdfPreview` convertFileSrc + 上述 4 个 Tauri 命令。
7. 复用既有 `MAX_ATTACHMENTS_PER_TURN=4`；临时清理仅针对粘贴图片（方案 A）。
8. 修文档矛盾（line 61 base64→app cache，已改）+ 去重扫描版 PDF 风险（已合并）。

---

## 11. 落地状态（已实现）

- ✅ **P0**：`tauri.conf.json` 启用 `assetProtocol`（scope: `$APPCACHE`/`$APPDATA`/`$HOME/.future`/`$TEMP`）+ Cargo `protocol-asset` feature；`PdfPreview` 改 `convertFileSrc`；新增 Rust 命令 `inspect_attachment` / `read_file_base64` / `write_thumbnail` / `delete_temp_attachment`（`commands/files.rs`，注册于 `lib.rs`）+ 前端封装（`integrations/storage/files.ts`）。
- ✅ **P1**：`attachments.ts` 新增 `classifyAttachment`（Rust 嗅探）、`TEXT_EXTENSIONS`/`PICKER_EXTENSIONS`、`buildInlineAttachmentContext`（pdfjs 抽 PDF + `read_text_file_preview` 抽文本，单文件 30KB/2000 行、合计 60KB）；`Composer` 接受 image/pdf/text、其余 toast 拒绝；发送路径把内联文本拼进 `promptContent`，并持久化 `inlineContext` 进 mixed 消息（不入可见气泡）。
- ✅ **P2**：图片缩略图 `generateImageThumbnail`（canvas→`write_thumbnail`→appCache）；`MessageBlock` 的 `AttachmentChip` 渲染图片缩略图 / PDF·文本图标；artifact 预览（image/pdf/text）经 P0 修复后可用。
- ✅ **P3**：chat 发送后 `delete_temp_attachment` 清理粘贴临时图（护栏限 `futureos-attachments`；workspace 保留以便重发）。
- 验证：`tsc` / `eslint` / `stylelint` / `vitest`(15) / `cargo clippy` 全过；`npm run build` 成功；`cargo build`(src-tauri) 成功。
- agent / proto **未改**。

---

## 附：关键源码索引
- 协议/类型（仅图片）：`proto/future.proto:21-36`、`agent/src/types/mod.rs`
- agent 组装：`agent/src/rpc/session_prompt.rs:62-80`（**本期不动**）
- GUI 入口/过滤：`gui/src/features/agent/Composer.tsx`、`gui/src/features/agent/attachments.ts:5,59-69`
- 发送路径/门控：`gui/src/features/agent/useAgentThreadState.ts:168-248`、`gui/src/integrations/agent/agentClient.ts:65`
- chat→artifact：`gui/src-tauri/src/store/artifacts.rs:68-109,159-178`、`gui/src/integrations/storage/artifacts.ts:24-26`
- 文本读取命令：`gui/src-tauri/src/commands/files.rs:32`
- artifact 预览（image/pdf/text 已成）：`gui/src/features/artifacts/ArtifactDetailPanel.tsx:31-60,174-214`、`PdfPreview.tsx`
- 消息附件类型：`gui/src/features/agent/agentThreadTypes.ts:34`
