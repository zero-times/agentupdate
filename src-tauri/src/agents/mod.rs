use serde::{Deserialize, Serialize};
pub mod channels;
pub mod discovery;
pub mod versions;

// IDs are an explicit allowlist, never executable paths or package names from the WebView.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentId {
    Codex,
    Gemini,
    Claude,
    Deepseek,
}

impl AgentId {
    pub fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Deepseek => "dsh",
        }
    }
    pub fn package(self) -> &'static str {
        match self {
            Self::Codex => "@openai/codex",
            Self::Gemini => "@google/gemini-cli",
            Self::Claude => "@anthropic-ai/claude-code",
            Self::Deepseek => "@deepseek-ai/dsh",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSupport {
    pub id: AgentId,
    pub name: &'static str,
    pub installation_source: &'static str,
    pub detection_state: &'static str,
    pub update_enabled: bool,
    pub description: &'static str,
}

pub fn support(id: AgentId) -> AgentSupport {
    let (name, description) = match id {
        AgentId::Claude => (
            "Claude Code CLI",
            "支持用户 npm、原生安装和 Homebrew Cask 最新稳定版升级。",
        ),
        AgentId::Codex => ("Codex CLI", "支持 npm 与 Homebrew Cask 检测与升级。"),
        AgentId::Deepseek => (
            "Deepseek Harness",
            "官方 dsh npm 渠道；预发行版本需显式选择。",
        ),
        AgentId::Gemini => (
            "Gemini CLI",
            "首批支持用户可写的 npm 安装；需要 macOS 15 或更高版本。",
        ),
    };
    AgentSupport {
        id,
        name,
        installation_source: "user_npm",
        detection_state: "not_scanned",
        update_enabled: false,
        description,
    }
}

pub fn catalog() -> Vec<AgentSupport> {
    [
        AgentId::Codex,
        AgentId::Claude,
        AgentId::Deepseek,
        AgentId::Gemini,
    ]
    .into_iter()
    .map(support)
    .collect()
}
