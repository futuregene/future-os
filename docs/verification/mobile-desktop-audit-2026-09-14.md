# Mobile / Desktop Remote 审查（2026-09-14）

## 范围与证据边界

源码检查起点为 `7ded5702`，提交前同步至 `d363beb6`。本次是按风险进行的源码审查、协议对照与移动端全量自动化测试，不是对每一行代码、所有设备或线上服务的无缺陷证明。

重点检查：

- 移动端 `client.ts`、`useRemoteConnection.ts`、`pairing.ts`、`storage.ts`：连接代次、候选交接、续期、可信配对入口、多 Desktop 凭证隔离。
- `secureChannel.ts` 与 Desktop `remote/commands.rs`、`remote/transfer.rs`：加密通道就绪、命令与回复路由、文件分块、重复命令语义。
- `syncEngine.ts`、`usePromptOutbox.ts`、`useSessionCatalog.ts`、`readPages.ts` 与 Desktop `remote_host/business.rs`、`read_pages.rs`：发送回执、回放水位、分页、目录与审批状态。
- `files.ts`、`useConversationController.ts`、`useFileDownload.ts` 与 Desktop `remote_host/files.rs`、`session_files.rs`：缓存身份、取消、迟到结果、大小限制、Windows 路径与受保护文件边界。
- App/聊天输入及草稿、系统分享导入、更新流程；Android `ShareIntentStore.kt`、`FileHandlerModule.kt` 的关键 IO 和文件访问路径。其余 UI 通过现有移动端测试覆盖，不宣称逐组件真机验证。

首轮 #604 的产品代码修改限于移动端 TypeScript，不改 Desktop Rust、NATS 消息格式、配对信任或审批执行规则。第二轮变更另列于文末。

## 已修复 / 加固

| 项目 | 原行为及影响 | 修复与回归证据 |
| --- | --- | --- |
| P1：预览元数据跨 Desktop / 会话复用 | 全局索引只使用路径、文件名和 variant；两台电脑的相同路径，或两条会话中的同名相对路径，可能直接展示另一来源的缓存文件，跳过当前 Desktop 的 prepare | 索引按 RemoteClient 和 session 隔离；不同 Desktop、不同 session 的相同路径用例。磁盘仍按内容哈希复用，不按来源重复存储相同字节 |
| P2：模型覆盖同路径文件后链接仍显示旧内容 | Markdown 文件链接默认可跳过 prepare；与不可变附件不同，生成的文件可原地更新 | 文件链接默认重新验证元数据，仍复用相同内容哈希的字节缓存；测试同时覆盖目录显式 refresh 和 Markdown 默认调用 |
| P2：首次下载后不能复用 prepare 结果 | prepare 先登记元数据，UI 在字节下载前查缓存时因文件不存在删除该登记；下载完成后仍需重新 prepare | 缓存 miss 保留有界元数据，每个 client 最多 128 条；测试覆盖 prepare → miss → 文件落盘 → 命中及容量淘汰 |
| P2：取消的 prepare 仍占用 Desktop 临时传输 | 上层 AbortSignal 先完成取消，底层请求稍后成功，其 transferId 无人释放；预先取消的调用也会先发出 RPC | 发起前检查取消；观察迟到成功回复并尽力发送 download_cancel；测试覆盖两种时序。回复永久丢失仍依赖 Desktop TTL，不声称能取消已不可知的 transferId |
| P2：取消 / 卸载后的下载重新弹窗 | 文件读取、下载或错误在取消后完成，handoff 没有核对 handle，可能覆盖新预览或弹旧错误；iOS 已开始 dismiss 的 handoff 不再属于 active ref | handoff 校验对象身份及取消；取消和卸载同时作废待展示 handle；原图准备取消在 finally 释放下载通道。测试覆盖迟到读取、卸载错误、原图取消后重试及 iOS onDismiss 队列 |
| P2：旧 Desktop 的审批回复污染新状态 | set_approval_tier / approval_decision 成功后直接使用当前 setter / syncEngine，不核对请求来源 | 回写前校验 client 身份；测试覆盖替换 Desktop 后的 tier 与 timeline 回复。仅修正手机显示，不改变 Desktop 实际审批结果 |
| P2：迟到附件 prepare 回填新会话 | 只在请求开始读取 selectedRef；响应后不核对导航身份 | 核对 client、session 和 conversation epoch，不匹配时取消原传输并返回取消错误；三类导航变化分别测试 |
| 防御性加固：下载分块尺寸 | 非正/非整数 chunkBytes 可造成错误循环；分块长度只在整个文件完成后才检验 | 下载前检查有限、安全整数及总大小上限；每块写盘前校验预期长度。测试覆盖 0、负数、NaN、Infinity、小数 chunkBytes，现有大小/哈希不匹配测试保持通过 |

