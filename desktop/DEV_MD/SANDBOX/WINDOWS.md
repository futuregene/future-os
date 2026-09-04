# Windows：Unelevated 写保护

更新：2026-09-04。W1–W7 后端、审批、维护与产品接入已实现；Windows11普通用户有原生、GUI核心写入、安装包生命周期及多主机历史PASS。**不是与Seatbelt等价的读写沙盒，也不是Elevated独立用户隔离。** 本页统一实现、取舍、真机操作与证据；共享规则/UI/参考见[COMMON.md](COMMON.md)。

## 1. 范围与方案选择

当前支持基线：Windows11、普通非管理员用户、绝对本地NTFS路径、PowerShell7优先/5.1回退。不假设Git Bash/WSL，不创建本地账户、不触发UAC、不设防火墙、不提供shell deny-read。

| 候选 | 取舍 |
|---|---|
| WRITE_RESTRICTED + capability SID + NTFS ACL | 当前方案，免管理员、读取兼容，接受写保护限制 |
| Elevated独立用户 | 需provisioning/UAC、跨用户runner/read ACL；后续候选，不作为本期缺陷 |
| 完整restricted token + 大量read ACE | 会破坏APPDATA/Cargo/Rustup/npm/Git/Python兼容，广泛修改真实ACL，不采用 |
| Low-IL | 需改变真实workspace完整性标签，可能扩大其他Low-IL进程写入面，不采用 |
| WSL / minifilter | 前者不是当前原生方案；后者需管理员、驱动签名与内核维护，未实施 |

产品档名为“写保护”。入口仅完整host probe通过后显示，不设独立W7开关。共享规则仍用于native read/write/edit，**shell默认可读取当前用户可读数据并联网外传**，不能把敏感守卫清单当作Windows shell拒读保证。

## 2. 技术方案与执行原理

```text
RuleSet → WindowsSandboxPlan
  → 校验声明的额外路径 + 可选审批receipt
  → policy/request capability records + 活动lease
  → 持久化metadata → 应用自己的ACE → 回收不活动旧代际
  → RestrictedToken + private desktop
  → CREATE_SUSPENDED → 加入Job → ResumeThread → shell/后代
  → Job结束 / 退出reset / 下次启动GC / 卸载cleanup
```

### 2.1 规则投影与token

`windows_plan.rs` 为平台无关纯计划：

| 字段 | 含义 |
|---|---|
| `writable_roots` | workspace、实际`temp_roots()`、字面allow-write根；去重包含关系 |
| `write_carveouts` | 字面ask/deny写保护，保留原decision |
| `unenforced_read_rules` | 不由shell强制的读ask/deny，仅诊断 |
| `unsupported_write_globs` | NTFS不可表达的写glob，native工具仍按规则 |

同一matcher抑制低优先级重复，但保留部分重叠父子规则。NTFS deny-wins不能表达所有first-match例外：显式ask/deny carveout即使再加更窄allow SID也可能仍拒绝，不能声称完整规则等价。

`CreateRestrictedToken(DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED)`保留当前真实用户主体。受限制的写权限需同时通过普通token与restricting SID检查；不能增加当前用户本来没有的权限。恢复`SeChangeNotifyPrivilege`用于路径遍历。

生产restricting集合是**capability + logon SID + Everyone，不包含真实User SID**。PowerShell/CLR启动要写session/Everyone可访问内核对象，仅capability真机会报HRESULT80070005；真实User SID则普遍命中用户文件ACL，破坏外部写边界。兼容宽SID带来既有ACL限制，见§4。token default DACL及private desktop只授当前用户+capability，不把新对象额外开放给Everyone/logon。

capability SID按确定性名称生成account-domain-shaped SID，只用作FutureOS ACL trustee/restricting SID，**不是AppContainer进程，也不使用DeriveCapabilitySidsFromName生成AppContainer SID**。

### 2.2 SID代际、ACL与生命周期

