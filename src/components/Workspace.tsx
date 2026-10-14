import { useCallback, useEffect, useRef, useState } from 'react';
import { installCli, cliInstalls, rollbackCliInstall, type CliInstall, restoreDetails, exportTransaction, cleanupRestore, revealExport, type RestoreDetails, type ExportResult, inspectInterrupted, type RecoveryReport, cancelPlan, executePlan, createPlan, discoverAgents, errorMessage, listVersions, planHistory, previewRollback, rollbackUpdate, type AgentId, type DiscoveryReport, type Installation, type RollbackPreview, type Stage, type UpdatePlan, type UpdateTransaction, type VersionCatalog } from '../api/desktop';

export const stageNames: Record<Stage, string> = {queued:'等待执行',preflight:'检查更新条件',backing_up:'备份配置与旧版本',prepared:'恢复材料已就绪',installing:'正在安装',verifying:'验证更新结果',succeeded:'更新成功',preparation_failed:'准备失败 · 安装未修改',needs_recovery:'待恢复',recovering:'正在恢复',restored:'已恢复原版本',recovery_failed:'恢复失败 · 材料已保留',cancelled:'已取消'};
const names: Record<AgentId,string> = {codex:'Codex CLI',gemini:'Gemini CLI',claude:'Claude Code CLI',deepseek:'Deepseek Harness'};
const recoverable = (tx: UpdateTransaction) => !!tx.restorePointId && ['succeeded','needs_recovery','recovery_failed'].includes(tx.stage);
function time(value: number) { return new Date(value).toLocaleString('zh-CN'); }

