# CLI 的域名解析（静态 musl 构建）

当系统的 DNS 配置不在 C 库查找的位置时，`future` 会自己解析主机名。本文说明
什么时候会这样、读哪些文件、两个覆盖用的设置，以及哪些结论是实测、哪些是推断。

范围：`future` CLI 自己的出网连接——平台 API、中继、技能与 MCP 服务、以及
TCP 形式的 agent 端点。桌面端、TUI 和 `future host` 仍然走它们的 C 库解析，
这里没有改动它们。

## 1. 静态 musl 构建绕不过的三件事

Linux 的 CLI 发布包静态链接 musl。有三个后果是配置无法消除的：

| 限制 | 含义 |
|---|---|
| 没有 NSS | musl 没有 `nsswitch.conf`，也没有可加载的名字服务模块。mDNS/Avahi 名字、LDAP 目录名字、systemd-resolved 的按链路/分域视图，一律不可用。 |
| 路径写死 | musl 的解析器打开 `/etc/hosts`（`src/network/lookup_name.c`）和 `/etc/resolv.conf`（`src/network/resolvconf.c`）。没有 `RESOLV_CONF` 之类的环境变量，也没有 `$PREFIX` 相对路径的回退。 |
| 没有 `dlopen` | 静态二进制不能在运行期加载插件，所以"装一个 NSS 模块"对静态发布不是解法——这也正是下文这个解析器存在的原因。 |

前两条解释了为什么一台机器的 DNS 完全正常，`future` 的每次解析却都失败：
nameserver 列表可能在 musl 根本不会打开的文件里。

在一个完全没有 `/etc` 的 `chroot` 里对 musl 静态二进制实测（`aarch64`，
musl 1.2.x）：

| `/etc/hosts` | `/etc/resolv.conf` | `getaddrinfo("example.com")` | `getaddrinfo("localhost")` |
|---|---|---|---|
| 缺失 | 缺失 | **5.0 秒**后失败 —— `EAI_AGAIN`，"Try again" | **5.0 秒**后失败 —— `EAI_AGAIN` |
| 存在 | 缺失 | **5.0 秒**后失败 —— `EAI_AGAIN` | 由 hosts 文件解析（微秒级） |
| 缺失 | 存在 | 解析成功（用文件里的 nameserver） | 立即失败 —— `EAI_NONAME`，"Name does not resolve" |
| 存在 | 存在 | 解析成功 | 解析成功（微秒级） |

两种失败并不相同，值得分开看：缺 `resolv.conf` 会让每次 DNS 查询先卡 **5 秒**
再返回 `EAI_AGAIN`；缺 `hosts` 只影响写在里面的名字（首当其冲是
`localhost`），而且立即失败。那 5 秒是 musl 自己的超时，不是网络问题——这就是
它在 issue 里看起来像"网络挂了"的原因。

## 2. 什么时候 CLI 自己解析

每个进程只判断一次，依据是环境变量和文件系统：

| 情况 | 方案 | 行为 |
|---|---|---|
| `FUTURE_DNS_SERVER` 已设置且非空 | 自定义 | 该地址是**唯一** nameserver；不读任何 `resolv.conf` |
| `FUTURE_RESOLV_CONF` 已设置且非空 | 自定义 | 读该文件，而不是候选路径 |
| `/etc/resolv.conf` 存在（且两个变量都没设） | **系统** | 什么都不装：每次解析都是 `getaddrinfo`，与此前完全相同 |
| 没有 `/etc/resolv.conf`，但下面某个候选存在 | 自定义 | 读第一个存在的候选 |
| 都不存在 | 自定义 | 解析失败，报错里写明查过哪些文件 |

第三行是约定：在普通 Linux（以及存在同名文件的 macOS）上，这个功能是不生效的。
§6 说明它如何被测试钉住。

## 3. 候选路径

`resolv.conf`，取第一个存在的：

1. `/etc/resolv.conf`
2. `/run/systemd/resolve/resolv.conf` —— systemd-resolved 自己的文件，通常就是
   `/etc/resolv.conf` 指向的目标
3. `/run/resolvconf/resolv.conf` —— openresolv
4. `$PREFIX/etc/resolv.conf` —— Termux（`PREFIX` 已设置时）

hosts 文件，所有存在的都会被读：

1. `/etc/hosts`
2. `$PREFIX/etc/hosts` —— Termux

hosts 文件缺失从不视为错误：libc 也是这个行为。`resolv.conf` 只用第一个存在的
文件，绝不合并多份——一台机器若配了两套视图，它的管理员既然做了这个选择，
就不该被第二个文件悄悄覆盖。

## 4. 两个设置

| 变量 | 取值 | 作用 |
|---|---|---|
| `FUTURE_DNS_SERVER` | `1.1.1.1`、`1.1.1.1:5353`、`::1`、`[::1]:53` | 唯一的 nameserver。不读 `resolv.conf`，所以在完全没有 `/etc` 的机器上也能用。hosts 文件仍会先被查（libc 也是如此）。取值不是 IP 地址时按配置错误报出，而不是被忽略。 |
| `FUTURE_RESOLV_CONF` | 一个路径 | 读该文件，替代候选路径，即使 `/etc/resolv.conf` 存在也一样。路径不存在时按配置错误报出——静默回退到系统会把错误藏起来。 |

