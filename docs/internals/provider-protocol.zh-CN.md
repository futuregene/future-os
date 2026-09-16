# Provider 协议专项测试

> （[English](provider-protocol.md)）面向 FutureOS 的手工 OpenAI Responses
> 回归套件。它是一个刻意独立的 Cargo workspace：不是 Agent 的单元／集成测试目标，
> 也不被 FutureOS CI 引用。

夹具来源是 Git 子模块。GitHub 上每个都显示为固定在某个 commit 的链接仓库；runner
原地读取它们的录制品，不会复制或 vendor 进 FutureOS。

## Runner

在本目录下运行：

```bash
cargo run --bin rig-cassette
cargo run --bin rig-structure
cargo run --bin rust-genai-yakbak
cargo run --bin anthropic-protocol
```

- `rig-cassette` 把真实的 Rig Responses 录制品通过 FutureOS 传输层与适配器重放。
  它对比外发 JSON 请求体与 Rig 记录的请求；唯一归一化是：Rig 省略了 false 字段的
  地方补上 FutureOS 显式的 `store: false`。
- `rig-structure` 是基于 Rig 身份回归的无密钥结构场景。它证明：无 id 的 reasoning
  条目保持流内局部，而迟到的真实 `rs_*` 身份会被持久化并重放。
- `rust-genai-yakbak` 重放链接中的 `completed.output` 为空、终态缺 `output`、
  UTF-8 HTTP 分块边界三组录制品，穿过 FutureOS。
- `anthropic-protocol` 重放 rust-genai 的缓存用量录制品，并检验 FutureOS 对自适应
  摘要思考、签名与脱敏思考顺序、工具结果优先排序、思考 token 的请求边界。

## 夹具子模块

克隆 FutureOS 后初始化：

```bash
git submodule update --init --recursive
```

固定的夹具来源：

- `fixtures/rig` — `0xPlaygrounds/rig`
- `fixtures/rust-genai` — `jeremychone/rust-genai`

要刻意把它们移到更新的上游 commit：更新子模块 checkout、运行相关 runner、提交变更
后的 gitlink。这样夹具更新可审查、可复现。
