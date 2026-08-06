# FutureOS 文档

> `docs/` 目录的导航索引。链接在当前仓库内均可解析；面向用户的 wiki 见
> [docs/wiki/](wiki/en/Home.md)（英文）与 [docs/wiki/zh/Home.md](wiki/zh/Home.md)（中文）。

## 顶层指南

| 文档 | 内容 |
|---|---|
| [构建与安装](build-and-install.zh-CN.md)（[English](build-and-install.md)） | 前置要求、各平台工具链（macOS / Linux / Windows）、`make` 目标、GUI 打包、`future-loop` 安装、技能安装 |
| [Loop 控制面](loop-control-plane.zh-CN.md)（[English](loop-control-plane.md)） | `future-loop`——目标/todos/门禁/监控、should-run 内核、额度、事件溯源、扩展与多 agent、交接 |
| [TUI](tui.zh-CN.md)（[English](tui.md)） | 终端界面（`future-tui`）：斜杠命令、键盘快捷键、设置 |
| [目录布局](directory-layout.zh-CN.md)（[English](directory-layout.md)） | `~/.future/` 下各目录的职责（agent、channels、TUI、GUI、loop） |
| [渠道配置](channels-config.zh-CN.md)（[English](channels-config.md)） | `~/.future/channels/config.json` 统一参考（agent / 飞书 / 钉钉 各块与默认值） |

仓库根 [README](../README.zh-CN.md)（[English](../README.md)）是入口；
[wiki](wiki/zh/Home.md) 是面向用户的 App 使用指南。

## Wiki（面向用户的 App 指南）

- 中文：[首页](wiki/zh/Home.md)、[安装](wiki/zh/Installation.md)、
  [快速开始](wiki/zh/Quick-Start.md)、[使用 FutureOS](wiki/zh/Using-FutureOS.md)、
  [设置](wiki/zh/Settings.md)、[技能](wiki/zh/Skills.md)、
  [命令行工具](wiki/zh/CLI.md)、[FAQ](wiki/zh/FAQ.md)、
  [飞书](wiki/zh/Feishu.md)、[钉钉](wiki/zh/DingTalk.md)、
  [模型目录](wiki/zh/Models.md) *（自动生成，勿手改）*
- English：[Home](wiki/en/Home.md), [Installation](wiki/en/Installation.md),
  [Quick Start](wiki/en/Quick-Start.md), [Using FutureOS](wiki/en/Using-FutureOS.md),
  [Settings](wiki/en/Settings.md), [Skills](wiki/en/Skills.md),
  [CLI](wiki/en/CLI.md), [FAQ](wiki/en/FAQ.md),
  [Feishu](wiki/en/Feishu.md), [DingTalk](wiki/en/DingTalk.md),
  [Models](wiki/en/Models.md) *(auto-generated)*

## 发布包内附说明（`docs/dist/`）

这些文件会被逐字复制进发布包作为 `Readme.txt`（macOS / Windows / Linux 便携版）。
它们是**活文档**——修改时必须与打包流水线同步。

- [readme-macos.txt](dist/readme-macos.txt) / [en](dist/readme-macos-en.txt)
- [readme-windows.txt](dist/readme-windows.txt) / [en](dist/readme-windows-en.txt)
- [readme-linux.txt](dist/readme-linux.txt) / [en](dist/readme-linux-en.txt)

## 内部工作文档（非用户文档）

- [wiki-prompt.md](wiki-prompt.md)（[en](wiki-prompt-en.md)）——（重新）生成 wiki
  页面的提示词；定义范围、风格与页面清单。
- [architecture-audit/](architecture-audit/README.md) —— 代码架构审计报告
  （时点快照，2026-08-05）。
- [verification/](verification/errors-outdated-missing.md) —— 文档↔源码核验工作
  笔记（事实清单、错误/过时/缺失清单）。

## 文档如何保持正确

- `docs/wiki/{en,zh}/Models.md` 由 `make generate-models`
  （scripts/generate_models.py）生成——勿手改。
- wiki 页面遵循 [wiki-prompt.md](wiki-prompt.md) 的范围：macOS + Windows App、
  不写 TUI 页、不暴露 gRPC/端口内部、CLI 一律叫 `future`。
- 本目录文档均对照源码核验；核验工作笔记把每条声明追踪到 `file:line`。
