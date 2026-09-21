# CLI 授权与 Loop 工具化改造

日期：2026-09-21

状态：开发设计草案；描述目标架构，不表示接口、临时授权或安全修复已经实现。

[English](CLI_REFACTOR.md) · [远程执行方案](REMOTE_EXECUTION_DESIGN.md)

## 1. 定位与决策

本工程先于远程执行落地。它首先解决本地技能 CLI 依赖直接读取账户凭据而造成的沙盒例外，同时为 Local 与 Remote 建立相同的 CLI 架构。

确认两个核心目标：

1. CLI 统一向可信 Agent 申请受限的临时调用授权，不再自行读取原始账户 Key。Local 直连 Agent；Remote 通过 runner 与既有 SSH 控制连接转发到同一个 Agent 授权服务。
2. Loop 成为与 read/write/edit/shell 同级的 Agent 内置工具。模型通过技能学习工作流，直接调用工具，不再通过 shell 执行 `future loop` 来编排自己。

“CLI 从 Agent 获取 key”在本文中专指获取**平台认可的短期、限权限、限业务操作的授权凭证**。不提供 `get_key(provider)` 一类可返回原始账户或模型 Key 的接口。把原始 Key 从文件读取改成通过 socket 返回，并没有解决沙盒内进程获取长期凭据的问题。

共享的是命令解析、参数规范化、授权客户端、服务调用、文件处理和输出语义；本地完整发行包与远端轻量发行包不必拥有完全相同的管理命令。集成在同一发行包/二进制中，不等于 CLI 子进程与 Agent 共享内存、凭据或权限。

## 2. 当前事实与改造动机

| 当前事实 | 源码依据 | 必须改变的部分 |
| --- | --- | --- |
| 沙盒暂时把 `agent/auth.json` 和旧 `agent-app/auth.json` 移出硬拒绝列表，以兼容技能 CLI | [rules.rs](../../agent/src/sandbox/rules.rs)、[沙盒说明](../internals/desktop/SANDBOX/COMMON.md) | 授权通道可用后恢复读写硬拒绝；不能信任某个可执行文件名来豁免路径 |
| `tools` 按 `FUTURE_API_KEY → auth.json → 测试 Key` 读取认证；account 另有读文件实现 | [tools.rs](../../cli/src/commands/tools.rs)、[account.rs](../../cli/src/commands/account.rs) | 运行期统一走授权客户端，移除生产路径的秘密回退 |
| `future auth credential` 会打印原始账户 Key | [auth.rs](../../cli/src/commands/auth.rs) | 不允许从受限 CLI 获取或导出原始 Key |
| 统一 CLI 直接依赖 Agent/TUI/channel/Loop，部分普通命令还借用 Agent 的路径工具 | [Cargo.toml](../../cli/Cargo.toml)、[main.rs](../../cli/src/main.rs) | 把公共客户端与完整发行包入口拆开，避免 runner 拉入完整 Agent |
| 默认内置工具只有 read/write/edit/shell；run 开始时固定工具定义，提示词枚举工具 | [工具](../../agent/src/tools/mod.rs)、[run loop](../../agent/src/agent/run_loop.rs)、[提示词](../../agent/src/prompt/mod.rs) | 增加 Loop handler 与按技能启用的工具定义 |
| Loop 已有 library，但大量 CLI 编排在 console；验证器直接调用本机 shell | [library](../../orchestration/loop/src/lib.rs)、[console](../../orchestration/loop/src/console.rs)、[validator](../../orchestration/loop/src/validator.rs) | 提取共享业务服务；工具不能简单包装 CLI，更不能在控制端误执行远端项目验证 |

以上为静态代码核查；没有在本次设计中进行凭据提取、真实攻击或平台授权接口测试。

## 3. 一套客户端，两种连接方式

```text
Local:
  Skill 脚本 → future CLI runtime → 受限本地 IPC → Agent 授权服务

Remote:
  Skill 脚本 → 同一个 CLI runtime → runner 受限入口
                                     → 既有 SSH 控制连接 → Agent 授权服务

获得临时授权后，两者均：
  CLI → HTTPS → 目标云服务
  文件在执行所在地读取、上传、下载和保存，不经控制端中转

Loop:
  模型 → Agent 内置 Loop 工具 → Loop 业务服务
                               → 项目操作交给 ExecutionBackend
```

组件名称是职责命名，实施时遵循仓库 crate 约定，不预建一组空壳包：