测试入口：

- `mobile/src/remote/__tests__/files.test.ts`
- `mobile/src/remote/__tests__/useConversationController.test.ts`
- `mobile/src/features/chat/__tests__/useFileDownload.test.ts`

## 首轮后续问题与优化建议（#604 未修复；后续处理见文末）

这些条目没有被本次测试结果“洗绿”，应单独复现、评估并补充测试。

1. **Android 分享导入应在复制过程中限流，而非复制完成后检查大小（优先级 P2）。** `ShareIntentStore.take` 先 `input.copyTo(output)`，之后才比较 25 MiB 上限；超大或无结束的数据源可先消耗大量磁盘，异常分支也没有显式删除已写的半成品。建议增加累计字节上限、批量总量/数量上限和 finally 清理，使用可控 ContentProvider 做原生回归。源码已确认检查顺序；本次未做磁盘耗尽或原生故障注入。
2. **会话标题 override 需要明确失效协议（P2，待复现弱网时序）。** `useSessionCatalog.applySessionSnapshot` 总是优先采用 titleOverrides。如果手机漏收后续 Desktop 重命名事件，目录快照可能无法纠正旧名字。建议用权威版本/确认水位淘汰 override，不宜简单删掉它而重新引入旧快照覆盖刚重命名的竞态。
3. **附件转换和导出目录的资源回收（P3）。** `files.ts` 多文件 Promise.all 在单项转换失败时没有回收已完成的临时转换；`futureos-exports` 没有和 preview cache 一样的容量清理。需要建立临时文件归属，兼顾未发送草稿及外部 App 仍持有 URI 的情况，避免过早删除。
4. **测试卫生（P3）。** 全量通过的旧连接/目录测试仍输出 React act/overlapping-act 警告，部分故障注入输出预期 console.warn/error。建议逐一收敛 act 生命周期并断言预期日志，不用全局屏蔽 console 掩盖真实失败。

## 验证

Windows x64 worktree；Node **24.21.0**（与 CI Node 24 主版本一致）直接执行：

```powershell
# 在 mobile/；临时选择 Node 24，不替换用户全局 Node
npm exec --yes --package=node-win-x64@24.21.0 -- node ../node_modules/typescript/bin/tsc --noEmit
npm exec --yes --package=node-win-x64@24.21.0 -- node ../node_modules/eslint/bin/eslint.js .
npm exec --yes --package=node-win-x64@24.21.0 -- node ../node_modules/jest/bin/jest.js --runInBand
```

- TypeScript、ESLint：通过。
- Jest：**81 个套件、1023 项测试通过**（含同步进入的 #603 测试）。本 PR 新增 20 项回归用例。
- 相同范围也在本机 Node 26.4.0 通过；上面的 Node 24 结果才作为 CI 主版本对齐的本地依据。
- 未修改 Rust，因此未为此 PR 重跑整个 Desktop Rust 测试矩阵；协议核对是源码证据，不是新的 Rust 测试通过声明。
- 未执行 Android/iOS 原生构建、真机相机/相册、系统分享弹窗、锁屏后台、真实 NATS/平台断网与 Desktop/Agent 重启联调；未读取用户配对密钥或对运行中的 Agent 注入故障。

