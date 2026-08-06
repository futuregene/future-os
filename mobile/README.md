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

NATS 一律走 `wss://`（TLS）连接：测试与生产环境均不下发明文 `ws://`
地址，客户端在收到非 `wss://` 地址时会拒绝配对。Android 不再允许
cleartext traffic。

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
debug APK。

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

### GitHub Action TestFlight 分发

主分发路径用 GitHub Action（`.github/workflows/build-ios-testflight.yml`），
手动触发，构建签名 `.ipa` 并上传 TestFlight：

1. 在 GitHub 仓库 Settings → Secrets 配置：
   - `IOS_DIST_CERT_P12_BASE64` / `IOS_DIST_CERT_P12_PWD` — iOS Distribution
     证书（.p12，Apple Developer 后台生成，base64 编码）
   - `IOS_PROVISIONING_PROFILE_BASE64` — App Store provisioning profile
   - 复用现有 `APPLE_API_KEY` / `APPLE_API_KEY_ID` / `APPLE_API_ISSUER`（App
     Store Connect API Key，与 macOS 公证共用；角色需为 App Manager 或以上）
   - 复用现有 `OSS_*` secrets（上传 IPA 到 `dl.future-os.cn`）
2. Actions → Build iOS TestFlight → Run workflow。
3. 构建用 `0.0.<提交数>` 作为 TestFlight 版本号（纯数字，TestFlight 要求），
   提交数同时作为 CFBundleVersion。
4. 上传成功后，登录 App Store Connect → TestFlight → 添加外部测试者。
   首次外部测试需苹果 Beta 审核（约 1-2 天）。
5. 测试者手机装 TestFlight App → 接受邀请 → 安装 FutureOS。

> 版本号机制：测试包（dev）版本为 `0.0.<提交数>[-<hash>]`，第一位 `0` 即
> 测试包；正式版从 `1.0.0` 起，打 `vX.Y.Z` tag 触发 release。iOS 测试包
> 去掉 `-<hash>` 后缀以通过 TestFlight 校验，但版本号仍是 `0.0.x`，判定规则
> 不变。

### iOS 平台注意

- Bundle identifier 不能含下划线，Android 的 `cn.future_os.mobile` 在 iOS 上
  不合法；本项目统一为 `cn.futureos.mobile`。
- NATS 一律走 `wss://`，不配置任何明文/cleartext 例外；iOS 依赖系统 ATS
  默认放行 TLS WebSocket，Android 保持 cleartext 禁止。
