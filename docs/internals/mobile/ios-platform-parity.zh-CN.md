# iOS 平台能力与 Share Extension

## 本次补齐

- **其他 App → FutureOS**：原生 `ShareViewController` 支持文字、网页链接、图片和文件。
  用户在系统分享面板选 FutureOS，添加说明并保存；再打开 FutureOS，配对后新建普通/工作区
  会话或选择已有会话，内容追加到目标草稿，确认发送前不上传。扩展不访问远程凭证、不联网，不使用
  responder-chain / `UIApplication` 强行打开主 App 的非标准做法。
- **存储到文件**：`UIDocumentPickerViewController(forExporting:asCopy:)`，用户选择位置；
  不再借用分享菜单完成保存。
- **用其他应用打开**：`UIDocumentInteractionController` 的 Open In 菜单，包含 iPad
  锚点；没有阅读器只影响打开，不影响保存和分享。
- **分享出去**继续使用 `UIActivityViewController`。旧 iOS 原生包没有新的文件模块时，
  打开和保存保留原分享面板回退；更新 JS 不能凭空添加 Share Extension，必须重新构建。
- 删除已无生产调用方的 `future-native-ui` Android 模块、npm 依赖和无效 mock。

## 收件箱边界

`future-share-intent/ios/ShareInbox.swift` 同时编译到扩展和主 App 的 Pod。
扩展只写独立临时批次，全部复制完成后原子发布；主 App 只消费已提交批次。

- 单文件 ≤10 MiB、总文件 ≤20 MiB、最多 10 个附件、文本 ≤256 KiB；图片数量等更细的
  限制沿用主 App 的附件校验。超限文件不保留半截副本，其余可用内容保留并提示。
- 文件逐块复制，不把整张原图解码进扩展内存；保留显示名称，物理路径使用随机名。
- 文件 URL 在 provider 回调有效期内复制，取得的 security-scoped 授权用完即释放。
- App Group 内最多 10 个待处理/在途批次，跨进程锁保护容量与消费；待处理内容 7 天过期，
  中断的暂存批次 1 天后回收，主 App 已取走但未使用的缓存 7 天后回收。
- 一次前台读取一个批次，多次分享会排队；再次回到前台继续读取。切换桌面不会自动发送
  到其他桌面。文件正文、凭证不会放入分享链接或日志。
- 解析清单校验路径、文件大小和符号链接；损坏批次不会阻塞后续批次，缓存写入失败不消费输入。

## Apple 配置与签名

插件 `plugins/withIosShareExtension.js` 在设置 `FUTURE_IOS_SHARE_EXTENSION=1` 后，于 Expo
prebuild 时生成并嵌入扩展，避免提交 `mobile/ios/`。扩展版本与主 App 同步，支持
 iPhone / iPad、最低 iOS 16.4。默认不启用扩展，以保持现有 host-only 签名流程不变。

需要在同一 Apple Developer 团队下配置：

| 项目 | 标识 |
| --- | --- |
| 主 App ID | `cn.futureos.mobile` |
| Share Extension ID | `cn.futureos.mobile.share` |
| 两个 App ID 共用的 App Group | `group.cn.futureos.mobile.share` |

1. 注册扩展 App ID 与 App Group，为两个 App ID 都开启并关联这个 App Group。
2. 重新生成主 App 的 profile，并创建扩展的独立 profile。
3. 本地在 `mobile/` 运行 `FUTURE_IOS_SHARE_EXTENSION=1 npx expo prebuild --platform ios`，
   在 Xcode 为两个 target 选择同一团队，并选择相应开发/分发 profile。
4. 如需手动签名，在 prebuild 时同时设置 `FUTURE_IOS_MANUAL_SIGNING=1`；插件给两个
   target 分别引用 `$(FUTURE_APP_PROFILE)` 与 `$(FUTURE_SHARE_PROFILE)`。调用 xcodebuild 时
   传入这两个自定义设置，不能全局传一个 `PROVISIONING_PROFILE_SPECIFIER` 覆盖所有 target。
   导出 IPA 的 `provisioningProfiles` 字典须包含两个 bundle ID 对应的 profile。
5. 可先用 `security cms -D -i <profile>` 解码两份 profile，再运行
   `python3 scripts/tests/validate-ios-share-profiles.py <host.plist> <share.plist>` 检查 App ID、团队、
   有效期、分发类型与 App Group。

**本次不修改 CI/发版工作流。** 现有工作流未设置 opt-in，继续生成不含扩展/App Group 的版本，
文件保存/打开仍可用。仅添加 secret 不会自动启用扩展；后续分发扩展需要另行配置签名流程。
这不是仅合并代码就能完成的 Apple 后台配置，不能把“代码已支持”当作已在 TestFlight 上线。

## 验证

- `npm run check`（在 `mobile/`）：类型、lint、Jest；覆盖独立文件操作、旧包回退、等待
  下载弹窗真正关闭后再呈现 UIKit，以及取消/失败路径。
- `python3 scripts/tests/test-mobile-ios-share.py`：实际 Swift 收件箱的原子消费、失败重试、字节
  限额、Unicode、坏清单、路径逃逸、目录/符号链接拒绝与队列容量。
- 显式启用扩展并 prebuild 后运行 `node scripts/tests/test-mobile-ios-project.cjs`：验证生成项目的
  源文件、嵌入关系、entitlements、重复运行、版本同步和每个 target 的签名设置。
- 有 Xcode/iOS SDK 的 macOS 上，可在 `mobile/ios/` 安装 Pods 后运行：
  `xcodebuild -workspace FutureOS.xcworkspace -scheme FutureOS -configuration Debug -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build`。
  这会编译 App、两个原生模块及已启用的扩展，不需要 Apple 密钥。本次本机只有 Command
  Line Tools，未执行 UIKit/App 的完整编译；现有 CI 不包含此原生编译检查。

真机验收仍需执行：Safari 链接、选中文字、照片单/多张、文件 App 的 PDF/中文文件名；
未配对时保存后再配对；连续分享、取消、超限、同名文件、冷启动/前后台；iCloud 文件下载；
iPhone/iPad 保存位置选择、取消、外部阅读器有/无、系统分享与远程连接恢复。

## 保留的系统差异

iOS 更新走 App Store / TestFlight，不下载 APK。Android Activity 重建恢复、通知 channel、
系统返回键不是 iOS 功能缺口。任务通知两端都只是本地通知，长期后台/离线推送两端均未实现。
