# AGE-5：正式桌面工程

本文保留 AGE-5 阶段的基础工程验证记录。下文“当前”描述对应当时的脚手架，不代表最终 v0.1.0 功能；最新功能与验收结果见 [版本完成情况检查](v0.1.0-readiness.md)、[工作台](workspace-interface.md) 和 [实施记录](implementation-progress.md)。

正式应用位于仓库根目录，技术实验保留在 `experiments/desktop-spike/`。当前版本号统一为 `0.1.0`，这是开发中的基础工程，尚未发布完整 MVP。

## 开发环境

- macOS 15+、Xcode 命令行工具；本次实际验证在 Apple Silicon / macOS 27.0.1 / Xcode 27.0。
- Node.js ≥22.12；本次使用 26.8.2。应用不内置 Node，后续管理 npm Agent 时仍需检查用户的 Node/npm。
- rustup 与 Rust 1.99.0。`rust-toolchain.toml` 固定版本、rustfmt 和两种 macOS targets。
- 依赖锁定于 `package-lock.json` 与 `src-tauri/Cargo.lock`；Tauri Rust/API 2.12.3，CLI 2.12.2。

从干净检出进入仓库根目录：

```sh
npm ci
rustup show
npm run dev:app
```

`dev:app` 启动本地 Vite 服务与真实 Tauri 窗口。`npm run dev` 仅供浏览器预览布局；浏览器会显示“请在 AgentUpdate 桌面窗口中打开此页面”，不会伪造本机工具或版本。若 1420 端口被占用，先关闭已有的本项目开发服务；不会自动选择另一个端口。

检查和构建：

```sh
npm run check
npm run build:mac
```

`check` 包含 TypeScript、Rust 格式和 Rust 测试。`build:mac` 生成通用包，默认输出为 `src-tauri/target/universal-apple-darwin/release/bundle/macos/AgentUpdate.app`。`npm run build:app` 可只构建当前主机架构。首次构建需要下载依赖；公司或本机代理按自己的环境配置，仓库不保存代理凭据。

此配置默认使用 ad-hoc 签名。本地审核候选使用 `npm run package:mac -- --candidate`。正式 `npm run package:mac` 必须显式设置已安装的 `AGENTUPDATE_SIGNING_IDENTITY`（Developer ID Application）与 Keychain 中可用的 `AGENTUPDATE_NOTARY_PROFILE`；不会自动挑选证书或降级成未公证包。输出位于独立的 `release-artifacts/`，当前发布阻塞见 docs/macos-release.md。

## 当前可操作的界面

应用显示真实 Rust 后端返回的应用版本、运行架构和首批工具支持范围。选择 Codex / Gemini 会调用后端读取对应支持信息；刷新按钮与 ⌘R 重新读取应用状态。采用原生窗口装饰、可调整尺寸、系统菜单、键盘焦点与系统深色外观。

“首批支持工具”是计划支持目录，不是发现结果。当前不扫描 Agent 安装或配置、不联网查询版本、不写入用户数据、不执行更新或恢复。未接入能力始终显示“尚未扫描”“尚未检查”或“正在开发”，不会显示已是最新版本。备份策略表示产品要求，实际强制备份协议由后续任务实现。

## 模块与接口边界

| 位置 | 职责与后续接入 |
| --- | --- |
| `src/api/desktop.ts` | 明确的前端业务调用与响应类型；不暴露通用 invoke 包装给产品组件 |
| `src/components/` | 基础界面组件；完整产品流程在 AGE-11 |
| `src-tauri/src/service.rs` | 汇总应用状态，编排领域服务 |
| `agents/` | Agent 标识和支持目录；AGE-6 接入发现及官方版本查询 |
| `platform/` | 平台能力边界；当前只报告 macOS 与编译架构，其他平台明确拒绝 |
| `updater/` | 更新能力状态；后续按受管实例/计划 ID 接入事务执行 |
| `backup/` | 默认备份要求；AGE-8 接入快照和恢复准备 |
| `storage/` | 持久化状态；AGE-7 接入 SQLite、模型与恢复协议 |
| `releases/` | 固定本仓库的 GitHub Releases 来源；AGE-17 接入应用自身版本检查 |

