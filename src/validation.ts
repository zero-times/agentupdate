import { invoke } from '@tauri-apps/api/core';
import { checkDesktopApps, getSupport, type AppOverview } from './api/desktop';

async function denied(command: string, args?: Record<string, unknown>) {
  try { await invoke(command, args); return false; } catch { return true; }
}

// The release binary does not register these validation commands or grant their capability.
export async function validateDesktop(overview: AppOverview) {
  const agent = await getSupport('codex');
  const desktopChecks = await checkDesktopApps();
  await until(()=>document.querySelectorAll('[data-desktop-product]').length===3);
  const checks = {
    installCommandsReady: Array.isArray(await invoke('cli_install_history'))&&Array.isArray(await invoke('app_install_history')),
    invalidInstallAgentDenied: await denied('install_cli',{agent:'/bin/sh',allowPrerelease:false}),
    invalidInstallProductDenied: await denied('prepare_app_install',{product:'arbitrary'}),
    domainCall: agent.name === 'Codex CLI' && agent.id === overview.agents[0]?.id,
    invalidAgentDenied: await denied('get_agent_support', { agentId: '/usr/bin/env' }),
    restrictedCommandDenied: await denied('restricted_probe'),
    arbitraryCommandDenied: await denied('run_arbitrary_command', { executable: '/bin/sh' }),
    arbitraryFileDenied: await denied('read_arbitrary_file', { path: '/etc/hosts' }),
    shellPluginDenied: await denied('plugin:shell|execute', { program: '/usr/bin/true' }),
    filesystemPluginDenied: await denied('plugin:fs|read_text_file', { path: '/etc/hosts' }),
    desktopProductsChecked: desktopChecks.length===3
      && new Set(desktopChecks.map(check=>check.product)).size===3
      && desktopChecks.every(check=>check.checkedAt>0 && (check.errorCode?check.state==='failed'&&check.latestVersion===null:!!check.latestVersion)),
    invalidDesktopProductDenied: await denied('open_desktop_app_channel', {product:'arbitrary',download:false}),
    desktopRendered: document.querySelectorAll('[data-desktop-product]').length===3,
    rendered: document.querySelector('h1')?.textContent === 'Agent 工作台'
      && document.querySelectorAll('[data-agent]').length === 4,
  };
  await invoke('finish_desktop_validation', { checks });
}

async function until(predicate:()=>boolean,timeout=240000) {const limit=Date.now()+timeout;while(!predicate()){if(Date.now()>limit)throw new Error(`UI validation timed out: ${document.body.innerText.slice(-3000)}`);await new Promise(resolve=>setTimeout(resolve,100));}}
function clickButton(text:string,scope:ParentNode=document){const button=Array.from(scope.querySelectorAll('button')).find(b=>b.textContent?.includes(text));if(!button)throw new Error(`Missing UI action ${text}`);if(button.disabled)throw new Error(`Disabled UI action ${text}`);button.click();}
export async function validateWorkflow(_overview:AppOverview){
 const fixtureSelector='[data-agent="codex"] .instance-choice input:not(:disabled), [data-agent="gemini"] .instance-choice input:not(:disabled)';
 await until(()=>document.querySelectorAll(fixtureSelector).length===2);
 const rendered=document.querySelectorAll('[data-agent]').length===4;
 for(const checkbox of document.querySelectorAll<HTMLInputElement>(fixtureSelector))checkbox.click();
 await until(()=>{const button=Array.from(document.querySelectorAll('button')).find(b=>b.textContent?.includes('查看更新计划'));return !!button&&!button.disabled;});
 const selectedBoth=document.querySelectorAll('.instance-choice input:checked').length===2;
 clickButton('查看更新计划');await until(()=>document.querySelector('dialog[open]')?.textContent?.includes('确认更新计划')??false);
 const planConfirmed=document.querySelectorAll('dialog[open] .confirm-item').length===2;
 clickButton('确认并顺序执行',document.querySelector('dialog[open]')!);
 await until(()=>document.querySelectorAll('.stage-succeeded').length===2);
 const bothUpdated=true;
 for(let i=0;i<2;i++) {
  clickButton('回滚至',document.querySelector('.stage-succeeded')!);
  await until(()=>document.querySelector('dialog[open]')?.textContent?.includes('恢复至')??false);
  clickButton('确认恢复',document.querySelector('dialog[open]')!);
  await until(()=>document.querySelectorAll('.stage-restored').length===i+1);
 }
 const bothRestored=true;
 clickButton('导出脱敏日志',document.querySelector('.stage-restored')!);await until(()=>document.querySelector('dialog[open]')?.textContent?.includes('导出已保存')??false);
 const privateLogExported=document.querySelector('dialog[open]')?.textContent?.includes('不含配置或本地路径')??false;clickButton('关闭',document.querySelector('dialog[open]')!);
 clickButton('查看恢复点',document.querySelector('.stage-restored')!);await until(()=>document.querySelector('dialog[open]')?.textContent?.includes('恢复点与保留管理')??false);
 const backupProtected=Array.from(document.querySelectorAll<HTMLButtonElement>('dialog[open] button')).some(b=>b.textContent==='清理此恢复点'&&b.disabled);clickButton('关闭',document.querySelector('dialog[open]')!);
 await until(()=>document.querySelector('.release-panel')?.textContent?.includes('发现新版本')??false);
 const releasePrompt=document.querySelector('.release-panel')?.textContent?.includes('打开安装包下载')??false;
 await invoke('finish_workflow_validation',{checks:{rendered,selectedBoth,planConfirmed,bothUpdated,bothRestored,privateLogExported,backupProtected,releasePrompt}});
}
