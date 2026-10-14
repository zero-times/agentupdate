# AGE-6：安装发现与版本查询

已接入 Rust 领域服务和受限 IPC，完整产品界面在 AGE-11 接入。

## 发现与身份核验

扫描当前 PATH、常见用户 bin、nvm/fnm 的版本目录和用户显式补充的目录或 codex/gemini 命令路径。补充路径必须为绝对路径，最多 32 项；它只用于发现，不能直接变成命令执行请求。

依据规范化的实际启动路径生成稳定实例 ID。同一真实启动路径的别名去重，不同 prefix 的安装分别保留。核对官方包名、全局 npm 目录布局、package.json 的 bin 声明、当前用户所有权和权限。Homebrew、系统级 npm、原生或无法确认来源的实例显示限制，不自动修改。

用户 npm 实例通过已确认的 Node 和启动文件进行版本检查；要求 Node ≥22.12，返回实际 arm64/x64 架构，并比较实际版本与包声明。进程在临时目录中运行，清空环境，不继承真实认证信息；macOS sandbox 禁止网络、真实 Agent/SSH/npm/Keychain 配置读取和测试目录之外的写入。只执行 --version 及固定 Node 版本/架构检查，不调用模型。

输出限制为 64 KiB。Node 探测和 arm64 工具单进程最多 12 秒；已确认使用 x64 Node 的工具启动最多 60 秒，以容纳实测约 31 秒的 Rosetta 二进制冷启动。超时回收所属进程组。扫描在每个候选开始前检查 120 秒总时限，避免第一个工具的冷启动用尽预算后跳过第二个工具；已经开始的候选继续受上述单进程时限约束，因此整个扫描可能超过 120 秒。来源、命令路径、包根、prefix、Node 路径、实际版本、架构及支持原因均由后端返回。实例未通过核验时不开放更新。

## 官方版本与选择

仅查询固定的 registry.npmjs.org 官方包端点，不接受前端 URL，不跟随重定向。请求连接最多 8 秒、整体最多 15 秒，最多两次尝试、间隔 500 毫秒，响应最多 32 MiB。网络或响应失败返回明确错误，不生成可执行目标。

按语义化版本排序，稳定列表排除预发行。`latest` 使用官方稳定渠道，先解析为固定版本；指定版本必须存在于有效元数据中，预发行必须显式开启。制品 URL 必须与官方包及版本完全对应，SRI 必须含有效的 64 字节 SHA-512 校验值。后续创建更新计划仍需重新核验选择，不直接相信前端提供的制品地址。

生产命令：`discover_agents(extraPaths)`、`list_agent_versions(agentId, includePrerelease)`、`select_agent_version(agentId, target, allowPrerelease)`。均通过 AppManifest / main capability 显式开放，不提供通用进程或文件接口。此任务不执行安装、备份或恢复。

## 验证

Rust 测试覆盖多个 prefix、实际/声明版本不符、路径校验、0.9/0.10 排序、预发行显式选择、不存在版本、错误包身份、非官方制品、错误 SRI 和网络失败的有限重试。

真实隔离实验安装 Codex 0.162.0 和 Gemini 0.62.0，生产发现代码正确返回 user_npm、arm64 和实际版本；官方查询分别得到最新稳定 0.162.1（202 个有效稳定版本）与 0.63.0（199 个）。日期与机器记录见 [age-6-discovery.json](evidence/age-6-discovery.json)。这些数量是检查时的结果，不作为程序中的固定值。

```sh
cargo test --manifest-path src-tauri/Cargo.toml --release --locked
node scripts/validate-agent-discovery.mjs
```

真实验证脚本只操作自建临时 prefix，复用 AGE-3 的 npm 下载缓存；运行前可先执行 AGE-3 验证脚本准备缓存。临时安装与结果文件在结束时清理，脱敏证据保存到 docs/evidence。原生及系统安装的完整版本查询、Windows/Linux、真实 Intel Agent 与 UI 操作路径仍由对应后续任务验收。
