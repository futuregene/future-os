# 用 FutureOS 打开文件

这是系统 **Open In / 用其他应用打开** 入口，不是已有的分享入口。
原生配置改动必须重新 prebuild、打包安装；只更新 JS 不会增加系统列表中的应用。

## 行为与边界

- Android：注册 `ACTION_VIEW`，接收系统文件提供器授予读取权限的 `content://` URI。
  支持 PDF、Word（doc/docx）、PowerPoint（ppt/pptx）、Excel（xls/xlsx）、文本
  （含 Markdown/CSV）、图片（含 JPEG/PNG）以及以 octet-stream 发送的文件。
  不接管 HTTP/HTTPS、配对链接或任意 `file://` 私有路径。
- iOS：声明文档 UTI，主 App 的 Expo AppDelegate subscriber 接收文件 URL。
  使用独立的本地收件箱，因此无需启用 Share Extension、App Group 或新增签名权限。
  获取 security-scoped 权限后，在串行后台队列中协调读取并复制；不原地编辑来源文件。
  UIKit 放进本 App `Documents/Inbox` 的临时副本会被清理，外部原文件不删除。
- 接收后沿用对话选择菜单：新普通对话、工作区对话、已有会话；追加到附件草稿。
  未配对时暂不消费，**用户确认发送前不上传、不自动发起模型请求**。
- 冷启动读取待接收内容；热启动/已在前台时通过原生事件触发读取，AppState 前台读取作为补充。
- 沿用附件大小、数量和类型校验。iOS 收件箱单文件 10 MiB、单批 20 MiB、最多 10 个待处理批次，
  7 天过期；Android 沿用 `ShareFileCopier` 限额及缓存清理。超限或读取失败显示提示。
- 系统列表是否显示也取决于发送方提供的 MIME/UTI、只读授权及是否使用系统菜单。
  这不是 Office 原生排版编辑器；文件被带入 FutureOS 对话交给助手处理。

## 自动验证

在 `mobile/` 运行 `npm run check`：包含实际 Expo 插件生成配置断言、原生收件事件、
读取竞态、草稿不自动发送及原有分享回归测试。

在仓库根运行 `python3 scripts/test-mobile-ios-share.py`：用 macOS Swift 执行实际
收件箱复制代码，覆盖中文文件名、文件内容、一次消费、超限、损坏输入和路径安全。

Android 原生（需要 JDK 17、Android SDK，首次会下载 Robolectric 测试运行时）：

```sh
cd mobile
npx expo prebuild --platform android --no-install
cd android
./gradlew :future-share-intent:testDebugUnitTest
```

iOS 使用完整 Xcode 对主 App 构建，并在设备上验证 UIKit 回调；Foundation 测试、
Expo 配置检查和 Swift 语法检查不能替代完整 iOS 编译或真机测试。

## 安装包验收

分别从 Android 文件管理器、iOS 文件 App 和常用来源应用测试：

1. 对 doc/docx、pdf、ppt/pptx、md、jpeg/png 选择“用其他应用打开”，确认列表中存在 FutureOS。
2. App 未启动、后台、已打开三种状态分别接收；中文及带空格的名称保留，附件只出现一次。
3. 新会话/已有会话分别选择，保留已有草稿；取消时不上传，发送后桌面能读到对应内容。
4. 未配对后再配对、超限、授权失效或云盘文件不可读时检查提示；原文件不改变。
5. 原有分享、图库分享、多文件分享及配对链接仍正常。