| 组件 | 职责与依赖边界 |
| --- | --- |
| CLI runtime | 公共解析、文件 I/O、云工具客户端、结果格式化、浏览器命令；不链接完整 Agent、Loop 调度器或会话数据库 |
| AuthorizationClient | 类型化申请授权、查询业务操作、取消/查询结果；业务命令不判断自己在本机还是远端 |
| Local transport | macOS/Linux 私有 Unix IPC；Windows 受限 named pipe/handle；验证当前受控会话 |
| Runner transport | 把同一请求转发至当前控制端；不签发授权、不读取账户配置、不缓存长期密钥 |
| Agent 授权服务 | 身份与作用域检查、已有审批/额度策略、向平台申请凭证、记录非秘密操作回执 |
| 平台授权/工具服务 | 签发并验证凭证、绑定业务操作、原子去重、计费、任务查询和撤销 |
| Loop 业务服务 | 目标/任务/worker/验证编排的唯一实现，供工具与人工 CLI 适配器复用 |

本地可以继续以现有 `future` 统一二进制打包 Agent 与 CLI；远端由 runner 发行包提供 `future` 兼容入口，复用同一 runtime。入口采用子命令、受管理启动器或其他方式是打包细节，但不能为此依赖 Python/Node，且远端不得链接完整 Agent。基础路径/协议类型放入现有合适的轻量公共层，不复制 home 解析逻辑。

本工程先实现 Local transport 与可替换接口、契约测试；真实 Runner transport 随远程执行工程实现。接口设计不得依赖桌面 OS、桌面绝对路径或默认本地 cwd。

## 4. 授权边界：不是把文件漏洞变成 RPC 漏洞

### 4.1 原始凭据只属于可信控制端

Agent 读取现有凭据存储；本工程不要求迁移全部账户文件或引入新的跨平台密钥库。模型 Key、账户 Key、刷新凭据不下发给 CLI、runner、技能脚本或工具输出。

运行期 CLI 不读取 `auth.json`，也不回退到 `FUTURE_API_KEY`、测试 Key、其他 home 或其他 Agent。测试只能通过显式 mock/测试注入提供假授权。Agent 启动环境中的原始秘密不能继承给 shell、CLI、浏览器或它们的子进程；仅删除一个变量不足以完成环境隔离，必须审核现有环境构造与受控传递规则。

恢复默认 home、重定向 `FUTURE_HOME` 以及旧凭据路径的读写硬拒绝，并验证实际平台沙盒，而不仅是规则字符串。符号链接、路径别名和子进程不能绕过保护。不把关闭沙盒或批准升权当作兼容认证失败的常规方法；用户明确关闭隔离的执行不在沙盒保密承诺内。

### 4.2 受限通道与管理通道分离

沙盒内只允许访问授权服务的窄接口，不开放完整 Agent RPC socket。现有管理 RPC 包含凭据、模型、会话和策略变更能力，不能因为需要授权就整体暴露给 shell。

Agent/runner 在启动受控执行时建立作用域，绑定既有 workspace、session/run、执行目标和生命周期。身份从可信连接/受控句柄导出；不能信任请求 JSON 自报的 session ID、目标 ID 或权限。

Unix peer UID、Windows 用户 SID/ACL 是必要的账号边界，但单独使用不足以隔离同一用户下的不同 run。通道还必须绑定受控执行上下文。优先继承受限 FD/handle；若平台需要 rendezvous，则使用短期、单次兑换且上下文绑定的连接凭证，不通过 argv、日志或通用环境变量传递秘密。仅在环境中提供非秘密发现信息不能成为授权依据。

同一已授权沙盒内的任意代码都可能使用该有限能力；不宣称能仅凭进程名、PID 或可执行文件路径区分“官方 CLI”和恶意兄弟进程。安全目标是它们最多获得该上下文允许的一次业务授权，永远不能获得原始 Key 或管理权限。平台原子消费限制复制凭证的重复使用，但不保证同一受损上下文中的善意进程一定抢先消费。

临时凭证仅在 CLI 必要内存中持有，操作结束/过期/连接失效后丢弃；不写 auth 文件、缓存、环境、argv、stderr、追踪或会话记录，不传播到无关子进程。不承诺对 root、调试器、内存转储或同 UID 特权进程实现绝对保密。

### 4.3 审批与网络约束

拿到通道不等于允许任意工具。Agent 检查工具 allowlist、目标服务、调用参数、数据上传边界以及现有审批/预算规则；不能因客户端声称来自某个技能而放行。未实现硬金额限制时不得对外承诺硬预算。

凭证由可信配置确定目标服务，禁止客户端指定任意签发/收款/转发端点；HTTP 重定向不能把认证头转发到未授权 origin。CLI 的直接 HTTPS 调用仍受执行端网络策略约束，不自动开放全部网络，也不隐式退回控制端文件代理。自建平台未支持授权协议时明确报告不支持。

