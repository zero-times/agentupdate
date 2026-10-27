import { useCallback, useEffect, useRef, useState } from 'react';
import { TaskHistory } from './TaskHistory';
import { RecoveryNotice, blockingTransaction } from './RecoveryNotice';
import { CliLaunch } from './CliLaunch';
import {OperationProgress,useOperationProgress,initialProgress} from './OperationProgress';
import {Spinner,Skeleton} from './Spinner';
import { installCli, cliInstalls, type CliInstall, exportTransaction, revealExport, type ExportResult, inspectInterrupted, type RecoveryReport, cancelPlan, executePlan, createPlan, discoverAgents, errorMessage, listVersions, planHistory, type AgentId, type DiscoveryReport, type Installation, type Stage, type UpdatePlan, type UpdateTransaction, type VersionCatalog } from '../api/desktop';

export const stageNames: Record<Stage, string> = {queued:'等待执行',preflight:'检查更新条件',backing_up:'备份配置与旧版本',prepared:'备份已就绪',installing:'正在安装',verifying:'验证更新结果',succeeded:'更新成功',preparation_failed:'准备失败 · 安装未修改',needs_recovery:'更新失败',recovering:'历史任务未完成',restored:'历史任务已结束',recovery_failed:'历史任务失败',cancelled:'已取消'};
const failureMessages: Record<string,string> = {BREW_REFRESH:'Homebrew 目录刷新失败，安装未修改；请检查网络后重试。',BREW_CATALOG:'无法读取 Homebrew 目录，安装未修改；请重新检测。',BREW_CATALOG_CHANGED:'Homebrew 目录与目标版本不一致，安装未修改；请重新检测更新。'};
const names: Record<AgentId,string> = {codex:'Codex CLI',gemini:'Gemini CLI',claude:'Claude Code CLI',deepseek:'Deepseek Harness'};
function time(value: number) { return new Date(value).toLocaleString('zh-CN'); }

