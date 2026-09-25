# 鸿蒙手机：APK 兼容运行与原生能力边界

核查日期：2026-09-14。问题环境：Mate XT、HarmonyOS 6.1.0，通过卓易通运行 APK。
本页区分源码检查、官方文档和待真机验证事项，不把 Android 系统界面视为鸿蒙原生界面。

## 结论

当前 `mobile/` 是 Expo / React Native 的 Android、iOS 客户端，没有鸿蒙原生构建目标。
卓易通中的 APK 使用 Android API；实际选择器以及可访问的应用、文件，由兼容环境及其
桥接能力决定。不能在 APK 的 TypeScript 中直接导入鸿蒙 `@kit.*`，也不能仅修改样式
就声称使用了鸿蒙原生相机、图库或文件选择器。

本次未从可访问的卓易通官方资料中找到可验证的公开桥接 SDK，让第三方 APK 直接调用
下表鸿蒙接口。官网首页抓取主要是图片，开放平台返回登录页；这不证明没有合作接口，
只说明本次无法核实。没有使用猜测的包名、私有 Intent 或 URI scheme。

要保证这五项由鸿蒙系统原生提供，需要新增鸿蒙原生构建目标并接入对应 Kit，或者取得
卓易通官方支持的桥接方案后再实现、验证。本次 APK 改动不是鸿蒙原生移植。

## 官方原生接口

| 功能 | 接入方式 | 文档确认的边界 |
| --- | --- | --- |
| 拍照 | Camera Kit `cameraPicker.pick` | 系统提供拍摄、确认界面；此方式无需申请相机权限。 |
| 相册 | Media Library Kit `photoAccessHelper.PhotoViewPicker.select` | 拉起系统图库，由用户选择；接口无需全相册权限，返回 URI 是只读授权。 |
| 手机文件 | Core File Kit `picker.DocumentViewPicker.select` | 拉起系统文件选择界面，需要 UIAbility 上下文；按 URI 授权读文件，不猜物理路径。 |
| 分享文件 | Share Kit `systemShare` | 用文件 URI 和准确 UTD 类型构造分享数据，拉起系统分享面板；目标应用需支持接收该类型。 |
| 其他应用打开 | Ability Kit 隐式 Want / `startAbility` | 携带 URI、MIME 类型及读取授权，匹配已注册的处理应用；仅有一个匹配时可能直接打开。 |

系统分享不等于保证微信一定出现。必须验证目标微信版本、文件类型和系统版本；尤其不能
将“卓易通内 Android SEND 已拉起”当作“鸿蒙版微信已收到文件”。

官方来源（华为相关正文已读取；卓易通的访问限制见上文）：

