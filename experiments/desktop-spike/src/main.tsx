import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import './style.css';

type Report = { platform: string; architecture: string; osVersion: string;
  childProcess: boolean; isolatedFileRoundtrip: boolean; restrictiveFilePermissions: boolean };

async function expectDenied(command: string, args?: Record<string, unknown>) {
  try { await invoke(command, args); return false; } catch { return true; }
}
function App() {
  const [report, setReport] = useState<Report>();
  const [error, setError] = useState('');
  const [checks, setChecks] = useState<Record<string, boolean>>();
  useEffect(() => {
    async function validate() {
      try {
        const result = await invoke<Report>('run_validation');
        const [restrictedCommandDenied, arbitraryCommandDenied, arbitraryFileDenied] = await Promise.all([
          expectDenied('admin_only_probe'),
          expectDenied('run_arbitrary_command', { executable: '/bin/sh', args: ['-c', 'true'] }),
          expectDenied('read_arbitrary_file', { path: '/etc/hosts' }),
        ]);
        const permissions = { restrictedCommandDenied, arbitraryCommandDenied, arbitraryFileDenied };
        setReport(result); setChecks(permissions);
        await invoke('finish_validation', { checks: permissions });
      } catch (e) { setError(String(e)); }
    }
    void validate();
  }, []);
  const passed = report && checks && Object.values(checks).every(Boolean) && report.childProcess && report.isolatedFileRoundtrip && report.restrictiveFilePermissions;
  return <main>
    <p className="eyebrow">AGE-4 · 本地技术实验</p>
    <h1>AgentUpdate</h1>
    <p>桌面窗口、受限本地调用和双架构验证。</p>
    <section aria-live="polite">
      <h2>{error ? '验证失败' : passed ? '验证通过' : '正在验证…'}</h2>
      {error && <p role="alert">{error}</p>}
      {report && <dl>
        <dt>运行环境</dt><dd>macOS {report.osVersion} / {report.architecture}</dd>
        <dt>固定子进程</dt><dd>{report.childProcess ? '通过' : '失败'}</dd>
        <dt>独立文件读写与权限</dt><dd>{report.isolatedFileRoundtrip && report.restrictiveFilePermissions ? '通过' : '失败'}</dd>
        <dt>前端权限限制</dt><dd>{checks && Object.values(checks).every(Boolean) ? '通过' : '验证中'}</dd>
      </dl>}
    </section>
    <small>此实验不扫描、安装或更新本机 Agent。</small>
  </main>;
}
createRoot(document.getElementById('root')!).render(<App />);