## 5. 临时授权协议与重试

“通用能力”是统一的申请、生命周期和错误语义，不是一个适用于所有第三方网站的万能 Key，也不是一期实现完整 OAuth 框架。

建议最小业务接口如下；名称待实现确定，不与模型工具名混淆：

| 接口 | 输入 | 返回 |
| --- | --- | --- |
| `authorize_operation` | `operation_id`、工具名、规范化业务参数/输入清单 | 临时授权、可信服务地址、到期信息或已有操作回执 |
| `get_operation` | `operation_id` | 已提交任务的状态/结果引用，或明确的 unknown |
| `cancel_operation` | `operation_id` | 服务实际确认的取消结果；不支持时明确返回 |

上下文绑定由通道提供，不重复建立 controller_id/runner_id/workspace_id 映射。Remote 接入时复用远程方案的 target、instance、context、control epoch 及 operation 身份，不建立第二套 SSH 会话世代。

凭证至少约束：授权主体/会话、目标服务、工具及业务操作、参数与输入身份、有效期、使用次数/任务范围。绑定模型/质量/数量等影响成本的参数；文件路径不能当作文件内容身份，可采用长度/hash 或服务已确认的上传对象 ID。平台核对实际提交内容，不能只验证客户端自报摘要。具体 wire 格式与平台共同确定。

平台负责签发和校验。Agent 不能随意生成一个随机字符串并假定现有工具接口认可。原始账户 Key 仅用于 Agent 与平台之间的认证。

一次授权对应**一次逻辑业务操作**，不是一次 HTTP 请求。现有 MCP 初始化、文件上传、提交、轮询、下载等可作为同一任务的受限步骤；不得借这些步骤创建第二个收费任务或读取别的任务。只读工具描述可来自公共版本化目录或经 Agent 获取非秘密数据，不为 `describe` 恢复读 Key。

去重与续接规则：

- 同一 operation ID + 相同规范化请求，重试查询/恢复已有操作；不同请求复用 ID 必须拒绝。
- 平台在接受业务提交/计费时原子消费创建权限并生成持久回执。返回包丢失后不能用新 ID 自动再做一次。
- 续发授权仍绑定原 operation，只能恢复允许的剩余步骤，不重置用量或生成额度。
- 平台无状态查询或断线导致无法确认时，返回 outcome unknown，不能把本地超时解释成云端取消。
- 运行状态可用 `authorized/submitted/running/succeeded/failed/cancelled/unknown` 表达；凭证过期/撤销是独立生命周期，不是任务失败。

连接失效时拒绝新的授权请求、丢弃本地临时凭证；重连先恢复既有操作事实并作废旧会话未使用授权，再开放新申请。Remote 复用控制连接世代，Local 以 Agent/受控执行生命周期处理重启与撤销。平台不可达时无法保证立即撤销，短有效期提供上界；不能宣称“内存清零等于服务端失效”。已经接受的任务不因重连自动重提或取消，新授权可以只允许查询原任务。

## 6. CLI 命令兼容与能力范围

| 能力 | 改造后的行为 |
| --- | --- |
| `future tools call` 云工具 | 同一 runtime 获取授权后直连服务；尽量保留参数、`--stdin/--input/--mask/--output/--raw`、业务输出和退出语义 |
| `future tools list/describe` | 不需要账户 Key 暴露；目录与实际可用能力一致 |
| `future tools call browser` | 在当前执行主机操作浏览器，无需云授权；远端 Linux 支持已安装 Chromium 的 headless 模式；缺少浏览器或启动失败直接报错，不自动回到控制端 |
| `future account profile/balance` | 一期由 Agent 返回只读业务结果，复用受限接口；无需下发账户 Key，未来可换只读临时授权 |
| 模型的 Loop 编排 | 使用 Agent Loop 工具，不需要运行期 CLI 回传 Loop 命令 |
| 人工使用 `future loop` | 保留兼容入口并迁移为同一 Loop 服务的客户端；管理能力不自动进入技能用的受限 CLI，远端一期不承诺该命令 |
| `future models` | 查询控制端非秘密模型目录；Loop 工具可直接使用该目录，不要求模型另起 shell |
| skill 创建 | 项目中编写/验证文件可用；远端应用技能库安装/更新不支持时明确失败，不增加反向安装系统 |
| login/logout/provider 管理 | 完整本地客户端的管理入口，交由可信 Agent 处理；不出现在受限 broker 中，也不经 runner 转发 |
| `auth credential` | 受限 CLI 明确拒绝；完整客户端的原始 Key 导出接口列为废弃并安排兼容迁移，不能通过通用 RPC 从沙盒间接调用 |
| 第三方数据库/SDK 密钥 | 不自动同步 `.env`、用户环境或 SDK 凭据；公共接口可用，缺少第三方授权则失败，不默认增加泛用代理 |