建议真机验收：两台 Desktop 的同路径不同内容；两条会话的同名相对链接；下载过程中取消并立即打开另一文件；prepare 过程中返回列表/切桌面；iOS 下载弹窗正在消失时返回；Desktop 重命名期间手机断网重连；25 MiB 以上 Android 分享源。

## 第二轮：剩余问题修复（基于 `617a7da1`）

本节是后续增量，不把首轮未验证的内容改写为首轮已通过。

- **Android 分享复制上限与回滚**：提取实际使用的 `ShareFileCopier` JVM IO 边界，单文件最多写入 25 MiB，每批累计读取预算 50 MiB、最多尝试 10 项。用至多一个额外字节区分恰好达到上限的 EOF 和超限源；超限、零进度、空文件、读取/关闭/输出打开异常都关闭流并清理半成品，后续元数据失败也清理。失败读取消耗的预算不会返还，避免多个超大源重复消耗磁盘。原有 composer 的 10 MiB/文件、20 MiB/消息限制不变。
- **标题最终一致性**：标题覆盖同步写入 ref；已确认的推送移除对应覆盖。目录请求捕获发起时的覆盖对象，只有该对象仍是当前对象时才可用返回快照移除它。改名成功后触发合并式刷新；旧请求保留新覆盖，后续读取才确认。显式读取允许同版本确认，但仍拒绝旧版本及错误 epoch；实时推送继续严格去重。覆盖错过的重命名事件、同版本回包、改名前已发出的旧读取和后续刷新竞态。
- **附件转换失败清理**：文件、相册、分享三条批量转换路径均等待所有转换结束，失败时只清理这一批新生成的临时输出；不动用户原件、已有草稿附件或可重试的分享输入。过大/空的转换输出也删除。SDK 抛错且没有返回输出 URI 时不能推断其内部临时路径，此类清理由 SDK 负责。
- **系统导出缓存**：每次导出使用带创建时间的独立目录，保留原文件名但不覆盖已交给其他 App 的同名文件。只在后续导出时清理至少 24 小时以前的导出副本，源文件和草稿不参与清理。容量为 100 MiB / 128 项；未满保留期的副本不为腾空间而删除，容量不足返回失败。序列化准备操作并清理失败副本。目录时间避免复制保留旧 mtime 时误删刚导出的文件。24 小时后系统 App 若仍持有旧 URI，不保证继续可读；这不是永久文件归档，应使用系统保存操作。
- **测试生命周期**：移除连接测试回调中嵌套的 async act，等待解绑/卸载的异步收尾；目录测试的异步启动放入 act。两组测试对 console.error 保留真实输出并断言零调用，避免未包裹/重叠 act 警告重新混入。故障注入的产品诊断日志仍可见，不全局屏蔽 console。

原生 IO 测试可用 `python scripts/test-mobile-share-io.py` 运行；复用 Java 17+，JDK 用 javac，仅有 JRE 时临时使用固定版本 Eclipse 编译器。JUnit/Hamcrest/编译器从 Maven Central 获取并校验固定摘要，临时目录退出即清理。Android 模块也声明了 JUnit 测试依赖，并新增路径限定的 `Mobile native IO` CI。它验证真实 Java 文件 IO，不冒充完整 APK/Kotlin/ContentResolver 集成测试。

第二轮本地验证：Windows x64 / Node 24.21.0，TypeScript、ESLint 通过，Jest **81 个套件、1040 项测试通过**（本轮新增 15 项，包含同步至 `e026af6c` 的列表回归用例）；Java 17.0.20.1 实际 IO 测试 **9 项通过**。测试依赖临时目录已自动清理。

第二轮仍未执行 Android/iOS 原生构建或真机测试；重点补验：恶意/超大 ContentProvider、分享权限失效、复制中磁盘满、断网期间 Desktop 改名、长时间持有导出 URI。未修改本地 `main` 的未合并提交。
