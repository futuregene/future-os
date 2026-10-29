import pathlib

p = pathlib.Path("README.md")
s = p.read_text()

# 1. The execution-boundary bullet names the tier most readers should pick.
old = "- **Choose the execution boundary.** Configure file-access approvals and OS sandboxing for agent tools. The desktop defaults to Unrestricted (`off`); select Manual or Sandboxed before working with untrusted content. See the [security model](SECURITY.md) and [sandbox guide](docs/wiki/en/Sandbox.md) for defaults and limitations."
new = "- **Choose the execution boundary.** File-access approvals and OS sandboxing for agent tools, in four tiers: `off`, `manual`, `sandbox` — and **`auto`**, the sandbox with the model reviewing the requests that would otherwise interrupt you. The desktop starts at Unrestricted (`off`); switch to Manual, Sandboxed or `auto` before working with untrusted content. See the [security model](SECURITY.md) and [sandbox guide](docs/wiki/en/Sandbox.md) for defaults and limitations."
assert old in s
s = s.replace(old, new, 1)

# 2. The TUI row stops claiming bilingual chrome.
old = "a queue band for prompts typed mid-reply — and the whole interface in **Chinese or English**, following the system or `/lang` ([guide](docs/guide/tui.md)) |"
new = "and a queue band for prompts typed mid-reply ([guide](docs/guide/tui.md)) |"
assert old in s
s = s.replace(old, new, 1)
p.write_text(s)

q = pathlib.Path("README.zh-CN.md")
z = q.read_text()

zold = "- **选择执行边界。** 为 Agent 工具配置文件访问审批与 OS 沙箱。桌面默认是不受限（`off`）；处理不可信内容前，请切换为手动或沙箱模式。默认值和限制见[安全模型](SECURITY.md)与[沙箱指南](docs/wiki/zh/Sandbox.md)。"
znew = "- **选择执行边界。** 为 Agent 工具配置文件访问审批与 OS 沙箱，四档可选：`off`、`manual`、`sandbox`，以及 **`auto`**——开着沙箱，由模型复核那些本来会来问你的请求。桌面默认是不受限（`off`）；处理不可信内容前，切到手动、沙箱或 `auto`。默认值和限制见[安全模型](SECURITY.md)与[沙箱指南](docs/wiki/zh/Sandbox.md)。"
assert zold in z
z = z.replace(zold, znew, 1)

zold = "以及回复进行中输入时用的排队带——整个界面支持**中文或英文**，跟随系统或 `/lang`（[指南](docs/guide/tui.md)） |"
znew = "以及回复进行中输入时用的排队带（[指南](docs/guide/tui.md)） |"
assert zold in z
z = z.replace(zold, znew, 1)
q.write_text(z)
print("auto tier named in Why; bilingual claim dropped from the TUI row")