- 基础身份由“规范化写根 + 有效策略指纹”确定。同代际可复用，规则变更创建新代际，不修改仍被旧Job使用的deny ACE。
- 一次批准为基础写根和每个批准目标创建request-scoped ephemeral SID；目标identity绑定request id/path/file或subtree scope，不能因父根去重丢失独立批准目标，也不能复用历史subtree ACE扩大file批准。
- `~/.future/windows-capabilities.json` schema1存名称、语义、实际carveout与批准目标，不存SID指针；同目录原子保存。先持久化可清理记录再加ACL，失败留metadata供恢复。
- `FrozenPath`只接受绝对本地NTFS，以`FILE_FLAG_OPEN_REPARSE_POINT`打开，拒绝reparse/device/UNC等不支持形态；handle final path须与已冻结规范路径一致。ACL应用前重验，发生路径变化使批准失效，不靠字符串canonicalize一次了事。
- `SetSecurityInfo`仅幂等增加/撤销FutureOS自己的SID ACE，保留其他DACL，不改owner、不整表覆盖。现存字面保护加deny-write；NotFound目标跳过，其他错误返回失败。
- 根及后代按scope加write ACE；subtree后代可有DELETE，**不给父目录capability FILE_DELETE_CHILD**；file只给内容写、不授DELETE。这不消除真实用户父目录已有删除权限。
- 准备过程进程内串行；每个child持lease，首个lease持metadata目录byte-range文件锁。进程崩溃OS释放锁；本/其他进程有活动Job时GC/reset拒绝撤销，不终止命令。持锁不是跨项目的永久授权。
- 只将当前代际/request SID装入新token，旧ACE残留不直接授权未来token；仍需GC回收。按REVOKE_ACCESS移除自己的SID，失败保留metadata，不依赖命令结束瞬间清理成功维持代际边界。

### 2.3 进程、shell与输出

`Winsta0`中创建UUID私有desktop，ACL当前用户+capability；不用普通用户不可可靠创建的自定义window station。`CreateProcessAsUserW(CREATE_SUSPENDED)`配STARTUPINFOEXW handle allowlist，仅继承stdio；加入无breakaway、KILL_ON_JOB_CLOSE的Job后才ResumeThread。初始化失败先终止悬挂进程，不让命令运行。

正常shell退出也清理残留后代；不复用unsandboxed detached browser的BREAKAWAY_OK/disarm逻辑。cwd/env/Unicode命令行、wait/timeout/cancel统一走restricted driver。

受限PowerShell5.1进入Constrained Language Mode。UTF8Encoding构造、Console.OutputEncoding setter或.NET写字节不是可靠方案；包装器编码设置放try中，按`$Error.Count`增量判定命令错误，避免初始化编码异常污染成功exit。

捕获端`decode_restricted_shell_output`：5.1走`MultiByteToWideChar(CP_OEMCP)`，pwsh7走UTF-8。pipe无控制台时5.1使用OEM回退，不是CP_ACP；中文二者同936掩盖问题，西欧1252vs437/850、俄文1251vs866、希腊1253vs737不同。超OEM字符若已编码为`?`无法在解码端恢复，不宣称5.1任意Unicode无损。

### 2.4 Probe、维护与应用归属

host probe在临时NTFS夹具中实际启动private-desktop受限shell，验证allowed写成功、邻接正常用户可写路径被拒，并清理ACE。不是仅测token创建，也不是证明主机所有目录ACL都安全。稳定结果含available或`backend_initialization_failed`、`write_boundary_failed`、`restricted_shell_failed`；内部Win32诊断留本机日志，不进普通UI。

```powershell
future agent --probe-windows-sandbox
future agent --reset-windows-sandbox
```

也可用公共`--probe-sandbox`。probe成功执行时输出JSON，available:false是支持的不可用结果。reset返回`removedCapabilities`；活动Job失败并保留metadata，不提权、不杀任务。维护CLI/RPC无会话、在singleton前可调用。

长驻Agent持`~/.future/agent/agent-instance.lock`用户级锁，换gRPC端口不能绕过。正常桌面退出、确认强制退出后的任务收敛、清数据/环境切换/更新重启，都在bundled Agent存活时先reset再终止；外部Agent不由Desktop清理/终止。Agent自身Ctrl+C也幂等reset。超时、崩溃或临时失败best-effort保留metadata，启动GC、设置reset、卸载继续回收。

NSIS installMode为currentUser，卸载pre-hook以同一用户profile清理；reset失败应保留可重试sidecar，不先删除清理工具。不手工`icacls /reset`覆盖无关ACL。

