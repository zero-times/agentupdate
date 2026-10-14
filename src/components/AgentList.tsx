import type { AgentId, AgentSupport } from '../api/desktop';

export function AgentList({ agents, selected, onSelect, loading }: {
  agents: AgentSupport[]; selected?: AgentId;
  onSelect: (id: AgentId) => void; loading: boolean;
}) {
  return <section className="agent-list" aria-labelledby="agents-title">
    <div className="section-heading"><h2 id="agents-title">首批支持工具</h2><span>{agents.length} 项</span></div>
    {agents.map(agent => <button key={agent.id} data-agent={agent.id}
      className={`agent-row ${selected === agent.id ? 'selected' : ''}`}
      aria-pressed={selected === agent.id} disabled={loading} onClick={() => onSelect(agent.id)}>
      <span className={`agent-symbol ${agent.id}`} aria-hidden="true">{agent.id === 'codex' ? '>_' : '✦'}</span>
      <span className="agent-copy"><strong>{agent.name}</strong><span>用户目录 · npm</span></span>
      <span className="pill">尚未扫描</span>
      <span className="chevron" aria-hidden="true">›</span>
    </button>)}
    <p className="list-note">这是支持范围，安装实例将在工具发现功能接入后显示。</p>
  </section>;
}
