# 设置

从窗口左下角的**齿轮图标**进入设置。左栏里还有一个 **Models** 快捷入口,可直接跳到 Models 页。

日常最常用的是 **General**、**Providers**、**Models** 三个页面。此外还有**检查更新(Check for updates)**和**重置(Reset)**页面。

---

## General(通用)

应用级别的桌面选项:

- **Language(语言)** —— 选择应用的显示语言。
- **Approval mode(批准模式)** —— agent 在操作前询问的程度:
  - **Manual(手动)** —— 读写文件前询问;只读命令自动运行。
  - **Sandboxed(沙箱,仅 macOS)** —— 命令在 macOS 沙箱内运行;文件操作仍会询问。
  - **Unrestricted(不受限)** —— 不询问、不沙箱,一切照跑。
- **Show thinking process(显示思考过程)** —— 在对话里显示或隐藏模型的推理过程。

批准机制的实际用法见 [[使用 FutureOS|Using-FutureOS]]。

---

## Providers(供应商)

provider 就是你的模型来源。

### FutureGene(内置)

FutureGene 是内置 provider。使用步骤:

1. 点 **Connect**。
2. 在浏览器里授权。若没自动打开,使用应用里显示的**验证码**和**可复制链接**。
3. 连接后,你随时可以**重新登录(Sign in again)**或**登出(Sign out)**。

列表里还有其他内置 provider(如 DeepSeek、OpenAI、Anthropic、Google 等)——点 **Set key** / **Update key** 可为它们填入你自己的 API key。用 **More providers** 可展开完整列表。

### 自定义 provider

你可以添加自己的 provider。点 **+ Add custom provider**,填写:

- **Name(名称)** —— 显示名(可选)。
- **Provider ID** —— 唯一 id(小写字母、数字、`-`、`_`)。
- **API type(API 类型)** —— OpenAI Completions、OpenAI Responses 或 Anthropic。
- **Base URL** —— provider 的 API 地址(`http`/`https`)。
- **API Key**。
- **Models(模型)** —— 一个或多个模型 ID(可带显示名)。

应用会校验字段,并检查 provider ID 是否唯一。之后可以**编辑(Edit)**或**删除(Remove)**自定义 provider。

> provider 的 API key 与其他凭证分开保存。

---

## Models(模型)

Models 页按 **provider 分组**列出所有可用模型:

- **搜索**可按模型名或 provider 名过滤。
- **切换每个模型的可见性** —— 隐藏的模型会从模型选择器里移除,让你的列表只保留常用的。

输入框里的模型选择器与这里同源,并会显示每个模型来自哪个 provider。

---

## 检查更新(Check for updates)

检查是否有新版本的 FutureOS,并下载对应系统的安装包。如何应用更新见 [[安装 FutureOS|Installation]]。

---

## 重置(Reset)

**清除本地数据(Clear local data)**会抹掉 FutureOS 的本地数据并重启应用。仅在你想彻底重来时使用——会话和本地设置都会被移除。

---

## 另见

- [[快速开始|Quick-Start]] —— 连接 FutureGene 并发出第一条消息。
- [[使用 FutureOS|Using-FutureOS]] —— 批准机制详解。
- [[技能|Skills]] —— agent 可使用的能力包。