仅注册两种生产命令：

| 命令 | 参数 | 行为 |
| --- | --- | --- |
| `get_app_overview` | 无 | 返回版本、平台、工具目录和各模块能力状态 |
| `get_agent_support` | `agentId: "codex" \| "gemini"` | 返回该工具支持信息；其他标识由 Rust 反序列化拒绝 |

响应采用 camelCase；领域错误为 `{ code, message }`。未知工具值在进入领域服务前拒绝。后续增加实例、计划或恢复点时，必须再次验证对象作用域；不接受任意命令、参数数组、文件路径或 URL 作为通用执行入口。

`build.rs` 通过 Tauri AppManifest 生成自定义命令权限；`capabilities/main.json` 仅允许主窗口读取上述两项信息，没有 shell、文件系统、远端 WebView 或窗口创建权限。生产 CSP 限制打包资源与 IPC，开发 CSP 只额外允许固定的本地 Vite 地址。没有通过关闭 CSP 来适配开发服务器。

Tauri ACL 是应用接口限制，并非 macOS App Sandbox。当前业务调用不需要文件或进程能力；后续能力按模块逐项增加，不能沿用实验中的通用读写方式操作真实 Agent。

## 可重复的桌面验证

```sh
npm run build:mac
npm run build:validation
npm run validate:desktop
```

第二条构建独立的 `AgentUpdate Validation.app`，显式启用 `desktop-validation` feature 和专用测试 capability。只有这个包注册测试完成/拒绝探针命令；正式 `AgentUpdate.app` 没有这些命令、权限或结果文件写入入口。不要将验证包发布为正式产品。

验证脚本检查正式包和验证包的双架构、版本、最低系统设置和签名，然后分别通过原生 arm64 与 Rosetta x86_64 打开真实 React WebView。它检查领域调用、界面渲染、未知工具拒绝、已注册但未授权命令拒绝，以及通用命令/文件和 shell/fs 插件入口不可用。临时结果写入验证脚本自建目录，随后清理；机器证据存于 `docs/evidence/age-5-desktop.json`。

如果没有可用的 Rosetta，x86_64 运行检查会失败并报告条件不足，不能仅凭双架构构建标记运行通过。本次 x86_64 运行来自 Rosetta，不代表物理 Intel 或 macOS 15 实机验收。

本机复用了 AGE-4 的隔离 Rust 1.99.0 和依赖缓存，通过 `RUSTUP_TOOLCHAIN=stable` 选择该已安装版本，未修改系统 Rust。缓存目标目录通过 `CARGO_TARGET_DIR` 指向实验的 target，正式与验证 `.app` 使用独立产品名，与旧实验包并存。正常开发无需使用这些本机临时路径；如果设置了 `CARGO_TARGET_DIR`，验证脚本也会从该目录查找应用包。

本次还将源码复制到不含 node_modules、dist、Tauri 生成文件和 Rust 输出的独立目录：`npm ci --offline`、前端构建，以及启用测试 feature 的默认 debug 开发启动均通过。开发窗口经 Vite 加载真实前端并完成相同的 IPC/权限检查；只复用已有依赖下载缓存。`npm run check` 的 TypeScript、rustfmt 和两项 Rust 边界测试均通过。正式通用包另行构建并经实际窗口人工检查，约 14.96 MiB。

## 范围限制

当前基础工程不提供生产发现、安装、配置读写、备份、回滚、事务数据库、实例锁、取消或崩溃恢复。应用自身 Release 检查目前只有固定来源边界，没有网络请求、版本比较或下载按钮。Windows/Linux 尚未编译运行；最低 macOS 15 与物理 Intel 的验收留待专用机器验证。

## 后端实施进展（AGE-6 → AGE-9）

上述界面和两条接口说明是 AGE-5 的基础工程验收记录。现已接入受限发现、官方版本、固定计划、SQLite 历史和单项更新接口；生产主窗口权限按业务命令逐项声明，没有通用 shell/文件接口。具体范围见 agent-discovery.md、transaction-storage.md、backup-recovery-material.md、single-tool-update.md。完整操作界面仍由 AGE-11 实现。
