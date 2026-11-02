# Windows 安装器预检

> （[English](desktop-windows.md)）

## Visual C++ 运行库

仓库的 `.cargo/config.toml` 为 `x86_64-pc-windows-msvc` 启用了 `+crt-static`，
把 CRT 静态链进桌面端与 CLI/agent。NSIS 与便携版构建都用此配置。这样
`future.exe` 在启动、探测沙箱或卸载清理沙箱权限之前，就不需要先装机器级
Visual C++ Redistributable（也免了提权）。打包构建不得用省略 `+crt-static` 的
`RUSTFLAGS` 或 `CARGO_ENCODED_RUSTFLAGS` 覆盖这些 flag。

Windows 发布验证时，用 `dumpbin /DEPENDENTS` 检查 **两个** 二进制
`future.exe` 与 `futureos.exe`：两者都不应导入 `VCRUNTIME140*.dll` 或
`MSVCP140*.dll`。在未装 VC++ Redistributable 的干净 Windows 机器上测试安装、
桌面端/agent 启动、沙箱探测与卸载。只在装了运行库的开发者机器上构建成功是不够的。

这只影响新构建的二进制。已安装的旧版本如果其卸载器调用的是旧的动态链接
`future.exe`，在"先卸载再升级"能继续之前可能仍需先修复 Microsoft 运行库。

## 进程与文件访问检查

Tauri 默认的 NSIS 运行中应用检查只覆盖主程序。捆绑的 `future.exe` 可能比桌面端
活得久，升级期间保持文件锁定。`installer-hooks.nsh` 在 Tauri 替换任何可执行文件
**之前**、以及新卸载器执行沙箱权限清理之前做文件访问检查。

- 检查 `$INSTDIR` 中的当前桌面端（`futureos.exe`）、旧桌面端（`future-desktop.exe`）、
  无头宿主（`futureos-headless.exe`）、CLI/agent（`future.exe`）以及已退役的独立 Agent（`future-agent.exe`）。持锁元数据同时核对 PID、创建时间、完整路径和数据根。
- 说明关窗不等于停止 agent。提供"是"（关闭本安装并继续）、"否"（手动退出后重查）或
  "取消"。取消是默认项，退出且不替换可执行文件。
- 关闭进程需要交互式同意，并给出任务/会话中断警告。可执行文件完整路径匹配不区分
  大小写；先关桌面端进程，再关 agent。绝不只按进程名 kill，也绝不 kill 进程树。
- 探测现有文件时不得截断它们；创建/删除一个唯一目录做写探测。即使没找到匹配进程，
  外部锁、只读文件和权限也可能阻塞安装。
- 关闭失败时提供"重试/取消"并附任务管理器与重启指引。写入/检查失败时提供
  "重试/取消"并附权限、磁盘空间、安全软件与默认目录指引。没有"忽略"通道。
- 通用静默（`/S`）与被动（`/P`）安装从不关闭程序，也不显示阻塞对话框：文件被占用
  退出 **32**，写入/检查失败退出 **5**。`scripts/release/install.ps1` 把这些码转成恢复指引，
  而不是继续初始化。
- 旧应用内更新（`/UPDATE`）会关闭本安装目录中精确路径匹配的可执行文件，以兼容旧版本未停止 sidecar 的情况。新宿主传入 `/FUTUREOS_CLEAN`，仅检查、不关闭进程；其他目录中持锁的外部宿主也阻止自动升级。只有交互安装明确选择“是”，才授权停止同一数据根且身份已核实的外部宿主。
- 在 Agent 锁租约内，先用安装包携带的本次构建维护 CLI 完整清理 Windows 沙箱权限，再备份和替换文件。清理错误或超时直接中止安装；旧 CLI 缺失/不支持维护命令不会妨碍使用新维护代码。
- 预检通过后，在 Tauri 复制新桌面端之前删除当前与退役的 sidecar 名称。替换前，把
  当前与遗留可执行文件保留在 NSIS 临时存储中。复制/解压失败时恢复原样这一组；首次
  安装失败则移除不完整的可执行文件。
- 复制完成后，运行有界的 `futureos.exe --help` 与 `future.exe --version` 探测。它们
  在 GUI/Agent 初始化前就返回，同时真实走了一遍 Windows 加载器。DLL 缺失、文件损坏、
  架构不符、非零退出或超时都会中止安装并恢复之前的可执行文件。

应用内更新先下载验签，再确认中断范围，执行可失败的完整 Desktop/Agent 清理。清理失败不启动安装器。独立观察进程等待原 Desktop 句柄退出后，以 `/P /R /UPDATE /FUTUREOS_CLEAN` 启动 NSIS，检查启动错误与退出码。安装失败只有在全部原可执行文件哈希恢复、无回滚失败标记时才重新启动旧程序；否则保留修复诊断。恢复日志在 `FUTURE_HOME/app/logs/update-recovery.log`。更新检查期限 30 秒、下载请求期限 15 分钟、观察进程就绪期限 10 秒、等待 Desktop 退出期限 60 秒；安装提取过程不因超时被强杀。Windows 便携版仍不执行 NSIS 原地更新。

