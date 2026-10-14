import {useCallback,useState} from 'react';
import {Channel,type OperationProgress as Progress,type ProgressPhase} from '../api/desktop';

const labels:Record<ProgressPhase,string> = {
 queued:'等待顺序执行',
 checking:'检查版本与安装条件',backing_up:'备份配置与旧版本',downloading:'下载官方制品',
 staging:'校验并准备新版本',installing:'安装新版本',verifying:'验证安装结果',
 restarting:'后台安装并重启',succeeded:'安装完成',failed:'操作未完成',cancelled:'已安全取消',
};
export function initialProgress(target:string):Progress {return {target,phase:'checking',downloadedBytes:0,totalBytes:null,errorCode:null};}
export function useOperationProgress() {
 const [progress,setProgress]=useState<Record<string,Progress>>({});
 const start=useCallback((target:string,phase:ProgressPhase='checking')=>setProgress(p=>({...p,[target]:{...initialProgress(target),phase}})),[]);
 const fail=useCallback((target:string)=>setProgress(p=>['succeeded','restarting'].includes(p[target]?.phase)?p:({...p,[target]:{...(p[target]??initialProgress(target)),phase:'failed'}})),[]);
 const channel=useCallback(()=>{const c=new Channel<Progress>();c.onmessage=value=>setProgress(p=>({...p,[value.target]:value}));return c;},[]);
 const clear=useCallback((target:string)=>setProgress(p=>{const next={...p};delete next[target];return next;}),[]);
 return {progress,start,fail,channel,clear};
}
const mb=(bytes:number)=>(bytes/1024/1024).toFixed(1)+' MB';
export function OperationProgress({progress,label='安装进度'}:{progress:Progress;label?:string}) {
 if(progress.phase==='succeeded') return null;
 const downloading=progress.phase==='downloading';
 const measured=downloading&&!!progress.totalBytes;
 const stopped=['failed','cancelled'].includes(progress.phase);
 const value=stopped||progress.phase==='queued'?0:measured?Math.min(1,progress.downloadedBytes/progress.totalBytes!):undefined;
 return <div className={`operation-progress phase-${progress.phase}`} data-progress-target={progress.target} data-progress-phase={progress.phase}>
  <div className="progress-heading"><span role="status">{labels[progress.phase]}</span><span>{measured?`${Math.floor(value!*100)}%`:''}</span></div>
  <progress aria-label={label} value={value} max={1}/>
  {downloading&&<p className="muted">{mb(progress.downloadedBytes)}{progress.totalBytes?` / ${mb(progress.totalBytes)}`:' · 正在下载，文件大小尚未确认'}</p>}
  {progress.phase==='restarting'&&<p className="muted">安装包已校验，旧应用已备份。后台进程完成安装后自动打开新版。</p>}
  {progress.errorCode&&<p className="muted">{progress.errorCode} · 请查看安装记录</p>}
 </div>;
}
