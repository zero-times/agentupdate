# AGE-4：桌面技术验证与选型

验证日期：2026-10-12。正式工程选用 **Tauri 2 + React + TypeScript + Rust**。已构建并运行最小 macOS 通用实验包，验证本地进程、文件权限及实际 WebView 的调用限制；正式管理应用尚未搭建，后续由 AGE-5 推进。

## 选型结果

| 方案 | 本项目判断 |
| --- | --- |
| Tauri 2 + React / Rust | 采用。系统 WebView 承载界面，Rust 实现安装/恢复与平台操作；可生成 macOS 双架构包并为后续 Windows/Linux 保留同一业务层 |
| Electron | 暂不采用。内置 Node 对 npm 工具集成方便，但带入 Chromium 和额外运行时；此次已证明 Tauri 能完成关键本地调用，无需为此更换框架 |
| SwiftUI / AppKit | 暂不采用。macOS 集成直接，但后续跨平台界面成本更高；未来个别平台能力仍可通过原生桥接扩展 |

SQLite 仍是事务元数据存储的建议方案，真实数据库与文件恢复协议由 AGE-7 验证。本实验未验证 SQLite，也未实现更新器或自身 Release 检查功能。

## 实际环境与开发基线

| 项目 | 本次实际版本 / 设置 |
| --- | --- |
| 主机 | Apple Silicon，macOS 27.0.1，Xcode 27.0 |
| Node.js | 26.8.2；开发和 npm 适配器预检建议 ≥22.12，最低 Node 版本尚未实测 |
| Rust | 1.99.0，独立工作区 toolchain，含 `aarch64-apple-darwin` 与 `x86_64-apple-darwin` |
| Tauri Rust / JS API | 2.12.3 / 2.12.3；CLI 2.12.2，必须与 Rust/API 保持同一 major/minor |
| React / TypeScript / Vite | 19.3.0 / 6.0.3 / 8.3.4 |
| 应用最低 macOS | 明确设置 15.0，与首批 Gemini 官方当前支持基线一致；未在 macOS 15 实机运行 |
| 实验应用版本 | 0.0.0，仅为技术验证，不发布为 v0.1.0 |

保留 `package-lock.json` 与 `Cargo.lock` 以复现依赖。本机原有 Homebrew Rust 1.98.1 未修改；实验另外将 Rust 1.99.0 安装到被忽略的 `.validation/toolchain/`，未改 shell 启动文件或默认工具链。正式开发可使用 rustup 的 1.99 工具链和两个 macOS targets。

已生成约 15.10 MiB 的 `.app`，主可执行文件包含 `arm64` 和 `x86_64`。系统 WebView 无需把 Node 或 Chromium 打入应用，但管理 npm Agent 时仍需用户机器上的合适 Node/npm，不能因此省略运行时预检。

## 实验路径

[experiments/desktop-spike](../experiments/desktop-spike/) 为独立技术实验，包含 React 窗口、Tauri 权限清单和 Rust 操作：

- `run_validation`：无路径或可执行文件参数；后台线程运行固定的 `/usr/bin/sw_vers -productVersion`，设置超时与进程回收。
- 文件实验：仅在自建临时目录读写人工 fixture，目录权限 0700、文件 0600；同步写入、读取校验后删除。
- `admin_only_probe`：注册为真实命令，但不在主窗口权限列表内，用来验证 ACL 确实拒绝调用。
- `finish_validation`：记录测试布尔结果；证据目标由开发者测试进程的环境指定，前端不能传入文件路径。自动化模式结束应用，普通打开时保留窗口。

没有接入通用 shell、文件系统或窗口创建插件；`run_arbitrary_command` 与 `read_arbitrary_file` 不在业务接口中。实际前端尝试这些调用并确认拒绝，不能仅靠检查配置来宣称权限有效。

### 构建与复验

在已配置 Node、Rust 1.99、Xcode 及两个 targets 的环境执行：

```sh
cd experiments/desktop-spike
npm ci
npm run tauri -- build --target universal-apple-darwin --bundles app
```

本机若使用此次隔离工具链，应先为该进程配置 `.validation/toolchain/cargo/bin`、`RUSTUP_HOME` 与 `CARGO_HOME`；使用正常 rustup 环境无需采用这些临时路径。网络代理按本机既有设置配置，不把私有凭据写入仓库。

随后回到仓库根目录运行：

```sh
node scripts/validate-desktop-spike.mjs
```

验证脚本从实验构建目录直接运行应用，不复制到 `/Applications`。它检查通用二进制、最低系统版本和临时签名，并分别启动两种架构的无界面 smoke 路径及真实 WebView 窗口。Intel 路径需要本机已经可用的 Rosetta；没有 Rosetta 时应报告条件不足，不能自动标记通过。

