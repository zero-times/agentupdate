# AGE-3：Agent 安装与恢复调研

核验日期：2026-10-12。测试机为 Apple Silicon、macOS 27.0.1，Node.js 26.8.2。本文记录首批选择依据和可复现的实验，不代表生产更新器已完成。

## 首批选择

首版先实现 **Codex CLI 与 Gemini CLI 的用户可写 npm 安装实例**。两种工具均已在独立 npm prefix 内完成固定旧版安装、更新和离线恢复。系统级 npm prefix、Homebrew、原生安装器和 Claude Code 保持“待验证”状态，发现时可显示信息，不能据此开放自动修改。

| 工具 / 来源 | 官方依据与版本来源 | 配置与检查 | 本次结论 |
| --- | --- | --- | --- |
| Codex CLI / npm | 官方 CLI 文档提供 npm 安装；包为 `@openai/codex`，稳定选择 npm `latest`；本次为 0.162.1 | 用户配置 `~/.codex/config.toml`；存在项目配置层；通过实例的 `codex --version` 和 `--help` 检查 | arm64 的 0.162.0 → 0.162.1 → 0.162.0 已验证；首批候选已确定 |
| Gemini CLI / npm | 官方文档提供 npm 安装，`@google/gemini-cli@latest` 为稳定渠道；本次为 0.63.0 | `~/.gemini/settings.json`，可用 `GEMINI_CLI_HOME` 隔离用户存储；实例的 `gemini --version`、`--help` | arm64 的 0.62.0 → 0.63.0 → 0.62.0 已验证；首批候选已确定 |
| Claude Code / npm、默认用户原生、标准 Homebrew Cask | 2026-10-13 已加入生产适配；来源各自查询最新稳定版，Homebrew 与官方原生渠道可能不同 | 默认 settings/credentials、`~/.claude.json`；隔离 `--version` / `--help` | 原生 arm64 真实 2.1.286 → 2.1.287 → 原版通过；Homebrew 日常升级及 Claude npm 真实往返未执行，详见 [追加功能](claude-and-desktop-updates.md) |
| Codex CLI / 原生 | 官方 CLI 文档提供独立安装器 | 安装根目录、旧版制品与 launcher 的恢复协议尚未验证 | 暂不开放修改，后续优先调研 |
| 任一工具 / Homebrew | 各工具支持情况应依据各自官方文档与实际 cask/formula | 需识别实际 Cellar/Caskroom、架构与命令链接 | 不能假设任意旧版本可获取或重装；本次未执行 brew 安装或降级 |

当前 Codex npm 元数据要求 Node.js ≥16，Gemini 要求 ≥20，Claude Code npm 要求 ≥22。首版 npm 管理路径建议预检 Node.js ≥22.12；本次实际验证使用 26.8.2，不能把最低版本也标成已测试。Gemini 官方当前列出的 macOS 基线为 15+，因此首版应用统一设为 macOS 15+。

Claude Code 的当前 npm 包使用平台原生二进制与 postinstall，并非所有 CLI 都能采用相同的安装策略。其自动更新与最低版本设置也可能影响降级，不能直接复用本次两种工具的结果。后续增加该适配器时需验证来源、签名、后台更新竞争和回滚限制。

## 实验流程与证据

运行 [验证脚本](../scripts/validate-agent-versions.mjs)：

```sh
node scripts/validate-agent-versions.mjs
```

如本机依赖 HTTPS 代理，使用本机已有代理环境；脚本不自行修改系统网络或 npm 配置。运行需要 macOS 的 `sandbox-exec`、Node.js/npm 和 npm registry 网络访问。

脚本为每次执行创建独立临时目录；npm 的 `--global` 始终搭配该目录的 `--prefix`，不调用系统的全局安装路径。使用空的独立 user/global npm 配置，缓存写入被忽略的 `.validation/`。测试配置为人工构造的无凭据 fixture。

1. 查询固定版本的官方 npm 元数据并安装旧版，确认 `--version` 和 `--help`。
2. 复制旧版的完整隔离安装树及配置，比较文件 SHA-256、权限和链接信息。
3. 安装目标版本，验证实际版本及配置没有被安装操作修改。
4. 人为模拟配置迁移失败：改写 fixture、改变权限，并新增一份受管测试文件。
5. 从本地快照恢复安装与 fixture，不重新下载旧版、不执行 npm 降级。
6. 禁止网络及真实用户 Agent 配置访问的进程再次检查旧版；比较完整安装树与配置清单，确认原始内容、权限和缺失状态恢复。
7. 删除实验临时目录，保留脱敏结果。