export function Workspace() {
  const [launch, setLaunch] = useState<Installation>();
  const [installs,setInstalls]=useState<CliInstall[]>([]);
  const [exported,setExported]=useState<ExportResult>();
  const [startup,setStartup]=useState<RecoveryReport>();
  const [report,setReport] = useState<DiscoveryReport>();
  const [selected,setSelected] = useState<Record<string,boolean>>({});
  const [targets,setTargets] = useState<Record<string,string>>({});
  const [pre,setPre] = useState<Partial<Record<AgentId,boolean>>>({});
  const [catalogs,setCatalogs] = useState<Record<string,VersionCatalog|undefined>>({});
  const [versionBusy,setVersionBusy] = useState<Record<string,boolean>>({});
  const [versionErrors,setVersionErrors] = useState<Record<string,string>>({});
  const [inspected,setInspected] = useState<UpdateTransaction>();
  const [plans,setPlans] = useState<UpdatePlan[]>([]);
  const [agentBusy,setAgentBusy] = useState<Partial<Record<AgentId,boolean>>>({});
  const [executingPlans,setExecutingPlans] = useState<Record<string,boolean>>({});
  const {progress,start,fail,channel}=useOperationProgress();
  const [scanBusy,setScanBusy] = useState(false);
  const [working,setWorking] = useState<Record<string,boolean>>({});
  const operations = useRef(new Set<string>());
  const anyWorking = Object.values(working).some(Boolean);
  const agentWorking = (agent: AgentId) => !!working[`agent:${agent}`];
  const [error,setError] = useState('');
  const [path,setPath] = useState('');
  const [extraPaths,setExtraPaths] = useState<string[]>([]);
  const sequences = useRef<Record<string,number>>({});
  const inFlight = useRef(false);
  const refreshHistory = useCallback(async () => {setPlans(await planHistory());setInstalls(await cliInstalls());},[]);
  const scan = useCallback(async (paths: string[] = extraPaths) => {
    if(inFlight.current) return;
    inFlight.current=true;setScanBusy(true);setError('');
    try {const found=await discoverAgents(paths);setReport(found);await refreshHistory();for(const i of found.instances.filter(i=>i.supported)) void versions(i.agentId,i.agentId==='deepseek',i.id);}
    catch(e) {setError(errorMessage(e));}
    finally {setScanBusy(false);inFlight.current=false;}
  },[extraPaths,refreshHistory]);
  const versions = useCallback(async (agent: AgentId, include: boolean, instanceId: string) => {
    const seq = (sequences.current[instanceId] ?? 0)+1;sequences.current[instanceId]=seq;
    setVersionBusy(v=>({...v,[instanceId]:true}));setVersionErrors(v=>({...v,[instanceId]:''}));
    setCatalogs(v=>({...v,[instanceId]:undefined}));
    try {const result = await listVersions(agent,include,instanceId);if(sequences.current[instanceId]===seq) setCatalogs(v=>({...v,[instanceId]:result}));}
    catch(e) {if(sequences.current[instanceId]===seq) setVersionErrors(v=>({...v,[instanceId]:errorMessage(e)}));}
    finally {if(sequences.current[instanceId]===seq) setVersionBusy(v=>({...v,[instanceId]:false}));}
  },[]);
  useEffect(()=>{void inspectInterrupted().then(result=>{setStartup(result);return scan();}).catch(e=>setError(errorMessage(e)));},[]);
  useEffect(()=>{const timer=setInterval(()=>{void refreshHistory().catch(e=>setError(errorMessage(e)));},1200);return()=>clearInterval(timer);},[refreshHistory]);
  useEffect(()=>{const listener=(e: KeyboardEvent)=>{if(e.metaKey&&e.key.toLowerCase()==='r'){e.preventDefault();void scan();}};window.addEventListener('keydown',listener);return()=>window.removeEventListener('keydown',listener);},[scan]);
  async function action(id: string, job:()=>Promise<unknown>, keys=[id]) {
    if(keys.some(key=>operations.current.has(key)))return;
    keys.forEach(key=>operations.current.add(key));
    setWorking(v=>({...v,...Object.fromEntries(keys.map(key=>[key,true]))}));setError('');
    try {await job();}catch(e){setError(errorMessage(e));}
    finally {keys.forEach(key=>operations.current.delete(key));setWorking(v=>({...v,...Object.fromEntries(keys.map(key=>[key,false]))}));await refreshHistory().catch(e=>setError(errorMessage(e)));}
  }
  const chosen = report?.instances.filter(i=>selected[i.id]&&i.supported) ?? [];
  const eligible = chosen.filter(i=>!agentWorking(i.agentId)&&!blockingTransaction(plans,i.id)&&((targets[i.id]??'latest')==='latest'?catalogs[i.id]?.state==='update_available':targets[i.id]!==i.version));
  function showPendingTask(tx: UpdateTransaction) {
    setInspected(tx);
    requestAnimationFrame(() => {
      const row=document.getElementById(`transaction-${tx.id}`);
      let parent=row?.parentElement;
      while(parent){if(parent instanceof HTMLDetailsElement)parent.open=true;parent=parent.parentElement;}
      row?.scrollIntoView({block:'center'});
      row?.focus({preventScroll:true});
    });
  }
  async function checkAgent(agent:AgentId) {
    if(inFlight.current)return;inFlight.current=true;setAgentBusy(v=>({...v,[agent]:true}));setError('');
    try {const found=await discoverAgents(extraPaths);setReport(found);await Promise.all(found.instances.filter(i=>i.agentId===agent&&i.supported).map(i=>versions(agent,agent==='deepseek'||!!pre[agent],i.id)));}
    catch(e){setError(errorMessage(e));}finally{inFlight.current=false;setAgentBusy(v=>({...v,[agent]:false}));}
  }
  async function updateInstances(instances:Installation[],latestOnly=false) {
    const first=instances[0];if(!first)return;
    const blocked=instances.map(i=>blockingTransaction(plans,i.id)).find(Boolean);
    if(blocked){showPendingTask(blocked);return;}
    await action(`update:${first.id}`,async()=>{
      for(const i of instances)start(i.id);
      let planId='';
      try {
        const plan=await createPlan(instances.map(i=>({instanceId:i.id,target:latestOnly?'latest':targets[i.id]??'latest',allowPrerelease:!!pre[i.agentId]})));
        for(const i of instances)start(i.id,'queued');
        planId=plan.id;setExecutingPlans(v=>({...v,[plan.id]:true}));await refreshHistory();
        await executePlan(plan.id,false,channel());
        const found=await discoverAgents(extraPaths);setReport(found);
        await Promise.all(found.instances.filter(i=>i.supported&&instances.some(old=>old.agentId===i.agentId)).map(i=>versions(i.agentId,i.agentId==='deepseek'||!!pre[i.agentId],i.id)));
      } catch(e){for(const i of instances)fail(i.id);throw e;}finally{setExecutingPlans(v=>({...v,[planId]:false}));}
    }, [...new Set(instances.map(i=>`agent:${i.agentId}`))]);
  }
  async function resume(plan:UpdatePlan) {
    await action(plan.id,async()=>{setExecutingPlans(v=>({...v,[plan.id]:true}));try{await executePlan(plan.id,plan.state==='paused',channel());await discoverAgents(extraPaths).then(setReport);}finally{setExecutingPlans(v=>({...v,[plan.id]:false}));}}, [...new Set(plan.transactions.map(tx=>`agent:${tx.installation.agentId}`))]);
  }
  function toggle(i:Installation,checked:boolean) {setSelected(v=>({...v,[i.id]:checked}));if(checked&&!catalogs[i.id]&&!versionBusy[i.id]) void versions(i.agentId,!!pre[i.agentId],i.id);}
  const planCompleted = (plan: UpdatePlan) => plan.transactions.every(tx=>!['queued','preflight','backing_up','prepared','installing','verifying','recovering'].includes(tx.stage));
  const renderTransaction = (planId: string, tx: UpdateTransaction) => <div id={`transaction-${tx.id}`} tabIndex={-1} className={`transaction stage-${tx.stage}`} key={tx.id}><div className="transaction-main"><strong>{names[tx.installation.agentId]}</strong><span className="version-pair">{tx.installation.version} <span aria-hidden="true">→</span> {tx.target.version}</span><span className="status" role="status">{stageNames[tx.stage]}</span></div><p className="path">{tx.installation.commandPath}</p>{executingPlans[planId]&&progress[tx.installation.id]?<OperationProgress progress={progress[tx.installation.id]}/>:tx.stage==='succeeded'?<OperationProgress progress={{...initialProgress(tx.installation.id),phase:'succeeded'}}/>:null}<div className="transaction-meta"><span>{tx.restorePointId?'升级前备份已保存':'安装前将备份配置'}</span>{tx.failureCode&&<span>原因：{failureMessages[tx.failureCode]??tx.failureCode}</span>}<button disabled={anyWorking} onClick={()=>void action('export',async()=>setExported(await exportTransaction(tx.id,'diagnostics')))}>导出脱敏日志</button></div>{['needs_recovery','recovery_failed'].includes(tx.stage)&&<p className="notice">更新未完成，请重新检测后重试；若命令不可用，请通过官方安装方式重新安装。</p>}</div>;
  return <>
    {launch&&<CliLaunch installation={launch} onClose={()=>setLaunch(undefined)}/>}
    {startup?.busy&&<p className="notice">其他进程正在处理任务，未修改其状态。结束后重新打开应用以检查未完成事务。</p>}
    <div className="section-heading workspace-heading"><div><p className="eyebrow">安装实例 / LOCAL TOOLS</p><h2>选择要更新的安装实例</h2></div><button onClick={()=>void scan()} disabled={scanBusy||anyWorking}>{scanBusy?<><Spinner size={12}/>正在扫描…</>:'↻ 重新扫描'}</button></div>
    {error&&<div className="error" role="alert">{error}<button onClick={()=>setError('')}>关闭</button></div>}
    <section className="tool-grid" aria-label="安装实例">
    {(['codex','claude','deepseek','gemini'] as AgentId[]).map(agent=><article key={agent} data-agent={agent} className="tool-card">
      <header><span className={`agent-symbol ${agent}`} aria-hidden="true">{agent==='codex'?'>_':agent==='claude'?'✳':agent==='deepseek'?'≈':'✦'}</span><div><h2>{names[agent]}</h2><span className="muted">{agent==='claude'?'npm / 原生 / Homebrew':'npm / 本机其他来源'}</span></div><button className="detect-button" disabled={agentWorking(agent)||scanBusy||!!agentBusy[agent]} onClick={()=>void checkAgent(agent)}>{agentBusy[agent]?<><Spinner size={12}/>检测中…</>:'检测更新'}</button></header>
      {report?.instances.filter(i=>i.agentId===agent).map(i=><div className={`instance ${selected[i.id]?'chosen':''} ${catalogs[i.id]?.state==='update_available'?'has-update':''}`} key={i.id}>
        <label className="instance-choice"><input type="checkbox" checked={!!selected[i.id]} disabled={!i.supported||agentWorking(agent)} onChange={e=>toggle(i,e.target.checked)}/><strong>{i.version?`v${i.version}`:'版本未验证'}</strong><span>{i.architecture} · {i.source} · {i.supported?'可更新':'仅查看'}</span></label>
        <p className="path" title={i.commandPath}>{i.commandPath}</p><details className="command-detail"><summary>实际命令与安装位置</summary><p className="path">{i.resolvedCommand}</p></details>
        <button className="open-cli-button" disabled={agentWorking(agent)||scanBusy||!!agentBusy[agent]} onClick={()=>setLaunch(i)}>打开</button>
        {i.supported&&blockingTransaction(plans,i.id)&&<button disabled={agentWorking(agent)||scanBusy} onClick={()=>showPendingTask(blockingTransaction(plans,i.id)!)}>查看待处理任务</button>}
        {i.supported&&!blockingTransaction(plans,i.id)&&catalogs[i.id]?.state==='update_available'&&progress[i.id]?.phase!=='succeeded'&&!versionBusy[i.id]&&<button className="primary" disabled={agentWorking(agent)||scanBusy||!!agentBusy[agent]||(agent==='deepseek'&&!pre.deepseek)} onClick={()=>void updateInstances([i],true)}>{agent==='deepseek'?'一键升级官方 latest':'一键升级最新稳定版'}</button>}
        {progress[i.id]&&<OperationProgress progress={progress[i.id]} label={`${names[agent]} 升级进度`}/>}
        {catalogs[i.id]&&<p className={catalogs[i.id]!.state==='update_available'?'version-alert':'muted'} role="status">官方{agent==='deepseek'?' latest':'稳定'}版本：{catalogs[i.id]!.latestStable} · {({update_available:'有新版本可升级',up_to_date:'已是最新版本',local_newer:'本机版本较新',unknown_version:'版本无法比较'})[catalogs[i.id]!.state]}{catalogs[i.id]!.versions.find(v=>v.version===catalogs[i.id]!.latestStable)?.prerelease?'（预发行版）':''}</p>}
        {versionBusy[i.id]&&<Skeleton lines={2} label="正在检测官方版本"/>}
        {versionErrors[i.id]&&<p className="notice" role="alert">{versionErrors[i.id]} · 请重新检测</p>}
        {!i.supported&&<p className="notice">{i.reason??'此安装来源暂不支持升级。'}</p>}
        {selected[i.id]&&<label className="target-label">目标版本<select aria-label={`${names[agent]} 目标版本 ${i.id}`} value={targets[i.id]??'latest'} disabled={versionBusy[i.id]||!catalogs[i.id]||agentWorking(agent)} onChange={e=>setTargets(v=>({...v,[i.id]:e.target.value}))}><option value="latest">最新稳定版{catalogs[i.id]?` · ${catalogs[i.id]!.latestStable}`:''}</option>{catalogs[i.id]?.versions.map(v=><option value={v.version} key={v.version}>{v.version}{v.prerelease?' · 预发行':''}</option>)}</select></label>}
      </div>)}
      {agent==='deepseek'&&<label><input type="checkbox" checked={!!pre.deepseek} disabled={agentWorking(agent)} onChange={e=>setPre(v=>({...v,deepseek:e.target.checked}))}/>允许官方预发行渠道（当前为 RC）</label>}
      {report&&!scanBusy&&!report.instances.some(i=>i.agentId===agent)&&<button className="primary" disabled={agentWorking(agent)||!!agentBusy[agent]||(agent==='deepseek'&&!pre.deepseek)} onClick={()=>void action('install',async()=>{start(`cli:${agent}`);try{await installCli(agent,!!pre[agent],channel());}catch(e){fail(`cli:${agent}`);throw e;}const found=await discoverAgents(extraPaths);setReport(found);for(const i of found.instances.filter(i=>i.agentId===agent&&i.supported))void versions(agent,!!pre[agent],i.id);}, [`agent:${agent}`])}>一键安装 {names[agent]}</button>}
      {progress[`cli:${agent}`]&&<div className="tool-install-progress"><OperationProgress progress={progress[`cli:${agent}`]} label={`${names[agent]} 安装进度`}/></div>}
      {!report?.instances.some(i=>i.agentId===agent)&&(scanBusy?<Skeleton lines={3} label="正在读取本机安装"/>:<p className="empty">未发现安装实例。安装到 ~/.local/bin，需要 Node.js 22.12+；终端 PATH 应包含该目录。</p>)}
      {report?.instances.filter(i=>i.agentId===agent&&selected[i.id]).map(i=><div className="version-controls" key={i.id}>{i.source==='user_npm'&&<label><input type="checkbox" checked={!!pre[agent]} disabled={agentWorking(agent)} onChange={e=>{setPre(v=>({...v,[agent]:e.target.checked}));setTargets(v=>({...v,[i.id]:'latest'}));for(const item of report.instances.filter(n=>n.agentId===agent&&selected[n.id])) void versions(agent,e.target.checked,item.id);}}/>显示预发行版</label>}{i.source!=='user_npm'&&<p className="muted">{i.source==='homebrew'?'此来源使用 Homebrew 最新稳定版；可能晚于官方原生渠道。':'使用 Claude 官方原生最新稳定版和架构制品清单。'}</p>}</div>)}
    </article>)}
    </section>
    {report?.warnings.map((w,index)=><p className="notice" key={index}>{w}</p>)}
    <details className="extra-path"><summary>补充工具命令路径</summary><form onSubmit={e=>{e.preventDefault();if(!path.trim())return;const next=[...new Set([...extraPaths,path.trim()])];setExtraPaths(next);setPath('');void scan(next);}}><label>绝对命令路径<input value={path} onChange={e=>setPath(e.target.value)} placeholder="例如 /Users/you/.npm/bin/codex" required/></label><button disabled={anyWorking||scanBusy}>添加并扫描</button></form>{extraPaths.map(p=><div className="path" key={p}>{p}<button onClick={()=>{const next=extraPaths.filter(v=>v!==p);setExtraPaths(next);void scan(next);}} disabled={anyWorking||scanBusy}>移除</button></div>)}</details>
    <p className="muted">不同工具可同时升级；共享安装目录的操作会依次执行。单项失败不影响其余工具。</p>
    <div className="plan-bar"><div><strong>已选择 {chosen.length} 项</strong></div><button className="primary" disabled={!eligible.length||scanBusy||eligible.some(i=>!catalogs[i.id]||versionBusy[i.id]||(i.agentId==='deepseek'&&!pre.deepseek))} onClick={()=>void updateInstances(eligible)}>{`同时升级所选工具${eligible.length?` · ${eligible.length} 项`:''}`}</button></div>
    <RecoveryNotice plans={plans} startup={startup} disabled={anyWorking} onInspect={showPendingTask}/>
    <TaskHistory revealId={inspected?.planId} items={plans} completed={planCompleted} title="更新任务" completedLabel="更新历史（含失败记录）">
      {plan=><article className="history-plan" key={plan.id}><div className="plan-heading"><time>{time(plan.createdAt)}</time><span>{plan.transactions.length} 项 · {{ready:'待执行',running:'同时升级中',paused:'执行中断',finished:'已结束',cancelled:'已取消'}[plan.state]??plan.state}</span></div><div className="queue-controls">{['ready','paused'].includes(plan.state)&&<button className="primary" disabled={plan.transactions.some(tx=>agentWorking(tx.installation.agentId))} onClick={()=>void resume(plan)}>执行剩余项</button>}{['ready','paused','running'].includes(plan.state)&&<button disabled={plan.cancelRequested} onClick={()=>{void cancelPlan(plan.id).then(refreshHistory).catch(e=>setError(errorMessage(e)));}}>{plan.cancelRequested?'已请求取消 · 等待安全边界':plan.state==='running'?'安全取消':'结束计划'}</button>}</div>{plan.transactions.map(tx=>renderTransaction(plan.id,tx))}</article>}
    </TaskHistory>
    <TaskHistory items={installs} completed={item=>!['prepared','installing'].includes(item.state)} title="CLI 安装记录" completedLabel="安装历史（含失败记录）">{item=><div className="instance" key={item.id}><strong>{names[item.agent]} · {item.version}</strong><p className="path">{item.command}</p><span>{item.state==='succeeded'?'安装完成':item.state==='restored'?'历史已结束':item.state==='prepared'?'尚未发布安装':'安装未完成 · 请重新检测'}</span>{item.failureCode&&<p className="notice">{item.failureCode}</p>}</div>}</TaskHistory>
    {exported&&<ConfirmDialog title="导出已保存" onClose={()=>setExported(undefined)}><p>{exported.sensitive?'导出包含完整恢复材料和敏感配置，请私下保管。':'日志仅包含工具、版本、阶段、时间和错误码，不含配置或本地路径。'}</p><p className="path">{exported.directory}</p><div className="dialog-actions"><button onClick={()=>setExported(undefined)}>关闭</button><button className="primary" onClick={()=>void revealExport(exported.id).catch(e=>setError(errorMessage(e)))}>在访达中打开</button></div></ConfirmDialog>}
  </>;
}
function ConfirmDialog({title,onClose,children}:{title:string;onClose:()=>void;children:React.ReactNode}) {
  const ref=useRef<HTMLDialogElement>(null);
  useEffect(()=>{ref.current?.showModal();},[]);
  return <dialog ref={ref} onCancel={onClose} onClose={onClose} aria-label={title}><h2>{title}</h2>{children}</dialog>;
}