Rust 本地集成检查：

```sh
cd experiments/desktop-spike/src-tauri
cargo test --release --target aarch64-apple-darwin --locked --offline
```

使用仓库附带的 SVG 与 icon 文件，构建不依赖额外设计资产。生成的 schemas、权限声明、node_modules、dist、target 和临时工具链均不进入 Git。

## 验证结果

| 验证 | Apple Silicon 原生 | x86_64 经 Rosetta |
| --- | --- | --- |
| 真实可执行文件启动、架构返回正确 | 通过 | 通过 |
| 固定子进程、超时边界代码及返回值 | 子进程正常执行；超时异常场景未注入 | 子进程正常执行；超时异常场景未注入 |
| 临时文件读写与 0700 / 0600 权限 | 通过 | 通过 |
| React 窗口与 Rust IPC | 通过 | 通过 |
| 已注册、未授权命令被拒绝 | 通过 | 通过 |
| 任意命令和任意文件读取入口不可用 | 通过 | 通过 |

附加检查：TypeScript 与 Vite 构建通过；Rust 集成测试 1 项通过；格式检查通过。`lipo` 证明包含两种架构；`codesign --verify --deep --strict` 对临时 ad-hoc 签名验证通过。

机器记录见 [age-4-desktop.json](evidence/age-4-desktop.json)。此证据验证实际窗口加载和 IPC，不是浏览器中模拟 Tauri 对象的测试。

## 生产实现边界

### 权限和接口

Tauri 的自定义命令默认不能想当然地受插件权限限制；本实验使用 `AppManifest::commands` 生成命令权限，并在 capability 中仅允许固定的业务入口。正式应用继续采取这一方式：

- 主窗口只加载打包资源，不允许远端页面拥有本地调用权限，配置 CSP。
- 接口使用 `instanceId`、`planId`、`restorePointId` 等受管对象标识；前端不能提交任意 shell 字符串或文件读取请求。
- Rust 服务根据适配器确认命令路径、参数、来源和文件范围，采用参数数组与最小进程环境；即使 frontend ACL 已验证，后端仍必须验证对象所有权与作用域。
- 用户补充安装路径由发现/预检模块检查后转为受管实例，不能成为通用命令执行接口。
- 网络版本查询由受控服务实现，GitHub Releases 仅查询本仓库；前端不接触用户 Agent 的认证配置。

Tauri capability 是应用接口边界，不等同于 macOS App Sandbox。本项目通过 GitHub Releases 分发、使用 Developer ID 签名与公证，首版不走 Mac App Store App Sandbox；权限范围仍须在业务层收紧，不提供管理员提权更新。

### 平台层

正式工程按 README 的 Agent / updater / backup / storage / platform 边界组织。业务层管理计划、事务与恢复点，平台层提供应用数据目录、架构、文件权限、进程占用检查和进程生命周期，Agent 适配器负责安装来源与配置定位。

macOS 采用当前用户 Application Support 目录、Unix 权限和实例锁；Windows 后续用用户数据目录、ACL 与 Windows 进程控制；Linux 后续遵循 XDG 目录、Unix 权限和桌面集成。此次实验仅为 macOS 编译和运行，未实现 Windows/Linux。

真实 npm 进程的输出上限、进程组取消、安装期间安全边界、占用检测、崩溃后恢复及数据库协议仍需 AGE-7 至 AGE-15 实现和故障注入，不能复用固定 `sw_vers` 实验的简单进程逻辑作为生产更新器。

### 发布与兼容性

- Intel 运行结果来自 Apple Silicon 上的 Rosetta，物理 Intel 机器验收仍由 AGE-15 完成。
- macOS 15 只是配置的最低版本，应补测最低系统与生产依赖的兼容性。
- 当前是 ad-hoc 签名，未用 Developer ID 签名或提交 Apple 公证；签名、公证与实际 Release 材料由 AGE-16 完成。
- 通用包内的应用二进制覆盖双架构，不代表每个外部 Agent/Node 安装实例都已覆盖双架构；该支持矩阵单独验收。
- AGE-17 的自身版本检查必须匹配 Release tag、应用内版本、架构和制品；实验 0.0.0 包不会公开发布。

## 官方参考

- [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)：Rust、平台开发工具及前端工具链。
- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)：窗口权限、自定义命令清单与远端来源边界。
- [Tauri macOS 应用包](https://v2.tauri.app/distribute/macos-application-bundle/)：minimumSystemVersion、应用资源及权限设置。
- [Tauri CLI](https://v2.tauri.app/reference/cli/)：macOS universal target 与构建选项。
- [Gemini 安装要求](https://geminicli.com/docs/get-started/installation/)：首版统一 macOS 15+ 的依据。
