import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { createRoot } from 'react-dom/client';
import { AppRelease } from './components/AppRelease';
import { DesktopApps } from './components/DesktopApps';
import { Workspace } from './components/Workspace';
import { getOverview, hideMainWindow, errorMessage, type AppOverview, type ReleaseCheck } from './api/desktop';
import './style.css';

type View = 'cli' | 'apps' | 'release';
const pages: Record<View, { label: string; title: string }> = {
  cli: { label: 'CLI 工具', title: 'Agent 工作台' },
  apps: { label: '桌面应用', title: '桌面应用' },
  release: { label: '版本更新', title: '版本更新' },
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
  const [availableRelease, setAvailableRelease] = useState<ReleaseCheck['latest']>(null);
  const main = useRef<HTMLElement>(null);
  useEffect(() => { void getOverview().then(setOverview).catch(e => setError(errorMessage(e))); }, []);
  useEffect(() => { if (import.meta.env.MODE === 'desktop-validation' && overview?.validationMode) void import('./validation').then(m => overview.validationPanel ? m.validatePanel(overview) : overview.validationCliLaunch ? m.validateCliLaunch(overview) : overview.validationWorkflow ? m.validateWorkflow(overview) : m.validateDesktop(overview)).catch(e => setError(errorMessage(e))); }, [overview]);
  useEffect(() => {
    const listener = listen('panel-exit-blocked', () => setError('当前安装或升级尚未结束，请完成后再退出。')).catch(() => () => {});
    return () => { void listener.then(unlisten => unlisten()).catch(() => {}); };
  }, []);
  useEffect(() => { main.current?.scrollTo(0, 0); }, [view]);
  const page = pages[view];
  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark" aria-hidden="true">↗</span><div><strong>AgentUpdate</strong><span className="brand-caption">工具更新工作台</span></div></div>
      <p className="nav-caption">工作空间</p>
      <nav aria-label="工作台导航">{(Object.keys(pages) as View[]).map(key => <button key={key} data-nav={key} aria-current={view === key ? 'page' : undefined} aria-controls={`view-${key}`} onClick={() => setView(key)}><NavIcon view={key}/><span>{pages[key].label}</span>{key === 'release' && availableRelease && <span className="update-badge">新版本</span>}</button>)}</nav>
      <div className="sidebar-bottom"><div className="device-info"><span className={`status-dot ${overview ? 'connected' : ''}`}/><span>{overview ? (overview.platform.architecture === 'aarch64' ? 'Apple Silicon' : 'Intel') : '正在连接本机…'}</span></div><span className="sidebar-version">AgentUpdate {overview ? `v${overview.appVersion}` : ''} · macOS 15+</span></div>
    </aside>
    <main ref={main}>
      <header className="page-header"><div><h1>{page.title}</h1></div><button className="collapse-panel" aria-label="收起到菜单栏" title="收起到菜单栏" onClick={() => void hideMainWindow().catch(e => setError(errorMessage(e)))}><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M5 4h14M12 20V8m-5 5 5-5 5 5"/></svg></button></header>
      {availableRelease && view !== 'release' && <div className="update-banner" role="status"><span className="update-banner-icon" aria-hidden="true">↑</span><div><strong>AgentUpdate v{availableRelease.version} 已发布</strong><p>新版本可用，查看更新内容并安装。</p></div><button className="primary" onClick={() => setView('release')}>查看版本更新</button></div>}
      {error && <div role="alert" className="error">{error}</div>}
      {!overview && !error && <div className="startup-loading" role="status" aria-label="正在连接本机工作台">
        <span className="skeleton-line skeleton-title" aria-hidden="true" />
        <div className="tool-grid" aria-hidden="true">
          {[0, 1, 2, 3].map(i => <div key={i} className="tool-card skeleton-card">
            <span className="skeleton-line" style={{ width: '46%' }} />
            <span className="skeleton-line" />
            <span className="skeleton-line" style={{ width: '72%' }} />
          </div>)}
        </div>
      </div>}
      {overview && <><section id="view-cli" data-view="cli" hidden={view !== 'cli'} aria-label="CLI 工具工作台"><Workspace/></section><section id="view-apps" data-view="apps" hidden={view !== 'apps'} aria-label="桌面应用工作台"><DesktopApps/></section><section id="view-release" data-view="release" hidden={view !== 'release'} aria-label="版本更新工作台"><AppRelease currentVersion={overview.appVersion} onUpdateAvailable={setAvailableRelease}/></section></>}
      {view === 'cli' && <footer><span>⌘ R 重新扫描</span></footer>}
    </main>
  </div>;
}
createRoot(document.getElementById('root')!).render(<App/>);