本地/远端的输出路径始终属于执行主机；相同 CLI 不代表相同文件系统。浏览器运行资源在 Local 沿用已有根，在 Remote 使用既有 runner 隔离根；缺少库/浏览器属于可选技能能力不足，不变为 runner 强制依赖。

交互式本地 CLI 没有 Agent 启动的 run 上下文时，由可信本地会话入口建立明确的用户授权作用域；不能省略身份和策略检查。Agent 未启动时，完整本地入口可按已有生命周期规则启动 Agent；沙盒内 runtime 和远端不得自启完整 Agent、找其他用户实例或回退读凭据。首次登录/bootstrap 使用管理入口，不需要预先拥有业务授权；CLI 只展示流程结果，不接收最终原始 Key。

“同一套架子”指运行期共用实现与协议；不能以命令同名为理由把安装、登录、账户管理和运行技能的权限合并。

## 7. Loop 内置工具设计

### 7.1 工具接口与业务服务

从现有 console 逐步提取 Loop 服务，保留存储格式与业务规则，避免新建第二套目标/任务数据库。工具和人工 CLI 适配器调用相同类型化接口，不拼接 shell，不解析终端文本，不在 Agent 内调用会再次创建 Tokio runtime 的 console 入口。

一期用一个有限的 `loop` 工具入口，按操作提供明确参数校验和结构化结果；操作覆盖技能实际依赖的目标、任务、gate、worker、steer、状态与完成流程。不得仅接收任意 CLI 命令字符串。具体 action/schema 与现有技能逐项映射后审查，避免将整套 console 无选择地暴露。

工具的身份、目标 workspace、权限和模型配置由 Agent 会话解析；模型不能通过传入本地路径或 session ID 接管其他任务。预算、审批、停止和生命周期规则不因技能已加载而失效。运行/监视型操作返回任务标识与状态，由现有后台调度继续，不长期阻塞模型工具调用，也不持有会话锁等待同一 Agent 创建 worker 而死锁。

### 7.2 技能触发的工具可见性

三件事分开：内部注册、模型 schema 可见、具体操作获准。

1. Agent 内部注册 Loop handler；默认基础提示词与工具列表不主动展示 Loop。
2. 用户显式选用或模型通过可信技能加载入口解析内置 `future-loop`，Agent 返回技能内容，同时为该会话启用 Loop 工具定义。
3. 下一次模型请求带上 schema。不能只在 SKILL.md 宣称工具存在，也不能等待下一次用户消息才能使用。
4. 会话恢复/上下文压缩后，由 Agent 保存的非秘密能力选择状态重建可见性，并重新核对策略和技能版本；不是从任意历史文本推断权限。派生 worker 依其技能与权限显式确定能力，不无条件继承管理权限。
5. 普通文件 `read`、任意路径伪装成 SKILL.md、第三方 frontmatter 或网页文本都不能激活特权工具。用户自定义同名技能不等于可信内置技能。

为此，技能加载需要 Agent 可观察的显式入口。优先复用远程方案提出的 `resolve_skill` 语义，并先实现 Local 路径；当前 Skill 结构和文件读取本身尚不具备该机制。工具 schema 在每轮模型请求前按当前能力生成，重试保持该请求的定义快照，schema 与实际可调用 handler 必须一致。使用有限的内置技能到工具映射即可，不建设通用插件权限系统。

### 7.3 执行位置与存储兼容

- Local 现有 Loop 项目状态尽量沿用原路径与格式，工具定位必须显式绑定项目，不能依赖 Agent 进程 cwd。
- Remote Loop 状态留在控制端，与远端项目根区分；使用控制端既有 workspace 数据根定位，不把 Linux 路径拿到 Windows/macOS 上打开或建目录。
- worker 继承固定目标和项目执行根；文件、验证命令、环境探测统一交给 ExecutionBackend。Local 先通过同一接口调用原逻辑，远程接入后不另写一套验证器。
- CLI 与工具访问同一 Loop 状态时复用现有并发/锁约束，不同时启动两套 watchdog、调度器或独立状态写入者。Agent 生命周期内的后台任务归属、取消和重启恢复需明确，不借工具化顺便重做 Loop 业务。

## 8. 迁移顺序与验收门槛