export function Workspace() {
  const [installs,setInstalls]=useState<CliInstall[]>([]);
  const [material,setMaterial]=useState<RestoreDetails>();
  const [exportConfirm,setExportConfirm]=useState(false);
  const [cleanupConfirm,setCleanupConfirm]=useState(false);
  const [exported,setExported]=useState<ExportResult>();
  const [startup,setStartup]=useState<RecoveryReport>();
  const [report,setReport] = useState<DiscoveryReport>();
  const [selected,setSelected] = useState<Record<string,boolean>>({});
  const [targets,setTargets] = useState<Record<string,string>>({});
  const [pre,setPre] = useState<Partial<Record<AgentId,boolean>>>({});
  const [catalogs,setCatalogs] = useState<Record<string,VersionCatalog|undefined>>({});
  const [versionBusy,setVersionBusy] = useState<Record<string,boolean>>({});
  const [versionErrors,setVersionErrors] = useState<Record<string,string>>({});
  const [plans,setPlans] = useState<UpdatePlan[]>([]);
  const [confirm,setConfirm] = useState<UpdatePlan>();
  const [recovery,setRecovery] = useState<RollbackPreview>();
  const [scanBusy,setScanBusy] = useState(false);
  const [working,setWorking] = useState('');
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
  async function action(id: string, job:()=>Promise<unknown>) {
    if(inFlight.current) return;inFlight.current=true;setWorking(id);setError('');
    try {await job();}catch(e){setError(errorMessage(e));}
    finally {await refreshHistory().catch(e=>setError(errorMessage(e)));inFlight.current=false;setWorking('');}
  }
  const chosen = report?.instances.filter(i=>selected[i.id]&&i.supported) ?? [];
  async function prepare() {await action('plan',async()=>{setConfirm(await createPlan(chosen.map(i=>({instanceId:i.id,target:targets[i.id]??'latest',allowPrerelease:pre[i.agentId]??false}))));});}
  function toggle(i:Installation,checked:boolean) {setSelected(v=>({...v,[i.id]:checked}));if(checked&&!catalogs[i.id]&&!versionBusy[i.id]) void versions(i.agentId,!!pre[i.agentId],i.id);}
  return <>
    {startup?.busy&&<p className="notice">其他进程正在处理任务，未修改其状态。结束后重新打开应用以检查未完成事务。</p>}
    {!!startup?.items.length&&<div className="notice" role="status">发现 {startup.items.length} 项未完成或待恢复任务。没有自动重装或判定成功，请在更新记录中处理。{startup.items.filter(i=>!i.materialReady).map(i=><p key={i.transactionId}>恢复材料无法验证（{i.materialError}）。请保留应用数据、包旁保留目录与配置副本，按人工恢复说明处理。</p>)}</div>}
    <div className="section-heading workspace-heading"><div><p className="eyebrow">安装实例 / LOCAL TOOLS</p><h2>选择要更新的安装实例</h2></div><button onClick={()=>void scan()} disabled={scanBusy||!!working}>↻ {scanBusy?'正在扫描…':'重新扫描'}</button></div>
    {error&&<div className="error" role="alert">{error}<button onClick={()=>setError('')}>关闭</button></div>}
    <section className="tool-grid" aria-label="安装实例">
    {(['codex','claude','deepseek','gemini'] as AgentId[]).map(agent=><article key={agent} data-agent={agent} className="tool-card">
      <header><span className={`agent-symbol ${agent}`} aria-hidden="true">{agent==='codex'?'>_':agent==='claude'?'✳':agent==='deepseek'?'≈':'✦'}</span><div><h2>{names[agent]}</h2><span className="muted">{agent==='claude'?'npm / 原生 / Homebrew':'npm / 本机其他来源'}</span></div><span className="tool-count">{report?.instances.filter(i=>i.agentId===agent).length??'—'} 个实例</span></header>
      {report?.instances.filter(i=>i.agentId===agent).map(i=><div className={`instance ${selected[i.id]?'chosen':''}`} key={i.id}>
        <label className="instance-choice"><input type="checkbox" checked={!!selected[i.id]} disabled={!i.supported||!!working} onChange={e=>toggle(i,e.target.checked)}/><strong>{i.version?`v${i.version}`:'版本未验证'}</strong><span>{i.architecture} · {i.source} · {i.supported?'可更新':'仅查看'}</span></label>
        <p className="path" title={i.commandPath}>{i.commandPath}</p><details className="command-detail"><summary>实际命令与安装位置</summary><p className="path">{i.resolvedCommand}</p></details>
        {i.supported&&<button disabled={!!working||scanBusy||(agent==='deepseek'&&!pre.deepseek)} onClick={()=>void action('plan',async()=>{setConfirm(await createPlan([{instanceId:i.id,target:'latest',allowPrerelease:agent==='deepseek'&&!!pre[agent]}]));})}>{agent==='deepseek'?'一键升级官方 latest':'一键升级最新稳定版'}</button>}
        {catalogs[i.id]&&<p className="muted">官方{agent==='deepseek'?' latest':'稳定'}版本：{catalogs[i.id]!.latestStable} · {i.version===catalogs[i.id]!.latestStable?'已与此渠道一致':'可选择此渠道版本'}{catalogs[i.id]!.versions.find(v=>v.version===catalogs[i.id]!.latestStable)?.prerelease?'（预发行版）':''}</p>}
        {!i.supported&&<p className="notice">{i.reason??'此来源尚未支持可靠恢复。'}</p>}
        {selected[i.id]&&<label className="target-label">目标版本<select aria-label={`${names[agent]} 目标版本 ${i.id}`} value={targets[i.id]??'latest'} disabled={versionBusy[i.id]||!catalogs[i.id]||!!working} onChange={e=>setTargets(v=>({...v,[i.id]:e.target.value}))}><option value="latest">最新稳定版{catalogs[i.id]?` · ${catalogs[i.id]!.latestStable}`:''}</option>{catalogs[i.id]?.versions.map(v=><option value={v.version} key={v.version}>{v.version}{v.prerelease?' · 预发行':''}</option>)}</select></label>}
      </div>)}
      {agent==='deepseek'&&<label><input type="checkbox" checked={!!pre.deepseek} disabled={!!working} onChange={e=>setPre(v=>({...v,deepseek:e.target.checked}))}/>允许官方预发行渠道（当前为 RC）</label>}
      {report&&!scanBusy&&!report.instances.some(i=>i.agentId===agent)&&<button className="primary" disabled={!!working||(agent==='deepseek'&&!pre.deepseek)} onClick={()=>void action('install',async()=>{await installCli(agent,!!pre[agent]);const found=await discoverAgents(extraPaths);setReport(found);})}>一键安装 {names[agent]}</button>}
      {!report?.instances.some(i=>i.agentId===agent)&&<p className="empty">{scanBusy?'正在读取本机安装…':'未发现安装实例。安装到 ~/.local/bin，需要 Node.js 22.12+；终端 PATH 应包含该目录。'}</p>}
      {report?.instances.filter(i=>i.agentId===agent&&selected[i.id]).map(i=><div className="version-controls" key={i.id}>{i.source==='user_npm'&&<label><input type="checkbox" checked={!!pre[agent]} disabled={!!working} onChange={e=>{setPre(v=>({...v,[agent]:e.target.checked}));setTargets(v=>({...v,[i.id]:'latest'}));for(const item of report.instances.filter(n=>n.agentId===agent&&selected[n.id])) void versions(agent,e.target.checked,item.id);}}/>显示预发行版</label>}<button onClick={()=>void versions(agent,!!pre[agent],i.id)} disabled={versionBusy[i.id]||!!working}>{versionBusy[i.id]?'查询版本…':'刷新版本'}</button>{versionErrors[i.id]&&<p className="notice" role="alert">{versionErrors[i.id]} · 请重试查询</p>}{i.source!=='user_npm'&&<p className="muted">{i.source==='homebrew'?'此来源使用 Homebrew 最新稳定版；可能晚于官方原生渠道。':'使用 Claude 官方原生最新稳定版和架构制品清单。'}</p>}</div>)}
    </article>)}
    </section>
    {report?.warnings.map((w,index)=><p className="notice" key={index}>{w}</p>)}
    <details className="extra-path"><summary>补充工具命令路径</summary><form onSubmit={e=>{e.preventDefault();if(!path.trim())return;const next=[...new Set([...extraPaths,path.trim()])];setExtraPaths(next);setPath('');void scan(next);}}><label>绝对命令路径<input value={path} onChange={e=>setPath(e.target.value)} placeholder="例如 /Users/you/.npm/bin/codex" required/></label><button disabled={!!working||scanBusy}>添加并扫描</button></form>{extraPaths.map(p=><div className="path" key={p}>{p}<button onClick={()=>{const next=extraPaths.filter(v=>v!==p);setExtraPaths(next);void scan(next);}} disabled={!!working||scanBusy}>移除</button></div>)}</details>
    <div className="plan-bar"><div><strong>每次更新，都有恢复点</strong><p>配置与完整旧版本强制备份。恢复材料验证通过后才会修改安装。</p></div><button className="primary" disabled={!chosen.length||!!working||scanBusy||chosen.some(i=>!catalogs[i.id]||versionBusy[i.id])} onClick={()=>void prepare()}>{working==='plan'?'正在固定目标版本…':`查看更新计划${chosen.length?` · ${chosen.length} 项`:''}`}</button></div>
    <section className="history" aria-label="更新任务"><div className="section-heading"><div><p className="eyebrow">任务历史 / RESTORE POINTS</p><h2>逐项进度与恢复</h2></div><span>{plans.length} 个计划</span></div>
      {!plans.length&&<p className="empty">还没有更新任务。选择安装实例和目标版本开始。</p>}
      {plans.map(plan=><article className="history-plan" key={plan.id}><div className="plan-heading"><time>{time(plan.createdAt)}</time><span>{plan.transactions.length} 项 · {{ready:"待执行",running:"执行中",paused:"失败后暂停",finished:"已完成",cancelled:"已结束"}[plan.state]??plan.state}</span></div><div className="queue-controls">{["ready","paused"].includes(plan.state)&&<button className="primary" disabled={!!working} onClick={()=>void action(plan.id,()=>executePlan(plan.id,plan.state==='paused'))}>{plan.state==='paused'?'继续剩余项':'执行整个计划'}</button>}{["ready","paused","running"].includes(plan.state)&&<button disabled={plan.cancelRequested} onClick={()=>{void cancelPlan(plan.id).then(refreshHistory).catch(e=>setError(errorMessage(e)));}}>{plan.cancelRequested?'已请求取消 · 等待安全边界':plan.state==='running'?'安全取消':'结束计划'}</button>}{plan.state==='paused'&&<span>成功项保留。可回滚失败项、继续其余项或结束。</span>}</div>{plan.transactions.map(tx=><div className={`transaction stage-${tx.stage}`} key={tx.id}><div className="transaction-main"><strong>{names[tx.installation.agentId]}</strong><span className="version-pair">{tx.installation.version} <span aria-hidden="true">→</span> {tx.target.version}</span><span className="status" role="status">{stageNames[tx.stage]}</span></div><p className="path">{tx.installation.commandPath}</p><div className="transaction-meta"><span>{tx.restorePointId?'恢复点已保存':'安装前将准备恢复点'}</span>{tx.failureCode&&<span>原因：{tx.failureCode}</span>}<button disabled={!!working} onClick={()=>void action('export',async()=>setExported(await exportTransaction(tx.id,'diagnostics')))}>导出脱敏日志</button>{tx.restorePointId&&<button disabled={!!working} onClick={()=>void action('details',async()=>{setExportConfirm(false);setCleanupConfirm(false);setMaterial(await restoreDetails(tx.id));})}>查看恢复点</button>}{recoverable(tx)&&<button disabled={!!working} onClick={()=>void action('preview',async()=>setRecovery(await previewRollback(tx.id)))}>回滚至 {tx.installation.version}</button>}</div>{['needs_recovery','recovery_failed'].includes(tx.stage)&&<p className="notice">请先恢复此实例。恢复失败时保留材料，参照人工恢复说明处理后重试。</p>}</div>)}</article>)}
    </section>
    {!!installs.length&&<section className="history" aria-label="CLI 安装记录"><h2>新工具安装记录</h2>{installs.map(item=><div className="instance" key={item.id}><strong>{names[item.agent]} · {item.version}</strong><p className="path">{item.command}</p><span>{item.state==='succeeded'?'安装完成':item.state==='restored'?'已恢复为未安装':item.state==='prepared'?'尚未发布安装': '安装中断 · 请恢复'}</span>{item.failureCode&&<p className="notice">{item.failureCode}</p>}{['succeeded','installing','needs_recovery'].includes(item.state)&&<button disabled={!!working} onClick={()=>void action('remove',async()=>{await rollbackCliInstall(item.id);setReport(await discoverAgents(extraPaths));})}>恢复为未安装（保留当前配置）</button>}</div>)}</section>}
    {exported&&<ConfirmDialog title="导出已保存" onClose={()=>setExported(undefined)}><p>{exported.sensitive?'导出包含完整恢复材料和敏感配置，请私下保管。':'日志仅包含工具、版本、阶段、时间和错误码，不含配置或本地路径。'}</p><p className="path">{exported.directory}</p><div className="dialog-actions"><button onClick={()=>setExported(undefined)}>关闭</button><button className="primary" onClick={()=>void revealExport(exported.id).catch(e=>setError(errorMessage(e)))}>在访达中打开</button></div></ConfirmDialog>}
    {material&&<ConfirmDialog title="恢复点与保留管理" onClose={()=>setMaterial(undefined)}><p>{material.verified?`校验通过 · ${(material.bytes/1024/1024).toFixed(1)} MB`:`材料无法验证 · ${material.errorCode}`}</p><p className="path">配置根：{material.configRoot??'无法确认'}</p>{material.configPaths.map(p=><p className="path" key={p}>{p}</p>)}<p>{material.protectionReason??'已有更新的完整恢复点，可手动清理这一份。'}</p>{exportConfirm&&<p className="notice">完整导出包含旧安装、原配置和变化副本，可能包含登录令牌。确认后存到当前用户的私有导出目录。</p>}{cleanupConfirm&&<p className="notice">确认清理后，此次更新不再提供回滚。较新的完整恢复点仍会保留。</p>}<div className="dialog-actions"><button onClick={()=>setMaterial(undefined)}>关闭</button><button disabled={!material.verified||!!working} onClick={()=>{if(!exportConfirm){setExportConfirm(true);return;}const id=material.transactionId;setMaterial(undefined);void action('export',async()=>setExported(await exportTransaction(id,'restore',true)));}}>{exportConfirm?'确认导出敏感恢复材料':'导出完整恢复点'}</button><button disabled={!material.canDelete||!!working} onClick={()=>{if(!cleanupConfirm){setCleanupConfirm(true);return;}const id=material.transactionId;setMaterial(undefined);void action('cleanup',()=>cleanupRestore(id));}}>{cleanupConfirm?'确认清理':'清理此恢复点'}</button></div></ConfirmDialog>}
    {confirm&&<ConfirmDialog title="确认更新计划" onClose={()=>setConfirm(undefined)}><p>版本已固定。下方每项都将先验证工具占用、空间与权限，再备份完整旧版本和配置。</p>{confirm.transactions.map(tx=><div className="confirm-item" key={tx.id}><strong>{names[tx.installation.agentId]} · {tx.installation.version} → {tx.target.version}{tx.target.prerelease?'（预发行版）':''}</strong><p className="path">{tx.installation.commandPath}</p><p>备份范围：{tx.installation.agentId==='codex'?'config.toml、auth.json':tx.installation.agentId==='claude'?'.claude/settings.json、settings.local.json、.credentials.json 与 .claude.json':tx.installation.agentId==='deepseek'?'cordis.patch.yml、.credentials.yaml 与 profiles':'settings.json、oauth_creds.json、google_accounts.json、mcp-oauth-tokens.json'}及受控链接目标；{tx.installation.source==='user_npm'?'完整 npm 包与依赖':'完整原生版本目录与启动链接'}。</p><span className="muted">恢复材料：执行前准备，尚未验证</span></div>)}<p className="muted">备份仅保存在当前用户的应用数据目录。成功更新也保留回滚入口。</p><div className="dialog-actions"><button onClick={()=>setConfirm(undefined)}>返回任务列表</button><button className="primary" disabled={!!working} onClick={()=>{const plan=confirm;setConfirm(undefined);void action(plan.id,()=>executePlan(plan.id));}}>确认并顺序执行</button></div></ConfirmDialog>}
    {recovery&&<ConfirmDialog title={`恢复至 ${recovery.originalVersion}`} onClose={()=>setRecovery(undefined)}><p>将恢复该次更新前的安装、配置内容和权限，并核验实际恢复结果。</p>{recovery.configurationChanged&&<div className="notice">配置在更新后发生了变化。确认后会先保存当前受管配置副本，再恢复更新前配置。</div>}<p className="path">变化副本：{recovery.preservationLocation}</p>{recovery.restoresAbsentFiles&&<p>原先不存在的受管文件将恢复为缺失状态，无关新增文件保留。</p>}<div className="dialog-actions"><button onClick={()=>setRecovery(undefined)}>返回</button><button className="primary" disabled={!!working} onClick={()=>{const r=recovery;setRecovery(undefined);void action(r.transactionId,()=>rollbackUpdate(r.transactionId,r.configurationChanged));}}>确认{recovery.configurationChanged?'保存并恢复':'恢复'}</button></div></ConfirmDialog>}
  </>;
}
function ConfirmDialog({title,onClose,children}:{title:string;onClose:()=>void;children:React.ReactNode}) {
  const ref=useRef<HTMLDialogElement>(null);
  useEffect(()=>{ref.current?.showModal();},[]);
  return <dialog ref={ref} onCancel={onClose} onClose={onClose} aria-label={title}><h2>{title}</h2>{children}</dialog>;
}
