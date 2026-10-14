import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { AppRelease } from './components/AppRelease';
import { DesktopApps } from './components/DesktopApps';
import { Workspace } from './components/Workspace';
import { getOverview, errorMessage, type AppOverview } from './api/desktop';
import './style.css';
function App() {
  const [overview,setOverview]=useState<AppOverview>();
  const [error,setError]=useState('');
  useEffect(()=>{void getOverview().then(setOverview).catch(e=>setError(errorMessage(e)));},[]);
  useEffect(()=>{if(import.meta.env.MODE==='desktop-validation'&&overview?.validationMode) void import('./validation').then(m=>overview.validationWorkflow?m.validateWorkflow(overview):m.validateDesktop(overview)).catch(e=>setError(errorMessage(e)));},[overview]);
  return <div className="app-shell"><header className="toolbar"><div className="brand"><span className="brand-mark" aria-hidden="true">↑</span><strong>AgentUpdate</strong><span className="stage">macOS</span></div><span className="muted">{overview?`v${overview.appVersion} · ${overview.platform.architecture==='aarch64'?'Apple Silicon':'Intel'}`:'正在连接…'}</span></header><main><div className="intro"><p className="eyebrow">你的工具，一处管理</p><h1>Agent 工作台</h1><p>选择版本，安心更新。需要时，回到原来的状态。</p></div>{error&&<div role="alert" className="error">{error}</div>}{overview&&<Workspace/>}{overview&&<DesktopApps/>}{overview&&<AppRelease currentVersion={overview.appVersion}/>}</main><footer><span>配置默认备份 · 旧版本离线恢复</span><span>macOS 15+</span></footer></div>;
}
createRoot(document.getElementById('root')!).render(<App/>);
