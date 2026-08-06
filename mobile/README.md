# FutureOS Mobile

FutureOS Mobile 是桌面端 Remote 功能的原生移动终端。当前支持 Android 和
iOS；业务层、主题、国际化和凭证存储均保持跨平台。

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

## iOS 开发

`app.config.ts` 已配置 bundle identifier（`cn.futureos.mobile`）、最低系统版本
（iOS 16.4）、相机权限和 SecureStore/Keychain；React Native 业务层不依赖
Android 专属 API，Android 与 iOS 共用同一套配对、会话和聊天逻辑。

首次开发 iOS 前需安装 Xcode 及对应 iOS 模拟器运行时。`mobile/ios/` 由
`expo prebuild` 生成，是本地构建产物，已被 `.gitignore` 忽略、不提交。

### 环境

- macOS + Xcode（含 iOS SDK 与模拟器运行时）。
- iOS 16.4 或更高版本的模拟器或真机。

### 手工运行 iOS（模拟器）

```bash
cd mobile
npm install
npm run ios
```

也可以从仓库根目录执行：

```bash
make run-mobile-ios
```

或者使用一键启动脚本（自动创建/启动模拟器、装依赖、prebuild 并运行）：

```bash
scripts/start-mobile-ios.sh          # dev 模式（Metro + debug 构建）
scripts/start-mobile-ios.sh release  # release 模式（独立运行，无需 Metro）
```

### 手工运行 iOS（真机）

将 iPhone 通过 USB 连接 Mac，在 Xcode 中选中开发团队后：

```bash
cd mobile
npm run ios:device
```

免费 Apple ID 即可真机调试；提交 App Store 需要付费 Apple Developer 账号。

### EAS 云构建与分发

`eas.json` 定义了 development / preview / production 三个 profile。首次使用前：

```bash
npx eas login
npx eas build:configure   # 生成 eas.json 并关联 EAS 项目
```

Development build（本地开发 + 调试）：

```bash
npx eas build --platform ios --profile development
```

Preview / TestFlight 分发：

```bash
npx eas build --platform ios --profile preview
```

正式发布使用 production profile 并通过 `eas submit` 上传 App Store。

### iOS 平台注意

- Bundle identifier 不能含下划线，Android 的 `cn.future_os.mobile` 在 iOS 上
  不合法；本项目统一为 `cn.futureos.mobile`。
- 测试环境返回明文 `ws://` NATS 地址，生产发布前必须完成 `wss://` 部署，
  届时在 `app.config.ts` 关闭 `usesCleartextTraffic` 或配置 ATS 例外。
