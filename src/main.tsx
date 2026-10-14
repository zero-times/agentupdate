import { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { AppRelease } from './components/AppRelease';
import { DesktopApps } from './components/DesktopApps';
import { Workspace } from './components/Workspace';
import { getOverview, errorMessage, type AppOverview } from './api/desktop';
import './style.css';

type View = 'cli' | 'apps' | 'release';
const pages: Record<View, { label: string; title: string; detail: string; index: string }> = {
  cli: { label: 'CLI 工具', title: 'Agent 工作台', detail: '管理你的命令行工具。每次更新，都留一条回来的路。', index: '01' },
  apps: { label: '桌面应用', title: '桌面应用', detail: '从官方渠道获取更新，在这里完成安装与恢复。', index: '02' },
  release: { label: '本应用更新', title: '让工作台保持最新', detail: '查看发布说明，在线更新 AgentUpdate。', index: '03' },
};
function NavIcon({ view }: { view: View }) {
  return <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {view === 'cli' ? <><rect x="3" y="4" width="18" height="16" rx="3"/><path d="m7 9 3 3-3 3m6 0h4"/></> : view === 'apps' ? <><rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/></> : <><path d="M20 8a8 8 0 1 0 0 8M20 3v5h-5m-3 0v8m-3-3 3 3 3-3"/></>}
  </svg>;
}
function App() {
  const [overview, setOverview] = useState<AppOverview>();
  const [error, setError] = useState('');
  const [view, setView] = useState<View>('cli');
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const main = useRef<HTMLElement>(null);
  useEffect(() => { void getOverview().then(setOverview).catch(e => setError(errorMessage(e))); }, []);
  useEffect(() => { if (import.meta.env.MODE === 'desktop-validation' && overview?.validationMode) void import('./validation').then(m => overview.validationWorkflow ? m.validateWorkflow(overview) : m.validateDesktop(overview)).catch(e => setError(errorMessage(e))); }, [overview]);
  useEffect(() => { main.current?.scrollTo(0, 0); }, [view]);
  const page = pages[view];
  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark" aria-hidden="true">↗</span><div><strong>AgentUpdate</strong><span className="brand-caption">工具更新工作台</span></div></div>
      <p className="nav-caption">工作空间</p>
      <nav aria-label="工作台导航">{(Object.keys(pages) as View[]).map(key => <button key={key} data-nav={key} aria-current={view === key ? 'page' : undefined} aria-controls={`view-${key}`} onClick={() => setView(key)}><NavIcon view={key}/><span>{pages[key].label}</span>{key === 'release' && updateAvailable && <span className="update-dot" aria-label="有新版本可更新"/>}</button>)}</nav>
      <div className="sidebar-bottom"><div className="safety-note"><svg width="20" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6z"/><path d="m8 12 3 3 5-6"/></svg><strong>更新有底，恢复有据</strong><p>配置与旧版本在本机备份，恢复材料验证后再安装。</p></div><div className="device-info"><span className={`status-dot ${overview ? 'connected' : ''}`}/><span>{overview ? (overview.platform.architecture === 'aarch64' ? 'Apple Silicon' : 'Intel') : '正在连接本机…'}</span></div><span className="sidebar-version">AgentUpdate {overview ? `v${overview.appVersion}` : ''} · macOS 15+</span></div>
    </aside>
    <main ref={main}>
      <header className="page-header"><div><p className="eyebrow">WORKSPACE <span>/ {page.index}</span></p><h1>{page.title}</h1><p className="page-description">{page.detail}</p></div><span className="page-index" aria-hidden="true">{page.index}</span></header>
      {error && <div role="alert" className="error">{error}</div>}
      {!overview && !error && <p className="empty" role="status">正在连接本机工作台…</p>}
      {overview && <><section id="view-cli" data-view="cli" hidden={view !== 'cli'} aria-label="CLI 工具工作台"><Workspace/></section><section id="view-apps" data-view="apps" hidden={view !== 'apps'} aria-label="桌面应用工作台"><DesktopApps/></section><section id="view-release" data-view="release" hidden={view !== 'release'} aria-label="本应用更新工作台"><AppRelease currentVersion={overview.appVersion} onUpdateAvailable={setUpdateAvailable}/></section></>}
      <footer><span>本机管理 · 官方更新渠道</span><span>{view === 'cli' ? '⌘ R 重新扫描' : '安装前自动校验与备份'}</span></footer>
    </main>
  </div>;
}
createRoot(document.getElementById('root')!).render(<App/>);