## 3. 路径 capability 审批

Windows不能仅凭`Access is denied`/error5知道拒绝目标，因此**没有整命令脱写保护重跑**。shell调用前声明：

```json
{
  "command": "Copy-Item C:\\build\\artifact.zip D:\\release\\artifact.zip",
  "additional_permissions": {
    "write": [{"path": "D:\\release", "scope": "subtree", "reason": "创建发布产物"}]
  }
}
```

1. 解析并冻结规范绝对路径；file只接受已存在普通文件，subtree只接受已存在目录。拒绝卷根、用户HOME根、非法/不支持路径。创建、替换、rename须**明确申请已存在父目录subtree**，后台不静默扩大。
2. 同一RuleSet按write求值：allow不询问、fallback ask合并前置审批、deny直接拒绝。显式ask carveout因deny-wins不能通过窄SID重开，当前在弹窗前拒绝，不能展示“批准仍失败”的承诺。
3. 最多8目标，整组决策；过宽/过多须调用方重写请求，不能自动把多个子项扩到父目录。
4. 后端可信语义显示“修改文件”或“在目录创建、修改、重命名和删除文件”；标题目标与真实scope一致。模型reason不决定范围。普通UI不显示SID/ACL/hash/glob，命令折叠，payload不可解析不显示批准。
5. receipt绑定request id、command hash、normalized paths、scope；批准前后重验规则与handle。任何增删/变更须新请求。
6. 一次批准仅本命令ephemeral能力；项目允许由可信GUI保存同一目标allow-write并session注入，再生成新策略代际。敏感、hard-deny、配置/规则路径或不安全/过宽目标无持久允许。手机第一版仅一次/拒绝。
7. 批准后仍运行RestrictedToken，不提升用户自身权限。未声明外部写失败后可提示模型声明路径重试；stderr推断只帮助诊断，不生成授权。确需完全放开须用户明确切off。

## 4. 与macOS/Linux的差异和已接受缺口

| 情况 | Windows当前保证 / 限制 |
|---|---|
| shell读取 `.ssh`、models、`.env` | **不拒读**，即使native工具会deny/ask；读取+网络外传是明确非目标 |
| workspace/temp/字面allow写 | capability ACE开放，基础根来自实际temp_roots；不承诺独立session私有temp |
| 外部内容写 | 在支持且ACL满足边界的目标上默认拒绝；具体capability可批准，不提权 |
| 已存在字面ask/deny | deny-write ACE额外硬化，不等于任意删除/rename都拦截 |
| 缺失 `.env`/approval_rule或未来名称 | 无对象可贴ACE；允许父目录创建时无法只拦未来某个名字；**不提供Linux式post-scan补报** |
| glob | read/write shell均不强制，字段只诊断，不可把diagnostics理解为防护 |
| 宽deny+窄allow | deny-wins偏严，无法完整表达优先例外；不是再弹一次整命令批准就能解决 |
| file scope与删除 | 不授DELETE不等于禁止删除；普通用户父目录FILE_DELETE_CHILD仍可能允许删除目标或sibling |
| 既有宽ACL | Everyone/logon等命中restricting SID的写权可削弱外部内容写边界；Users/Authenticated Users等普通主体的父目录删除权也须审查，不能声称所有外部对象只由capability决定 |
| 状态与宿主修改 | 会修改真实NTFS ACL，需metadata/lease/GC/reset/卸载；macOS profile与Linux mount没有同样的ACL持久生命周期 |

真机AccessCheck曾出现external目录FILE_DELETE_CHILD=true而DELETE/FILE_WRITE_DATA=false，随后Remove-Item成功。这是WRITE_RESTRICTED模型的知情限制，不应修改测试宣传“file批准绝对禁止删除”；也不以此要求本期临时加Elevated。

网络/读取等模式差异在产品说明、首次启用/设置与开发材料说明；具体路径审批只突出当前行为+目标。既有宽ACL取舍需在测试记录注明，不能借一个临时fixture的外部拒写PASS推导全主机无绕过。

## 5. 开发进度、代码地图与历史踩坑