helper 被嵌入 NSIS 的临时插件目录，不随应用安装。即使从 32 位 NSIS 调用也要用原生
Windows PowerShell，以便检查 64 位进程路径。若 PowerShell 无法运行，fail-closed 并
给出指引。常规的当前用户/非提权安装模式不变。卸载先关闭精确路径的当前与遗留进程，
再在 Tauri 删除卸载记录之前验证每个当前与退役可执行文件都能删除。交互式移除提供
"重试/取消"；静默/被动移除遇到锁时退出 32，而不是文件还在却报告成功。临时可执行
文件备份还封住了访问检查与删除之间的竞态：删除最后失败时恢复卸载前的可执行文件
集合。这避免了 `/REBOOTOK`——其延迟删除路径需要当前用户安装器不具备的权限。沙箱
重置保持尽力而为且有时间上限，缺失、过旧或损坏的 CLI 不会挂起或阻塞移除。
Capability 元数据留在 `$INSTDIR` 之外，供后续安装/修复重试。

## 离线回归

阶段六离线 hooks 和升级观察器已在 2026-10-15 的原生 Windows CI、daily 未签名打包中通过，执行证据见[开发计划第 10.10 与 10.13 节](../architecture/embedded-agent-development.zh-CN.md)。回归使用临时模拟安装器，并验证真实产品包内容，不安装真实 FutureOS；干净 VM 的真实升级和回滚仍需单独验收。

在装有 NSIS 的 Windows 上（也会检测 Tauri NSIS 缓存）：

```powershell
& ./scripts/tests/test-windows-installer-preflight.ps1
& ./scripts/tests/test-install.ps1
```

测试用 NSIS `/WX` 编译真实的 hooks，创建无害的临时可执行文件，只按 PID 驱动测试
安装器的对话框。覆盖：全新安装、中英文指引、取消/否/是、静默/被动失败、运行中混合
安装的自动更新恢复、精确路径关闭（含两个遗留可执行文件名）、外部锁、重试、只读
文件、Agent 运行中卸载、任意清理失败、沙箱前的混合 CLI、挂起的清理、缺失的 CLI、
fail-closed 锁定卸载、复制后加载验证、复制/验证失败后的回滚。它不会启动、停止或
修改开发者机器上的真实安装。Windows 打包工作流在发布制品前运行这些检查。

发布前，还要在上一版本之上冒烟测试**完整打包安装器**，覆盖"重装"与"先卸载再升级"
两种选择。旧版本的卸载器包含的是它自己的原始 hooks，不是新的。任何预检都无法阻止
其他应用在检查之后获得锁；安装完成前避免重新打开 FutureOS。

## 无终端的启动没有可写的 stderr

由资源管理器启动、安装器"立即运行"或开机自启拉起的进程没有控制台窗口，fd 2 要么不存在，
要么指向一个本进程无写权限的句柄。`eprintln!` 在这种情况下不是"丢掉这行"：
`std::io::stdio::print_to` 在写入失败时 panic，直接结束发射它的那个任务。

2026-11-01 在同一个 `futureos.exe` 进程的 `app/logs/crash.log` 里实测到 **699** 条
`panicked ... failed printing to stderr: 拒绝访问 (os error 5)`，全部在 `tokio-rt-worker`
上，且每一条都固定出现在中继报出 `authorization violation` 之后 **11.1 秒**。当天两件后果
都发生了：远程桌面停掉的原因在磁盘上查不到，而 panic 可能杀掉本来要去重建那条链路的任务。

两层措施，缺一不可：

- **`future_agent::console::redirect_windowless_standard_streams`**，由
  `future_host::environment::prepare_process` 最先调用：`GetConsoleWindow()` 为空时，把本进程的
  stdout/stderr 指向 `~/.future/app/logs/desktop.stdio.log`（超过 4 MiB 换新文件），于是**任何**
  `eprintln!`（std、依赖、没人审过的代码）都会写进文件而不是 panic。从终端启动的进程保持原终端。
- **容错写入器**，用于运行**中途**失效的流：`console::eprintln!` / `console::println!` 丢弃失败的写入，
  `logfile::eprintln_log!` 同时镜像进 `desktop.log`，remote 栈则统一走
  `future_remote_protocol::diagnostics`（host 把它接到 `environment::report`）。启动之后才断掉的终端
  在启动时无法检测，所以长生命周期任务里的诊断不能再用裸 `eprintln!`。

本仓库新代码的规矩：长生命周期任务里的诊断必须走上面三者之一；如果 `grep -rn 'eprintln!'`
能搜到你那行，就说明它没有。
