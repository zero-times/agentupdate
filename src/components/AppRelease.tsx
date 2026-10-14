import {useCallback,useEffect,useRef,useState} from 'react';
import {checkRelease,openRelease,errorMessage,type ReleaseCheck} from '../api/desktop';
import {BundleInstall} from './BundleInstall';
const labels:Record<ReleaseCheck['state'],string>={not_checked:'尚未检查',no_releases:'暂无稳定发布版本',update_available:'发现新版本',up_to_date:'已是最新稳定版',local_newer:'本地版本高于远端',failed:'检查失败'};
export function AppRelease({currentVersion,onUpdateAvailable}:{currentVersion:string;onUpdateAvailable?:(available:boolean)=>void}) {
 const [result,setResult]=useState<ReleaseCheck>();const [busy,setBusy]=useState(false);const [error,setError]=useState('');const inFlight=useRef(false);
 const check=useCallback(async(manual:boolean)=>{if(inFlight.current)return;inFlight.current=true;setBusy(true);setError('');try{setResult(await checkRelease(manual));}catch(e){setError(errorMessage(e));}finally{inFlight.current=false;setBusy(false);}},[]);
 useEffect(()=>{void check(false);},[check]);
 useEffect(()=>{onUpdateAvailable?.(result?.state==='update_available');},[result,onUpdateAvailable]);
 return <section className="release-panel" aria-label="关于与应用更新"><p className="release-label">AGENTUPDATE / RELEASE CHANNEL</p><div className="app-version"><div><h2>关于 AgentUpdate</h2><p>当前 v{currentVersion}<span>·</span>{error?'检查失败':result?labels[result.state]:busy?'正在后台检查…':'尚未检查'}{result?.state==='update_available'&&` · v${result.latest?.version}`}</p></div><button disabled={busy} onClick={()=>void check(true)}>{busy?'检查中…':'检查应用更新'}</button></div>
 {(error||result?.state==='failed')&&<p role="alert" className="notice">{error||`GitHub 检查失败（${result?.errorCode}），未确认最新版本。`}{result&&result.retryAfter>Date.now()&&` ${new Date(result.retryAfter).toLocaleString('zh-CN')} 后可重试。`}</p>}
 {result?.checkedAt&&<p className="muted">最近检查：{new Date(result.checkedAt).toLocaleString('zh-CN')}{result.cached?' · 缓存结果':''}{result.state==='failed'&&result.successfulAt?` · 上次成功：${new Date(result.successfulAt).toLocaleString('zh-CN')}`:''}</p>}
 {result?.latest&&<details open={result.state==='update_available'}><summary>{result.state==='failed'?'上次已确认发布：':'稳定发布：'}v{result.latest.version} · {result.latest.publishedAt?new Date(result.latest.publishedAt).toLocaleDateString('zh-CN'):'公开稳定渠道'}</summary><div className="release-notes">{result.latest.notes||'该发布没有填写更新说明。'}</div><div className="release-actions">{result.state==='update_available'&&result.asset&&<BundleInstall product="agentupdate" label="下载更新并安装重启"/>}<button onClick={()=>void openRelease(false).catch(e=>setError(errorMessage(e)))}>查看 GitHub Release</button>{result.asset?<button className="primary" onClick={()=>void openRelease(true).catch(e=>setError(errorMessage(e)))}>打开安装包下载</button>:<span className="muted">没有匹配当前 macOS 架构的安装包。</span>}</div><p className="muted">应用内安装需要可信 SHA256 校验；将备份旧应用，通过后台进程完成替换和重启。</p></details>}
 </section>;
}