无论读哪个文件，`options` 和搜索域都会生效（`search`/`domain`、`ndots`、
`timeout`、`attempts`，取值裁剪与 libc 一致）。`sortlist`、`rotate` 和 EDNS 相关
选项被忽略：这个解析器每个名字只发一次 A 和一次 AAAA 查询，没有可轮转的东西。

## 5. 实现了什么、没实现什么

已实现：UDP 上的 A 与 AAAA、事务 ID 校验、NXDOMAIN、`rcode`、跳过名字压缩、
搜索域与 `ndots`、每次尝试的超时与重试次数、带别名且大小写不敏感的 hosts 查询、
以及去重后的结果。

有意不实现：

- **没有 TCP 回退。** 回复被截断时直接报告，不重试 TCP。单条 A/AAAA 应答不可能
  填满 512 字节，所以这是理论缺口，不是实际缺口。
- **不缓存。** 一次解析就是一次查询，与 `getaddrinfo` 时相同。
- **没有 DNSSEC、EDNS、DoH/DoT。** 无可配置项；需要的话，请把它放在 nameserver
  地址所指向的那个解析器上。
- **没有 NSS。** 见 §1——mDNS、LDAP、按链路视图仍然不可用。

## 6. "默认行为没变"是怎么钉住的

`Plan::detect` 是对环境变量和文件存在性的纯函数，所以这个判断不需要碰真实的
`/etc`，也不会和其他测试通过 `std::env::set_var` 抢环境。两个测试守住这个约定：

- `dns::config::tests::an_ordinary_linux_keeps_the_system_resolver`：有
  `/etc/resolv.conf` 的机器必须得出 `Plan::System`。
- `dns::tests::the_system_plan_attaches_no_resolver`：系统方案不挂 `reqwest`
  解析器、也不挂 gRPC 解析器；自定义方案两者都挂。

两个都曾被证伪一次后才合入：把 `/etc/resolv.conf` 那一支短路掉（让所有机器都走
自定义），第一个失败；把自定义分支改成返回 `None`，第二个失败。gRPC 侧由
`future_rpc::transport::tests::a_supplied_resolver_decides_where_a_tcp_endpoint_connects`
双向断言——注入的解析器对一个哪都不存在的名字仍然能连上，而同一个端点在没注入时
仍走 `getaddrinfo` 并失败。

## 7. 各类环境的对照

| 环境 | 还需要什么 |
|---|---|
| 普通 Linux / macOS | 什么都不用。两个变量都不要设。 |
| 没有 `/etc` 的容器 | `FUTURE_DNS_SERVER`，或者挂一份 `resolv.conf` 并用 `FUTURE_RESOLV_CONF` 指过去。能控制容器时，bind-mount 宿主机的 `/etc/resolv.conf` 是最小的修法。 |
| 精简 rootfs / initramfs | 同上。若机器的 DNS 由别处管理（比如 `127.0.0.1:53` 上的本地 stub），把 `FUTURE_DNS_SERVER` 指向它。 |
| Termux | `/etc/resolv.conf` 不存在时会自动用 `$PREFIX/etc/resolv.conf`。`localhost` 仍需 `$PREFIX/etc/hosts`；如果 Android 两者都没提供，设 `FUTURE_DNS_SERVER`。 |
| 企业分域 DNS / mDNS / LDAP 名字 | **没有解决。** 这需要 NSS，静态 musl 构建加载不了。请用动态链接的构建，或让一个能作为普通 nameserver 访问的 stub 来解析这些名字。 |

## 8. 失败时用户看到什么

```
cannot resolve "api.example.com": no resolv.conf is readable, so this process
has no nameserver. Resolution in this process: no resolv.conf found; checked
/etc/resolv.conf, /run/systemd/resolve/resolv.conf,
/run/resolvconf/resolv.conf; hosts: /etc/hosts (missing). Set
FUTURE_DNS_SERVER=1.1.1.1 (or 1.1.1.1:53), set
FUTURE_RESOLV_CONF=/path/to/resolv.conf, add the name to one of the hosts files
above, or use an IP address instead of a host name. A static musl build reads
only these files and cannot use NSS modules (mDNS, LDAP, systemd-resolved).
```

请求级的失败也带这段文字：CLI 会把 `reqwest` 错误连同它的成因链一起渲染
（`crate::dns::describe`），因为 `reqwest::Error` 自己的 `Display` 只打印
"error sending request for url (…)"，把解析器的解释藏在下一层。

gRPC 路径会在连接**之前**先解析一次 TCP 端点（`dns::preflight`），理由相同：
`tonic` 把任何连接器错误都渲染成 `transport error`，于是"没有 nameserver"和
"端口拒绝连接"此前无法区分。连接器在真正连接时仍会自己解析一次；预检只决定
这次失败值不值得解释。