1. [通过系统相机拍照和录像](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/camera-picker)
2. [使用 Picker 选择媒体库资源](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/photoaccesshelper-photoviewpicker)
3. [文件选择器](https://developer.huawei.com/consumer/cn/doc/harmonyos-references/js-apis-file-picker)
4. [systemShare](https://developer.huawei.com/consumer/cn/doc/harmonyos-references/share-system-share)
5. [调用其他已安装应用打开文件](https://developer.huawei.com/consumer/cn/doc/harmonyos-faqs/faqs-ability-54)
6. [卓易通官网](https://www.droitong.com/)
7. [卓易通开放平台](https://developer.droiapps.com/)

## 当前 APK 源码核查及改动

- 拍照：`expo-image-picker` 的 `CameraContract` 使用 Android 相机 Intent。
- 相册：Android 13+ 在系统照片选择器确实响应时用 `expo-image-picker` 的 `ImageLibraryContract`
  （AndroidX `PickVisualMedia` / `PickMultipleVisualMedia`，`legacy: false`）。**API 33 以下且缺少 Play 服务的照片选择器回退时，
  AndroidX 会把这个契约解析成 `ACTION_OPEN_DOCUMENT`**，也就是文件选择器——2026-09-25 在无 Play 服务的
  华为手机上点「相册」弹出的正是文件浏览器。现在在启动任何界面之前先用原生探针
  （`future-file-handler` 的 `resolveImagePickRoutes`）解析候选：有响应 `ACTION_PICK` +
  `content://media/external/images/media` 的图库就显式定向启动它；只响应 `ACTION_GET_CONTENT`
  的图库同样定向启动；只有真正的照片选择器（框架版 `android.provider.action.PICK_IMAGES`、AOSP 回退版
  `androidx.activity.result.contract.action.PICK_IMAGES`、或 Play 服务版 `com.google.android.gms.provider.action.PICK_IMAGES`）
  才走契约。
  **两者都没有时应用自绘相册网格**（`listAlbumImages`：MediaStore 优先，再用 DCIM/Pictures/Download 等常见目录扫描兜底），
  并为此申请媒体读取权限（拒绝时在网格内说明），不再降级到文件选择器。
  各条系统路由都不申请全相册读取权限，只能拿到被选中的照片。
  若某环境的 `ACTION_PICK` 只由文件管理响应（探针会看到 `com.huawei.hidisk` 一类包名，不当图库），
  则走自绘网格；若自绘网格也读不到任何图片（容器未映射媒体库与共享存储），相册才真正不可用。
- 手机文件：`expo-file-system` 的 `FilePickerContract` 使用 `ACTION_OPEN_DOCUMENT`。
- `NativeFileActionSheet` 明确分开“用其他应用打开 / 保存 / 分享”。
- Android 外部打开沿用 `future-file-handler` 的 `ACTION_VIEW`、FileProvider 与只读授权；
  分享也通过 `future-file-handler` 的 `ACTION_SEND`、`EXTRA_STREAM`、`ClipData` 与
  FileProvider 发送真实文件，不是把缓存路径作为文本发送。保存沿用 Storage Access Framework。
  分享在系统接受 chooser 后返回，不等待接收应用的结果，也不保留全局 pending promise。
  这是为避免兼容环境未回传 activity result 时，`expo-sharing` 永久拒绝后续分享；
  返回仅代表系统接管，不代表接收应用已发送成功。iOS 继续使用 `expo-sharing`。
- 不再在进入文件菜单时检查阅读器。只有选择外部打开时才检查 VIEW 能力；缺少 PDF 阅读器
  不影响保存或分享。预览页也有分享和外部打开入口，并获取原文件而非预览截断文本。
- iOS 保存使用 `UIDocumentPickerViewController`，外部打开使用 `UIDocumentInteractionController`，
  分享仍使用 `UIActivityViewController`；旧原生包回退到分享面板，不声称新增内置文档阅读器。
- 维持文件类型白名单和 10 MiB 上限，取消传输后不执行迟到的文件分享。

## 真机验收（未执行）

1. 记录卓易通版本；检查鸿蒙侧授予卓易通的相机、媒体和文件权限，以及 APK 内部权限。
2. 分别打开、完成和取消相机、相册、文件选择；记录实际页面属于鸿蒙系统还是兼容环境。
   能选择文件只证明数据访问可用，不证明界面是鸿蒙原生。相册在卓易通容器里预期走**应用自绘网格**
   （探针看不到图库也看不到照片选择器）；网格为空或申请不到媒体权限时，才是「容器未映射媒体」的边界。
   若相册出现文件浏览器，则说明探针把某个文件管理包当成了未知应用（包名未命中图库/文件管理特征），
   需记录包名补进规则。
3. PDF 没有匹配阅读器时，文件菜单仍提供保存、分享；外部打开提示当前运行环境无处理应用。
4. 分享中文文件名 PDF、图片、文本，确认目标应用收到真实文件、名称和 MIME 正确。
   分别核查卓易通内应用与鸿蒙原生微信是否出现、能否读取；未出现时记录兼容限制。
5. 安装可访问的阅读器后验证“用其他应用打开”；不承诺能枚举卓易通之外的鸿蒙应用。
6. 文件目录进入两层后使用系统返回手势，逐级返回；根目录再返回会话。左上角按钮直接
   回会话。预览或系统面板关闭后，原目录保持不变。
7. streaming 会话带已有内容重开、弱网重连：同步完成前显示同步/重试提示，已有消息不清空。
8. 连续增量、批量补回、代码块和表格更新时检查逐批显现、新块淡入；阅读旧消息不自动滚底，
   系统“减少动态效果”开启时立即显示。检查 Mate XT 折叠/展开、大字号及低帧率情况。

自动化测试覆盖菜单分流、原文件分享、取消/上限、返回层级、同步状态、渐进显现及减少
动态效果逻辑；不能替代兼容环境的跨应用授权、原生界面和动画观感真机测试。