W0契约冻结，W1纯计划、W2token/ACL/capability、W3restricted driver、W4绑定审批、W5共享UI/手机、W6生命周期/诊断、W7动态入口均已落地；完整链路共同交付，避免只有入口无强制或只有ACE无回收。

| 代码（仓库根相对） | 职责 |
|---|---|
| `agent/src/sandbox/windows_plan.rs` | 平台无关投影及差异字段 |
| `agent/src/sandbox/windows/{capability,token,acl,audit}.rs` | 代际/request身份、token/SID、自己的ACE、handle校验 |
| `agent/src/sandbox/windows/{process,runner}.rs` | private desktop、Job、shell、lease、持久化/GC/probe/reset |
| `agent/src/tools/mod.rs`、`agent/src/rpc/approval.rs` | additional_permissions预检、receipt和执行 |
| Desktop supervisor/shutdown、NSIS hooks | bundled归属、先reset后终止、currentUser卸载 |
| `scripts/test-windows-sandbox*.ps1` | 原生与安装包生命周期验收，不进入CI |

2026-08-21～24原生排障的重要结论：

| 症状 | 根因 / 已有修复 |
|---|---|
| CreateWindowStationW error5 | 普通用户自定义station不适用，改Winsta0中UUID私有desktop |
| CLR HRESULT80070005 | 仅capability不足；加入logon/Everyone兼容，不加真实User SID |
| 成功命令exit1、CLIXML编码报错 | CLM禁止构造/设置编码污染$Error；try与错误增量，捕获端OEM解码 |
| file批准后Remove-Item仍成功 | FILE_DELETE_CHILD限制；保留知情边界，不伪造防删除结论 |
| 中文正常但西文乱码 | CP_ACP误用；5.1改CP_OEMCP，7用UTF8，无法恢复已丢字符 |
| probe生产lib E0433 | tempfile原仅dev依赖，补cfg(windows)生产依赖 |
| 单例测试未拿锁、写入真实用户目录 | dirs忽略测试HOME；统一绝对非空HOME→USERPROFILE→系统profile，锁/凭据/规则/workspace/capability一致 |
| Desktop lib-test崩0xc0000139 | TaskDialogIndirect需comctl32v6；Tauri原只给bin manifest，build.rs改所有目标嵌入v6，mt.exe验证 |
| Windows lint/日志测试假失败 | Rust1.97 lint修正、Unix-only import加cfg、日志smoke固定RUST_LOG=info |
| PS5.1生命周期Snapshot空数组null | `if`管道折叠数组；`41d458b3`显式初始化数组 |

## 6. 原生验收操作

必须在Windows11**非管理员**PowerShell运行；workspace与TEMP为本地NTFS，Rust按仓库toolchain、安装MSVC C++ Build Tools/SDK。关闭FutureOS和自行管理Agent；保留用户dirty文件，不执行git clean/reset。记录候选commit+git status，**不要照旧文档切回已合并的历史分支**。

```powershell
git rev-parse HEAD
git status --short
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
rustc -Vv
cargo -V
powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox.ps1 -IncludeClippy
```

IsInRole必须False；脚本拒绝管理员/非NTFS TEMP，`-AllowElevated`仅诊断，不算产品验收。原生测试强制单线程，不含UI自动化，不接CI；保存 `target\windows-sandbox-results\windows-sandbox-<时间>.log` 完整日志。

覆盖：token/SID/ACL/reparse/UNC/Job/PowerShell/Unicode/大输出，审批request/hash/path/scope，用户级singleton/强杀锁恢复，Desktop退出顺序，release统一CLI probe、Agent/Desktop Clippy、capability归零。Tauri测试只在缺少sidecar时造测试占位，结束仅删除脚本自己创建的文件，绝不覆盖已有真实sidecar。

PASS必须所有命令exit0，末尾同时有 `Remaining persisted Windows capability records: 0` 与 `RESULT: PASS`。`RESULT: UNSUPPORTED`表示受支持的fail-closed不可用，不是PASS；其他测试/cleanup/probe失败记FAIL并保留现场，不先手改ACL。不只截最后一行。

### 6.1 安装包生命周期（RM-01～07）