| 阶段 | 交付物 | 完成门槛 |
| --- | --- | --- |
| A：协议与依赖切分 | 公共 CLI runtime、Local/Runner transport 契约、命令/能力清单；平台确认授权与幂等协议 | 轻量 runtime 不依赖完整 Agent/Loop/TUI/channel；明确不支持的命令 |
| B：本地授权闭环 | Local IPC、作用域校验、平台临时授权、CLI 云工具迁移、日志/环境清理 | 本地主要内置技能在不读取原始 Key 时正常完成，未知结果不会自动重做 |
| C：关闭凭据例外 | 恢复所有相关 auth 路径硬拒绝，隔离管理 IPC、移除秘密回退与受限导出 | macOS/Linux/Windows 真正沙盒内正反例验证通过；未测平台不得声称修复完成 |
| D：Loop 工具化 | 共享业务服务、按技能暴露 schema、worker/验证执行接口、人工 CLI 兼容 | Local 工作流与已有状态兼容；不产生双调度器、错误 cwd 或同 Agent 死锁 |
| E：远程工程接入 | runner transport、SSH 世代/恢复绑定、远端轻量入口及浏览器能力 | 使用同一契约测试；不复制 CLI 业务逻辑、不引入远端长期秘密/会话数据库 |

A—D 是远程执行的前置工程；E 属于远程执行实现。平台能力是 B/C 的实质依赖，不能用原始 Key 临时下发来宣布修复完成。可以先做 mock 驱动开发，但 mock 通过不等于安全闭环上线。不支持新协议的平台明确失败；不长期保留静默兼容开关继续读 auth.json。

必要验收场景：

| 类别 | 核心场景与预期 |
| --- | --- |
| 正常调用 | CLI 云工具 `web_search`、论文检索、ParseDoc、图片输入/mask/输出、slides 脚本；业务参数和产物仍兼容；`web_search` 不等于独立 Agent `search` 工具 |
| 凭据保护 | shell/Python/子进程读写新旧 auth 路径、默认/重定向 home、路径别名均失败；正常 CLI 成功 |
| 入口保护 | 自报 run、换 workspace、过期句柄、同 UID 错误作用域、访问完整 RPC/管理命令均拒绝 |
| 授权滥用 | 修改工具/模型/质量/数量/输入/目标 origin、复制 token 重复提交、续发后重复计费均被拒绝或返回同一回执 |
| 秘密泄漏 | stdout/stderr、日志、模型输入、journal、argv、环境、崩溃报告不包含原始或临时凭证；代理不得记录完整授权帧 |
| 弱网 | 申请后丢包、提交成功但回包丢失、上传中断、Agent/runner 重启、重连撤销失败；恢复原 operation 或明确 unknown |
| Loop | 同一模型 run 内加载技能后即可调用；恢复后 schema/handler 一致；取消/steer/验证/后台生命周期正常 |
| 跨平台 | Windows 控制端连 Linux 的模拟契约不使用 Windows shell/路径解析远端项目；实际 Remote 验收在 E 完成 |
| 可选能力 | 远端无浏览器/无法启动、技能库安装不支持、第三方密钥缺失均明确失败，无本地回退 |

连接/任务状态与错误码分开。错误至少区分 Agent 不可用、上下文失效、权限拒绝、平台不支持授权、授权过期/撤销、幂等冲突、结果未知、能力不支持；错误码不作为新的状态机枚举。认证失败不输出任何凭证，不自动触发登录、付费重试或扩权。

## 9. 实现前 Review 重点与非目标

Review 必须落实的细节：平台授权接口及单任务多步骤范围、跨平台沙盒内 IPC 传递、手动 CLI 身份建立、完整管理端点的实际隔离、Loop 原有用户与后台调度兼容、状态目录映射、错误/退出码兼容、原始 Key 导出废弃计划。这些是实现细化项，不改变“原始 Key 不进入运行期 CLI”的决策。

不是本期目标：第三方服务通用密钥库、把任意第三方 Key 换成一次性 Key、任意 CLI/RPC 反向代理、远端技能安装系统、远端完整 Agent、浏览器自动安装、HPC 调度、端口转发、全部账户存储迁移。依然允许用户项目使用自己的依赖，但 FutureOS 不自动复制控制端秘密。

本方案改变 CLI/Loop 的前置架构；远程方案仍负责 SSH、主机身份、单占用锁、runner 目录、Skill 快照、执行上下文和 Files/Review/Terminal。两份文档遇到 CLI 凭据/Loop 接入表述差异时，以本文的职责划分为准，并在实施时同步契约。
