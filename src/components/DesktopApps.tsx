import { useEffect, useRef, useState } from 'react';
import { appInstalls, finishAppInstall, rollbackAppInstall, type AppInstall, checkDesktopApps, openDesktopChannel, errorMessage, type DesktopAppCheck } from '../api/desktop';

import {BundleInstall} from './BundleInstall';
const labels: Record<string,string> = {not_installed:'未安装',update_available:'发现新版本',up_to_date:'与公开稳定版一致',local_newer:'本机版本较新',unknown_version:'版本无法比较',failed:'检查失败',not_checked:'尚未检查'};
export function DesktopApps() {
  const [records,setRecords]=useState<AppInstall[]>([]);
  const [checks,setChecks] = useState<DesktopAppCheck[]>([]);
  const [busy,setBusy] = useState(false);
  const [error,setError] = useState('');
  const active = useRef(false);
  async function refresh() {
    if(active.current) return;
    active.current=true;setBusy(true);setError('');
    try {setChecks(await checkDesktopApps());setRecords(await appInstalls());} catch(e) {setError(errorMessage(e));}
    finally {active.current=false;setBusy(false);}
  }
  async function operate(job:()=>Promise<unknown>){if(active.current)return;active.current=true;setBusy(true);let reboot=false;try{const result=await job();if(typeof result==='object'&&result!==null&&'state' in result&&result.state==='restarting'){reboot=true;return;}setRecords(await appInstalls());setChecks(await checkDesktopApps());}catch(e){setError(errorMessage(e));}finally{if(!reboot){active.current=false;setBusy(false);}}}
  useEffect(()=>{void refresh();},[]);
  return <section className="desktop-apps" aria-label="桌面应用更新检测">
    <div className="section-heading"><div><p className="eyebrow">官方渠道 / 本机应用</p><h2>应用版本与更新检测</h2></div><button disabled={busy} onClick={()=>void refresh()}>{busy?'正在检测…':'重新检测应用'}</button></div>
    <p className="muted">检测本机应用，与官方公开渠道比较。可在这里下载、校验、备份并安装；升级前请退出目标应用。</p>
    {error&&<p className="error" role="alert">{error}</p>}
    {!checks.length&&<p className="empty">{busy?'正在读取应用版本与公开渠道…':'尚未检查桌面应用。'}</p>}
    <div className="tool-grid">{checks.map(check=><article className="tool-card desktop-app-card" key={check.product} data-desktop-product={check.product}>
      <header><span className={`agent-symbol ${check.product}`} aria-hidden="true">{check.product==='codex'?'>_':check.product==='claude'?'✳':'◎'}</span><h2>{check.name}</h2><span className="pill" role="status">{labels[check.state]??check.state}</span></header>
      {!check.installed.length&&<p className="empty">未发现独立应用安装。</p>}
      {check.installed.map(app=><div className="instance" key={app.path}><strong>本机 {app.version??'版本无法读取'}</strong><span className="muted"> · {labels[app.state]??app.state}</span><p className="path">{app.path}</p></div>)}
      <p>公开稳定版本：<strong>{check.latestVersion??'本次未获取'}</strong></p>
      {check.errorCode&&<p className="notice" role="alert">本次查询失败（{check.errorCode}），无法判断是否有更新。</p>}
      <p className="muted">检查于 {new Date(check.checkedAt).toLocaleString('zh-CN')}</p>
      <p className="muted">{check.channelNote}</p>
      <div className="version-controls">{check.downloadAvailable&&['not_installed','update_available','unknown_version'].includes(check.state)&&<BundleInstall product={check.product} label={check.installed.length?'下载并升级':'一键安装'} onInstalled={()=>void refresh()}/>}<button onClick={()=>void openDesktopChannel(check.product).catch(e=>setError(errorMessage(e)))}>打开官方更新入口</button>{check.downloadAvailable&&<button onClick={()=>void openDesktopChannel(check.product,true).catch(e=>setError(errorMessage(e)))}>打开匹配安装包</button>}</div>
    </article>)}</div>
    {!!records.length&&<div className="history" aria-label="应用安装记录"><h2>应用安装与恢复记录</h2>{records.map(r=><div className="instance" key={r.id}><strong>{r.product} · {r.version}</strong><span> · {({prepared:'已准备 · 等待安装',installing:'安装中断 · 需恢复',succeeded:'安装完成',needs_recovery:'待恢复',restored:'已恢复',helper_failed:'后台安装失败',installed_not_restarted:'已安装 · 重启未完成'} as Record<string,string>)[r.state]??r.state}</span><p className="path">{r.destination}</p>{r.failureCode&&<p className="notice">{r.failureCode}</p>}{r.state==='prepared'&&<button disabled={busy} onClick={()=>void operate(()=>finishAppInstall(r.id))}>安装{r.product==='agentupdate'?'并重启':''}</button>}{['succeeded','installing','needs_recovery','installed_not_restarted'].includes(r.state)&&<button disabled={busy} onClick={()=>void operate(()=>rollbackAppInstall(r.id))}>{r.hasBackup?'恢复旧应用':'恢复为未安装'}{r.product==='agentupdate'?'并重启':'（保留当前配置）'}</button>}</div>)}</div>}
  </section>;
}