使用同一候选portable与NSIS：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-desktop-windows-portable.ps1 -SkipDeps
powershell -ExecutionPolicy Bypass -File .\scripts\build-desktop-windows-installer.ps1 -SkipDeps
powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox-lifecycle.ps1 -Action Snapshot
```

portable为根目录`FutureOS-portable-windows.zip`，保持FutureOS.exe/future.exe同目录；NSIS在`desktop\src-tauri\target\release\bundle\nsis\`。每次生命周期调用留独立时间戳日志，不记录完整命令行。

`-Action SeedCleanupFixture`仅record为0时写入指向`%TEMP%\FutureOS-Sandbox-Lifecycle-Fixture`的合法测试metadata，不加ACE、不覆盖真实记录。它证明应用/安装器到达reset，不单独证明真实ACE撤销（原生测试负责）。每个reset场景先seed，预期从1归0；中断后无活动任务时可用CLI reset恢复。

| ID | 操作与生命周期Action | 必须观察 |
|---|---|---|
| RM-01 | ExpectClean→启动Desktop→再启动→ExpectBundled | 同用户一个Desktop/一个Agent，Agent父为Desktop |
| RM-02 | ExpectBundled→SeedCleanupFixture→正常退出，等最多20秒→ExpectStopped | 两进程退出，record1→0 |
| RM-03 | 分别seed后清数据重启、开发环境切换重启、更新后重启→ExpectRecovered | 旧Agent退出、新Agent唯一且配置正确、端口释放、record0；无法触发者NOT RUN |
| RM-04 | 手启`future.exe agent --grpc-addr 127.0.0.1:50051`，Desktop连接→ExpectExternalAttached→seed→退出Desktop→ExpectExternalSurvives | 地址与FUTURE_AGENT_GRPC_ADDR一致；外部Agent及record1保留；其Ctrl+C后ExpectClean归0 |
| RM-05 | bundled+seed→任务管理器强杀→Snapshot→重启→ExpectRecovered | 强杀不承诺同步清理；singleton可恢复，启动GC归0；活动lease不误撤 |
| RM-06 | ExpectClean→seed→实际发布CLI `agent --reset-windows-sandbox`→ExpectClean；设置reset | 同一primitive，输出removedCapabilities；活动Job拒绝且不杀命令 |
| RM-07 | 普通用户NSIS，退出/ExpectClean→seed→卸载→ExpectClean | metadata清理、安装目录移除、夹具/用户数据与无关DACL保留；reset失败不先删sidecar |

### 6.2 产品与多主机矩阵

每台记录edition/build/arch/普通用户/NTFS/TEMP/shell/制品/Defender状态。最低覆盖Windows11Home+Pro，PowerShell5.1+7，ASCII与中文用户名/workspace，源码debug+portable+NSIS，Defender默认开启；管理员仅诊断。

| ID | 产品检查 |
|---|---|
| W7-01 | probe available、Desktop选项可见、只有一个bundled Agent |
| W7-02 | Desktop/已配对手机消费同Agent能力、语义一致 |
| W7-03 | workspace写成功、未批准外部预存目标内容写失败 |
| W7-04 | 卡片行为+完整目标，先拒绝目标不变，再一次批准仅所需内容写成功 |
| W7-05 | file批准不扩大父目录/sibling内容写；另一个文件须新审批/拒绝 |
| W7-06 | 外部Agent的probe为准，Desktop不另启/接管 |
| W7-07 | 重启probe成功保留sandbox；明确不可用回manual/隐藏Windows入口；瞬时连接故障按公共规则处理 |
| W7-08 | 正常退出ExpectStopped，record0，保留用户文件/无关ACL |

W7-03～05检查内容写入，不包含绝对防删除保证；宽ACL与FILE_DELETE_CHILD按§4记录。在启用前准备可控目标：

```powershell
$outside = Join-Path $env:USERPROFILE "Desktop\FutureOS-W7-Outside"
New-Item -ItemType Directory -Force -Path $outside | Out-Null
Set-Content -LiteralPath (Join-Path $outside "approved.txt") -Value "before"
Set-Content -LiteralPath (Join-Path $outside "sibling.txt") -Value "before"
```

拒绝后仍before，再只批准approved.txt；reset/卸载不得删除这两个用户测试文件。

报告模板：commit+dirty状态、Windows edition/build/arch、User elevated=False、workspace/TEMP NTFS、shell版本、包hash、Batch PASS/UNSUPPORTED/FAIL、每项RM/W7 PASS/FAIL/NOT RUN、remaining records、日志/截图、已接受限制。状态文件可能含路径，不公开贴完整JSON。

## 7. 历史验证证据（不是当前候选自动PASS）

| 日期 / 提交 | 结果与证据 |
|---|---|
| 2026-08-24 `a55c558200a80d2c2008c6ee2ef0c0c0ce86aa8e` | NT10.0.26200 AMD64、普通用户、NTFS、PS5.1、Rust1.97MSVC；原生50、capability11、singleton1、Desktop shutdown2、Agent/Desktop Clippy、release probe通过，record0。日志`target/windows-sandbox-results/windows-sandbox-20260824-102215.log` |
| 2026-08-24 11:21 `471a8cd79da99c3186a88448a0b685c0130cb2e4` | 干净工作区、Agent home C:\Users\FgClaw01；上述行为矩阵复验PASS、record0。此报告未带IncludeClippy；同提交Agent开发端Clippy另过 |
| 同主机SID实验 | capability+Everyone可启动，capability+logon不可启动；仅是该主机证据，不据此无矩阵删生产兼容SID |
| 后续packaged RM-01～07 | 原稿记录全部PASS：单例/退出、三种重启、外部归属、崩溃启动恢复、CLI reset、真实NSIS卸载；RM-05强杀当时已归0，停止后人工seed再启动为1/1/0 |
| P2多主机 | 原稿记录Pro/PS7/中文用户名与路径/portable矩阵完成；未逐项附commit与日志，本次不补造证据，发布复验应回填 |
| 2026-08-24 GUI Home | probe/入口、workspace内容写、外部内容写拒绝、正常退出record0通过；对应W7-01/03/08核心，不覆盖全部scope按钮、手机、重启回退或sibling |

旧文首“全部完成”与后文“部分产品项待验证”不应混用：当前保留确切底层日志与原稿后续PASS记录，但W7-02/04/05/06/07完整产品证据仍需逐项核实。本次文档整理未在Windows执行测试。

## 8. 后续计划与发布要求

发布仍要求：候选Home/Pro批量PASS、适用RM全部通过、W7 scope/手机/回退补证、无高优先级安全问题、不支持稳定fail closed、ACL升级/退出/崩溃/reset/卸载可恢复。不能以入口已开放替代安全review。

| 优先级 | 工作 |
|---|---|
| P1 | 当前候选完整原生与产品交互复验，保留逐主机/提交证据；复核宽ACL与最小兼容SID集合 |
| P1 | 持续审计reparse/handle变更、receipt篡改、活动lease/跨进程GC竞态，保证失败不Resume/不裸跑 |
| P2 | 单专用用户候选的独立设计与兼容评估；不是本期承诺 |
| 已接受 | shell读/网络开放、glob与未来文件名不强制、deny-wins、真实用户删除权、既有宽ACL、PS5.1代码页损失 |

### 8.1 单专用用户候选（未实施）

若产品目标升级到独立安全主体，优先评估Codex Elevated精简变体：一次性elevated setup创建普通本地用户（暂名FutureSandbox）与专组；随机凭据用DPAPI保存且沙盒用户不可读；真实用户Agent经CreateProcessWithLogonW启动跨用户runner，再在该用户下创建WRITE_RESTRICTED token与完整Job。

workspace/temp需同时授专组普通访问与capability第二道ACE，继续不给父目录capability FILE_DELETE_CHILD；工具链要补必要read/execute并排除敏感目录。真实用户owner/父目录删除权不再自动匹配，但Everyone/Users/Authenticated Users等宽ACL仍非绝对安全。

网络开放故候选只需一个用户，不照搬CodexOnline/Offline双用户与防火墙；未来网络隔离再评估WFP。必须单独覆盖UAC、账户隐藏、凭据保密、跨用户IPC、ACL刷新、升级/reset/卸载恢复，不能混入Unelevated维护。

不优先用PowerShell文本解析（无法覆盖任意子程序）、Low-IL（改真实workspace信任标签）或minifilter（驱动安装/签名/维护成本）代替这一身份边界。
