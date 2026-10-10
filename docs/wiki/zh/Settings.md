# 设置

从窗口左下角进入设置：左栏折叠时是底部的**齿轮图标**，展开时在账户菜单里（点击头像和邮箱）。左栏里还有一个 **Models** 快捷入口，可直接跳到 Models 页。

日常最常用的是 **General**、**Providers**、**Models** 三个页面。此外还有 **手机遥控（Phone Control）**、**账户（Account）**、**检查更新（Check for updates）**、**关于（About）**和**重置（Reset）**页面；测试版构建还多一个 **Environment（环境）** 开关。设置对话框把它们分为 Desktop / Server / Debug 三组。

---

## General(通用)

应用级别的桌面选项:

- **Language(语言)** —— 选择应用的显示语言。
- **Approval mode(批准模式)** —— agent 在操作前询问的程度:
  - **Manual(手动)** —— 文件访问遵循 Allow/Ask/Deny 路径规则；shell 除只读白名单外先询问。
  - **Sandboxed(沙箱)** —— 可用时使用 macOS Seatbelt、Linux 系统 Bubblewrap 或 Windows 受限令牌写保护；文件访问仍遵循路径规则，各平台保护范围不同。
  - **Unrestricted(不受限，默认)** —— 不询问、不沙箱，一切照跑。
- **Auto-upgrade skills(技能自动升级)** —— 每次应用打开时,静默把已安装技能升级到最新版本。
- **Skill recommendations（技能推荐）**（默认开启）—— 发送消息时，最多推荐一个你尚未安装的技能。
- **Generate a title after the first answer（首轮回答后生成标题）**（默认开启）—— 新会话首次成功回答后自动命名；之后的回答不会再次触发。
- **Completion bell（完成提示音）** —— agent 跑完时播放提示音并提醒窗口。

思考过程在对话中默认折叠，始终可以点击展开，无需先到设置中开启。

默认值、Linux 安装、诊断与限制见 [[审批与沙箱|Sandbox]]；批准卡片用法见 [[使用 FutureOS|Using-FutureOS]]。

---

## Providers(供应商)

provider 就是你的模型来源。

### Future（内置）

**Future** 是内置 provider（即你的 FutureOS 账号）。使用步骤:

1. 点 **Sign in**——会打开欢迎/登录界面。
2. 在打开的浏览器页面里授权。当前版本会自动打开，不再单独显示验证码。
3. 连接后,你随时可以**登出(Sign out)**。

列表里还有其他内置 provider(如 DeepSeek、OpenAI、Anthropic、Google 等)——点 **Configure** 可为它们填入或更新自己的 API key。用 **More providers** 可展开完整列表。

### 自定义 provider

你可以添加自己的 provider。点 **+ Add custom provider**,填写:

- **Name(名称)** —— 显示名(可选)。
- **Provider ID** —— 唯一 id(小写字母、数字、`-`、`_`)。
- **API type(API 类型)** —— OpenAI Completions、OpenAI Responses 或 Anthropic。
- **Base URL** —— provider 的 API 地址(`http`/`https`)。
- **API Key**。
- **Models(模型)** —— 一个或多个模型 ID；每个模型还要填写显示名、必需的上下文窗口与最大输出 token，并可选填思考能力、模态和每百万 token 价格。

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

## 账户(Account)

显示已登录的 FutureOS 账号信息（资料与余额），并可登出。

---

## 关于(About)

显示应用版本、平台与相关发布信息。

---

## 重置(Reset)

**清除本地数据(Clear local data)**会抹掉 FutureOS 的本地数据并重启应用：会话、后台程序和审查记录会被移除，登录状态和 provider 设置保留。Windows 上这里还可以重置 FutureOS 为写保护添加的目录权限（不会删除文件）。

---

## 另见

- [[快速开始|Quick-Start]] —— 连接你的 FutureOS 账号并发出第一条消息。
- [[使用 FutureOS|Using-FutureOS]] —— 批准机制详解。
- [[技能|Skills]] —— agent 可使用的能力包。
