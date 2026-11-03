# 共享包

> （[English](packages.md)）`packages/` 目录存放被多个 FutureOS 应用或服务消费的
> 可复用包。包名与公开 API 与其实现语言无关。

- `rpc`：Rust 线上契约 crate 与 protobuf 事实源。
- `remote-crypto`：Rust/TypeScript Noise 安全通道；共用握手、记录与拒绝条件素材。
- `remote-client`：Rust 主控连接、凭据 DTO、多远端实例、任务生命周期与文件编排；TS 主控连接、宿主接口、通用同步/投影、文件编排和停止/等待已接回 Mobile，运行与应用验收仍待最终 CI。
- `remote-protocol`：Rust/TypeScript 线上契约、命令 schema/生成类型、握手封装、邀请及 JSON/gzip 编解码。
- `remote-server`：Rust 被控安全端点、宿主契约、文件帧、有序事件/合并、命令接入、wire 策略、NATS 连接、心跳/目录发布及实例监督器/控制权状态；已接回连接启动/恢复、凭据刷新及角色停止/清理等待，运行与互通验收仍待最终 CI。
- `app-settings`：桌面端自己的设置文档（`~/.future/app/app.db`），由 Tauri 后端与 `future desktop` 共用。
- `app-workspaces`：同一数据库里的桌面端工作区记录，由 Tauri 后端与 `future workspace` 共用。
- `config`：`~/.future/` 下手写 JSON 配置（`models.json`、`auth.json`、TUI 的
  `settings.json`）的严格读取与原子写入，由应用宿主与 TUI 共用。
- `http`：每个 Rust 使用方都从这里构造 HTTP 客户端，进程级 TLS provider 因此只安装一次。
- `markdown`：共享 TypeScript markdown 解析器与类型。
- `thread-projection`：共享 TypeScript 线程投影逻辑。
- `json-preview`：共享 TypeScript JSON 预览/渲染逻辑。

一个包应有自己的 manifest、公开入口点与测试。包可以依赖其他包，但不得依赖
`desktop`、`mobile`、`agent` 等产品实现。

[Remote 拆包开发计划](remote-package-development.zh-CN.md)定义实施中的
Rust/TypeScript 协议、主控、被控边界，并保留独立 Crypto 和应用宿主适配。
Rust Client/Server 与 TS Client 已接回应用；独立双语言互通与共享行为驱动已建立，最终运行、性能与人工验收仍待完成。
