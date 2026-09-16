# 共享包

> （[English](packages.md)）`packages/` 目录存放被多个 FutureOS 应用或服务消费的
> 可复用包。包名与公开 API 与其实现语言无关。

- `rpc`：Rust 线上契约 crate 与 protobuf 事实源。
- `remote-crypto`：Rust Noise 协议端到端加密，桌面/移动远程通道共用。
- `markdown`：共享 TypeScript markdown 解析器与类型。
- `thread-projection`：共享 TypeScript 线程投影逻辑。
- `json-preview`：共享 TypeScript JSON 预览/渲染逻辑。

一个包应有自己的 manifest、公开入口点与测试。包可以依赖其他包，但不得依赖
`desktop`、`mobile`、`agent` 等产品实现。
