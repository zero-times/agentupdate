# AgentUpdate

AgentUpdate 是 macOS 上的 AI Agent 工具更新工作台，集中发现本机 CLI 与桌面应用、查询官方版本、安装并升级。支持 Codex、Claude Code、Gemini、Deepseek CLI，以及 Codex、Claude、ChatGPT 桌面应用。

下载：[GitHub Releases](https://github.com/zero-times/agentupdate/releases/latest)。当前版本 v0.4.1，详见[更新说明](docs/releases/v0.4.1.md)。提供 macOS 15+、Apple Silicon / Intel 通用 DMG、应用 ZIP 和 SHA256SUMS。安装包采用 ad-hoc 完整性签名，未完成 Developer ID 签名与 Apple 公证。

## 同时升级

- 选择多个 CLI 工具后点击「同时升级所选工具」，每项可选择不同的目标版本，最多四项并发。
- 一个工具升级时，可以继续启动其他工具或桌面应用的升级。每项独立显示进度、成功或失败结果。
- 同一工具的多个实例共用配置，共享安装目录的 CLI 也可能共用包管理器。这些操作会依次执行；相同桌面应用不能重复安装。
- 某项失败后，其他项目继续执行。安全取消跳过尚未修改安装的项目；已经进入安装阶段的操作完成后保留真实结果。
- AgentUpdate 本应用的更新与其他安装操作互斥，避免安装重启打断升级。任务执行期间退出会被阻止，可收起到菜单栏。

## 备份与任务记录

升级前仍备份配置与旧安装，材料保存在当前用户的私有应用数据目录。历史记录可查看结果、错误码并导出脱敏日志。已结束的任务（含失败）默认折叠在页面底部，每次显示 5 条；失败摘要按安装实例去重，后续成功后不再提示旧失败。

v0.4.0 已移除恢复旧版本、回滚应用和恢复为未安装功能，包括界面入口和对应桌面调用接口。历史数据继续兼容读取，不自动删除既有备份。失败记录不会永久阻止重新检测和升级；若工具命令已经损坏，请通过官方安装方式重新安装。

## 使用

1. 打开应用，扫描本机 CLI，或通过「补充工具命令路径」添加实际命令路径。
2. 点击各工具的「检测更新」，选择目标版本或一键升级最新稳定版。Deepseek 官方预发行渠道需显式勾选。
3. 多选后同时升级，在任务记录查看每项结果。支持的来源包括用户可写 npm、Claude 官方原生与标准 Homebrew、Codex Homebrew；具体能力由发现结果决定。
4. 「桌面应用」可检测、下载、校验并安装官方应用；升级前退出目标应用。
5. 「版本更新」检查 GitHub Releases，可下载并安装 AgentUpdate 更新后自动重启。

CLI 提供「打开」按钮，可选择最近项目目录并在系统终端启动工具。操作说明见 [CLI 项目启动](docs/cli-launch.md)。

## 开发与发布

采用 Tauri 2、React、TypeScript、Rust。需要 Node.js 22.12+、Rust 1.99 和 macOS 构建工具。

```sh
npm ci
npm run dev:app
npm run check
npm run build:web
npm run package:mac -- --candidate
```

发布前同步 package.json、package-lock.json、Cargo.toml、Cargo.lock 与 tauri.conf.json 的版本，提交源码并从干净工作区构建。产物位于 `release-artifacts/v<版本>/candidate-<时间>/`。使用 `node scripts/validate-macos-package.mjs <制品目录>` 验证安装包，再将对应 tag、更新说明、DMG、应用 ZIP、SHA256SUMS 与构建清单上传 GitHub Release。

开发说明见[桌面开发](docs/desktop-development.md)，签名公证流程见[macOS 发布材料](docs/macos-release.md)。较早文档保留当时版本的行为，当前功能以本 README 和当前版本更新说明为准。
