# 共享包

> （[English](packages.md)）`packages/` 目录存放被多个 FutureOS 应用或服务消费的
> 可复用包。包名与公开 API 与其实现语言无关。

- `rpc`：Rust 线上契约 crate 与 protobuf 事实源。
- `remote-crypto`：Rust/TypeScript Noise 安全通道；共用握手、记录与拒绝条件素材。
- `remote-client`：Rust 主控连接、凭据 DTO、多远端实例、任务生命周期与文件编排；TS Client 仍在迁移。
- `remote-protocol`：Rust/TypeScript 线上契约、命令 schema/生成类型、握手封装、邀请及 JSON/gzip 编解码。
- `remote-server`：Rust 被控安全端点、宿主契约、文件帧、有序事件/合并、命令接入、wire 策略、NATS 连接、心跳/目录发布及实例监督器/控制权状态；连接启动/重建和完整角色清理仍在迁移。
- `markdown`：共享 TypeScript markdown 解析器与类型。
- `thread-projection`：共享 TypeScript 线程投影逻辑。
- `json-preview`：共享 TypeScript JSON 预览/渲染逻辑。

一个包应有自己的 manifest、公开入口点与测试。包可以依赖其他包，但不得依赖
`desktop`、`mobile`、`agent` 等产品实现。

[Remote 拆包开发计划](remote-package-development.zh-CN.md)定义实施中的
Rust/TypeScript 协议、主控、被控边界，并保留独立 Crypto 和应用宿主适配。
Rust Client 已开始迁移，Server 安全端点已接回；TS Client 尚待实施。