| 工具 | 旧版 | 升级目标 | 恢复结果 | 验证 |
| --- | --- | --- | --- | --- |
| Codex CLI | 0.162.0 | 0.162.1 | `codex-cli 0.162.0` | 版本、帮助、离线安装树恢复、配置内容/权限/缺失状态通过 |
| Gemini CLI | 0.62.0 | 0.63.0 | `0.62.0` | 版本、帮助、离线安装树恢复、配置内容/权限/缺失状态通过 |

完整记录见 [age-3-arm64.json](evidence/age-3-arm64.json)，包含源制品 URL、npm SRI、配置摘要与逐项结果。安装使用 `--ignore-scripts`，这两种固定版本的启动检查通过；该选项不构成其他版本/工具的安装保证。初次下载遇到网络断开，使用已有代理重新执行后通过，也说明生产更新器必须在下载失败时保留可用恢复点。

## 能力边界与适配器约束

- 已验证架构为 arm64；Intel Agent 的真实安装/恢复需要在 AGE-15 补验。桌面 Intel/Rosetta 实验不等价于 Intel Agent 安装验收。
- `--version` 和 `--help` 验证本地启动，不登录、不执行推理、不读取真实凭据，也不承诺远端服务与模型兼容性。
- 本次恢复整个 prefix 是因为 prefix 完全属于实验。生产环境常有共享 prefix，必须按安装实例列出其拥有的包树与 launcher，不能恢复整个共享目录或覆盖其他工具。
- npm `dist.integrity` 与依赖 lockfile 负责下载校验；本地旧版快照另外保存文件校验清单。仅保存主包 tarball 不足以保证离线恢复，还需要平台可执行文件及依赖树。
- “最新”必须先解析渠道为固定版本，固定计划后再下载，不能执行时再次隐式选择不同版本。Codex/Gemini 的 `latest` 稳定含义已确认；Claude 的渠道需独立处理。
- 每个实例保存实际命令路径、包根、prefix、平台/架构和来源。后续发现模块需处理 nvm、fnm、多个 Node 版本及同名命令，不能仅以 PATH 第一项作唯一实例。
- 默认备份用户配置；项目配置、环境变量和系统托管配置只识别并说明来源，不擅自修改。认证文件和系统 Keychain 的恢复不在此次 fixture 实验内，需在生产备份协议中明确可恢复范围。
- 恢复写入前验证快照；实际配置有后续变化时先保留当前副本。配置清理仅处理受管路径，不能像实验一样删除整个真实用户配置根。
- 后续版本需要覆盖占用、磁盘不足、快照损坏、符号链接、安装中止、进程锁与崩溃恢复；此次成功实验不能替代 AGE-8 至 AGE-15 的实现验收。

## 实施估算

以下按一名开发者、首批两种工具、macOS 两种架构及现有技术验证估算，单位为工作日；阶段可局部交错，不是承诺发布日期。

| 阶段 | 估算 | 前提 |
| --- | --- | --- |
| AGE-5 至 AGE-7：工程、发现和事务存储 | 4–6 天 | 技术方案确认，明确用户可写安装范围 |
| AGE-8 至 AGE-11：备份、单工具更新/回滚与界面 | 7–10 天 | 配置范围与共享 prefix 的所有权协议明确 |
| AGE-12 至 AGE-14、AGE-17：批量、恢复、历史及自身版本检查 | 5–8 天 | 单项事务可靠，GitHub Release 制品约定确定 |
| AGE-15、AGE-16：双架构验收与发布 | 4–6 天 | Intel 测试环境、Developer ID 与公证材料可用 |

合计约 20–30 个工作日；增加 Claude 原生支持、系统级安装或全自动应用升级会改变此估算。

## 官方来源

- [Codex CLI](https://learn.chatgpt.com/docs/codex/cli)：已获取该页面的 Markdown 版本核实 npm、独立安装器及 Homebrew 入口。
- [Codex 配置](https://learn.chatgpt.com/docs/config-file/config-basic)：用户与项目配置层。
- [Gemini 安装与发布渠道](https://geminicli.com/docs/get-started/installation/)：安装方式、运行要求与稳定渠道。
- [Gemini 配置](https://geminicli.com/docs/reference/configuration/)：配置层及 `GEMINI_CLI_HOME`。
- [Claude Code 安装与更新](https://code.claude.com/docs/en/setup)：指定版本、npm 原生包、自动更新和版本策略。
- [Claude Code 配置文件](https://code.claude.com/docs/en/settings)：用户与项目配置范围。
- [Codex npm 元数据](https://registry.npmjs.org/@openai%2fcodex)、[Gemini npm 元数据](https://registry.npmjs.org/@google%2fgemini-cli)、[Claude Code npm 元数据](https://registry.npmjs.org/@anthropic-ai%2fclaude-code)：版本、引擎要求和制品校验信息。
