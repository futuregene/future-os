# FutureOS Mobile

FutureOS Mobile 是桌面端 Remote 功能的原生移动终端。当前只实现 Android；
业务层、主题、国际化和凭证存储均保持跨平台，供后续 iOS 客户端复用。

## 当前能力

- 扫描桌面端一次性二维码配对，不提供绕过扫码的手工入口。
- 使用设备独立 NKey、短期 JWT 和刷新 token 连接远程 NATS。
- 显示桌面在线状态与会话列表，新建或继续桌面对话。
- 流式展示回复、思考与工具执行；支持审批、停止、模型、思考等级和重命名。
- 历史分页加载、按 `(runId, idx)` 去重，并在连接恢复后通过
  `get_events_since` 回补实时事件。
- seed、JWT 和刷新 token 分项存入 Android Keystore 支持的 SecureStore；
  明文不会进入 AsyncStorage、日志或二维码。

## 环境

- Node.js 22.11 或更高版本。
- Android Studio、JDK 17 和 Android SDK / Build Tools 36。
- 一台启用开发者模式与 USB 调试的 Android 设备，或 Android 模拟器。

项目版本由仓库根目录 `scripts/version.mjs` 生成，与桌面端使用同一版本真源。
开发控制面固定为 `https://test.future-os.cn`，生产控制面为
`https://future-os.cn`。客户端会拒绝扫描其他环境签发的配对码。

测试环境目前返回明文 `ws://` NATS 地址，因此 Android 开发构建临时允许
cleartext traffic。生产发布前必须先完成 `wss://` 部署并关闭这个选项。

## 手工运行 Android

```bash
cd mobile
npm install
npm run android:device
```

也可以从仓库根目录执行：

```bash
make run-mobile-android
```

Expo 会在本地生成被 `.gitignore` 忽略的 `mobile/android/`，然后编译并安装
debug APK。本阶段没有 CI、Action、EAS Build 或自动发布。

首次启动后：

1. 在 FutureOS 桌面端登录，打开 Remote，点击“配对并启动”。
2. 手机授予相机权限并扫描桌面端二维码。
3. 配对成功后选择一个桌面会话，或创建新对话。

二维码有效期为 5 分钟且只能使用一次。解除设备配对后必须重新扫码。

## 质量控制

```bash
npm run typecheck
npm run lint
npm run format:check
npm test
npm run check
```

仓库根目录提供对应入口：

```bash
make lint-mobile
make test-mobile
make fmt-mobile
make check-mobile
```

## iOS 预留

`app.config.ts` 已保留 bundle identifier、最低系统版本、相机权限和
SecureStore/Keychain 配置；React Native 业务层不依赖 Android 专属 API。
目前不生成、不维护也不验证 `mobile/ios/`，正式开发 iOS 时再确定签名、推送、
后台恢复和 App Store 隐私清单。
