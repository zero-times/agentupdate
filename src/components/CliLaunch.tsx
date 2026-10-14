import { useEffect, useRef, useState } from 'react';
import { cliProjects, errorMessage, openCli, saveCliProject, type Installation } from '../api/desktop';

export function CliLaunch({installation, onClose}: {installation: Installation; onClose: () => void}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [projects, setProjects] = useState<string[]>([]);
  const [selected, setSelected] = useState('');
  const [path, setPath] = useState('');
  const [profile, setProfile] = useState('web');
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const pending = useRef(false);
  useEffect(() => {
    dialog.current?.showModal();
    let active = true;
    void cliProjects().then(paths => {
      if (!active) return;
      setProjects(paths); setSelected(paths[0] ?? ''); setPath(paths[0] ?? '');
    }).catch(e => { if (active) setError(errorMessage(e)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, []);
  async function submit(launch: boolean) {
    if (pending.current || loading || !path) return;
    pending.current = true; setBusy(true); setError(''); setNotice('');
    try {
      if (launch) { await openCli(installation.id, path, selected || undefined, installation.agentId === 'deepseek' ? profile : undefined); onClose(); }
      else {
        const paths = await saveCliProject(path, selected || undefined);
        setProjects(paths); setSelected(paths[0]); setPath(paths[0]); setNotice('项目目录已保存');
      }
    } catch (e) { setError(errorMessage(e)); }
    finally { pending.current = false; setBusy(false); }
  }
  return <dialog ref={dialog} className="cli-launch" aria-label="选择项目目录" onCancel={e => {e.preventDefault(); if (!pending.current) onClose();}}>
    <h2>打开 {installation.agentId === 'deepseek' ? 'Deepseek Harness' : installation.agentId === 'claude' ? 'Claude Code CLI' : `${installation.agentId === 'codex' ? 'Codex' : 'Gemini'} CLI`}</h2>
    <p>选择项目目录，在系统终端中启动这个 CLI。</p>
    <p className="path">{installation.commandPath}</p>
    {loading ? <p role="status">正在读取最近项目…</p> : <form onSubmit={e => {e.preventDefault(); void submit(true);}}>
      <label>最近项目<select value={selected} disabled={busy} onChange={e => {setSelected(e.target.value); setPath(e.target.value); setError(''); setNotice('');}}>
        <option value="">新增项目目录</option>
        {projects.map(project => <option key={project} value={project}>{project}</option>)}
      </select></label>
      <label>项目目录<input autoFocus aria-label="项目目录" value={path} disabled={busy} onChange={e => {setPath(e.target.value); setNotice('');}} placeholder="例如 /Users/you/Projects/my-project" required /></label>
      {installation.agentId === 'deepseek' && <>
        <label>启动 profile<input aria-label="Deepseek profile" value={profile} disabled={busy} onChange={e => setProfile(e.target.value)} placeholder="web" required maxLength={128}/></label>
        <p className="muted">默认 web 启动 Harness 网页界面。其他名称须为已安装的 profile；tui 不属于内置模板。</p>
      </>}
      <p className="muted">可修改所选目录，或新增项目目录。最近使用的 20 个目录会保存在本机。</p>
      {error && <p className="error" role="alert">{error}</p>}
      {notice && <p role="status">{notice}</p>}
      <div className="dialog-actions">
        <button type="button" disabled={busy} onClick={onClose}>取消</button>
        <button type="button" disabled={busy} onClick={() => {setSelected(''); setPath(''); setError(''); setNotice(''); dialog.current?.querySelector<HTMLInputElement>('input')?.focus();}}>新增目录</button>
        <button type="button" disabled={busy || !path} onClick={() => void submit(false)}>保存目录</button>
        <button className="primary" disabled={busy || !path || (installation.agentId === 'deepseek' && !profile)} type="submit">{busy ? '正在处理…' : '在此目录打开 CLI'}</button>
      </div>
    </form>}
    {loading && <div className="dialog-actions"><button onClick={onClose}>取消</button></div>}
  </dialog>;
}
