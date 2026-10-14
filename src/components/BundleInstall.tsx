import {useRef,useState} from 'react';
import {installApp,errorMessage,type BundleProduct} from '../api/desktop';
import {OperationProgress,useOperationProgress} from './OperationProgress';

export function BundleInstall({product,label,onInstalled,onBusyChange,disabled=false}:{product:BundleProduct;label:string;onInstalled?:()=>void|Promise<void>;onBusyChange?:(busy:boolean)=>void;disabled?:boolean}) {
 const [busy,setBusy]=useState(false);const [error,setError]=useState('');const [restarting,setRestarting]=useState(false);const active=useRef(false);
 const {progress,start,fail,channel}=useOperationProgress();const target=`app:${product}`;
 async function install(){
  if(active.current)return;active.current=true;setBusy(true);onBusyChange?.(true);setError('');start(target);let reboot=false;
  try {
   const installed=await installApp(product,channel());
   if(installed.state==='restarting'){reboot=true;setRestarting(true);return;}
   await onInstalled?.();
  } catch(e){fail(target);setError(errorMessage(e));}
  finally{if(!reboot){active.current=false;onBusyChange?.(false);}setBusy(false);}
 }
 return <div className="bundle-install"><button className="primary" disabled={disabled||busy||restarting||progress[target]?.phase==='succeeded'} onClick={()=>void install()}>{restarting?'正在安装并重启…':busy?'正在安装…':progress[target]?.phase==='succeeded'?'安装完成':label}</button>{progress[target]&&<OperationProgress progress={progress[target]}/>} {error&&<p className="notice" role="alert">{error}</p>}</div>;
}
