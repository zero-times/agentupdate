# CLI 项目目录启动（AGE-24）

在 CLI 工具页，每个已发现的安装实例都有“打开”按钮，包括可启动但尚不支持更新的来源。同一工具的多个来源分别使用各自的实际命令路径。

1. 点击“打开”，默认填入最近使用的项目目录。
2. 从“最近项目”选择目录，可直接修改路径；“保存目录”替换所选的历史条目。
3. 点击“新增目录”输入另一项目的绝对目录路径，保存后加入最近列表。
4. 点击“在此目录打开 CLI”，应用打开 macOS 系统终端，切换到所选目录并启动对应安装实例。打开时也保存目录。

最近 20 个项目目录跨工具共享，按使用时间排列，去重并保存在本机应用数据目录的 `cli-projects.json`（文件权限 0600）。退出或重启应用后保留。目录须已经存在；应用不会创建项目文件夹，也不会读取项目文件。历史中的目录被移动后，可在弹窗修改路径。

Deepseek Harness 必须指定 profile。打开弹窗默认使用官方内置 `web`（启动网页界面），最终命令为 `dsh --profile web`。可填写其他已经安装的 profile 名称；`tui` 是自定义 profile 的示例，不是内置模板，`desktop` 专属于 Electron 桌面应用。AgentUpdate 不会替你安装 profile 或修改 Harness 配置。

首次启动时，macOS 可能要求允许 AgentUpdate 控制“终端”。拒绝或打开失败时弹窗保留输入并显示重试说明；可在系统设置 → 隐私与安全性 → 自动化中调整。终端打开成功表示启动命令已交给终端；CLI 自身的登录、运行结果和错误显示在终端里。

后端只接受扫描得到的安装实例 ID，检查命令仍指向相同文件且可执行。Node 与命令目录加入启动命令的 PATH，供 npm 安装使用。项目目录和命令按 shell 参数转义，完整命令通过 argv 传给固定 AppleScript，不插入 AppleScript 源码。

验证：Rust 的 `cli_launch` 集成测试覆盖目录保存、编辑、去重、20 项上限、权限、无效路径，以及 `/bin/sh` 和 `/bin/zsh` 下中文、空格、引号和 shell 元字符的实际启动。`scripts/validate-cli-launch.mjs` 使用独立测试应用与隔离 CLI，操作真实 React WebView，再确认系统终端启动的实际工作目录和历史文件。运行脚本前构建 `desktop-validation` feature 的验证包；默认使用 `src-tauri/target/release/bundle/macos/AgentUpdate Validation.app`，也支持 `CARGO_TARGET_DIR` 或 `AGENTUPDATE_LAUNCH_VALIDATION_BINARY`。

验证材料见 `docs/evidence/age-24-cli-launch.json`。该功能当前面向 macOS；物理 Intel 设备验证留待后续。

发布验证状态（v0.2.3）：默认测试集与 Deepseek 默认/自定义 profile 参数执行测试通过，本机 dsh --profile web --help 正常退出。新增完整 profile 界面测试在 Mac 锁屏期间超时，未记为通过；最初的目录选择与 Terminal 启动验收通过记录保留在证据中的 baselineBeforeDeepseekProfileChange。
