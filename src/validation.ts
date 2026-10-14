import { invoke } from '@tauri-apps/api/core';
import { checkDesktopApps, cliProjects, getSupport, type AppOverview } from './api/desktop';

async function denied(command: string, args?: Record<string, unknown>) {
  try { await invoke(command, args); return false; } catch { return true; }
}

export async function validateCliLaunch(overview: AppOverview) {
  try { await runCliLaunchValidation(overview); }
  catch (e) { await invoke('finish_cli_launch_validation', {checks: {[String(e)]:false}}); }
}

async function runCliLaunchValidation(overview: AppOverview) {
  const card = () => document.querySelector('[data-agent="codex"]')!;
  await readyButton('打开', card);
  const command = card().querySelector('.instance > .path')!.textContent!;
  const base = command.slice(0, command.indexOf('/.local/bin/'));
  const first = `${base}/first project`;
  const second = `${base}/中文 project ' " ; $(touch unsafe) \`touch unsafe\``;
  const dialog = () => document.querySelector<HTMLDialogElement>('.cli-launch[open]')!;
  async function input(value: string) {
    const field = dialog().querySelector<HTMLInputElement>('input')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(field, value);
    field.dispatchEvent(new Event('input', {bubbles: true}));
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  clickButton('打开', card());
  await until(() => !!dialog()?.querySelector('input'));
  await input('/missing-age24-project');
  clickButton('保存目录', dialog());
  await until(() => !!dialog().querySelector('[role="alert"]'));
  await input(first); clickButton('保存目录', dialog());
  await until(() => dialog()?.textContent?.includes('项目目录已保存') ?? false);
  await input(second); clickButton('保存目录', dialog());
  await until(() => dialog()?.textContent?.includes('项目目录已保存') ?? false);
  if ((await cliProjects()).length !== 1) throw new Error('Editing did not replace the recent project');
  clickButton('新增目录', dialog()); await input(first); clickButton('保存目录', dialog());
  await until(() => dialog()?.textContent?.includes('项目目录已保存') ?? false);
  clickButton('取消', dialog()); await until(() => !dialog());
  clickButton('打开', card()); await until(() => !!dialog()?.querySelector('input'));
  if (dialog().querySelector('input')!.value !== first || (await cliProjects()).length !== 2) throw new Error('Recent project persistence failed');
  await input(second);
  // Updating an existing entry deduplicates it and launches only the discovered fixture.
  clickButton('在此目录打开 CLI', dialog());
  await until(() => !dialog() || !!dialog().querySelector('[role="alert"]'), 45000);
  if (dialog()) throw new Error(dialog().textContent!);
  const paths = await cliProjects();
  if (paths.length !== 1 || paths[0] !== second) throw new Error('Launch recent project mismatch');
  const deepseekCard = () => document.querySelector('[data-agent="deepseek"]')!;
  await readyButton('打开', deepseekCard);
  for (const profile of ['web', 'my-tui']) {
    clickButton('打开', deepseekCard());
    await until(() => !!dialog()?.querySelector('[aria-label="Deepseek profile"]'));
    const field = dialog().querySelector<HTMLInputElement>('[aria-label="Deepseek profile"]')!;
    if (field.value !== 'web') throw new Error('Deepseek default profile is missing');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(field, profile);
    field.dispatchEvent(new Event('input', {bubbles:true}));
    await new Promise(resolve => setTimeout(resolve, 50));
    if (!dialog().querySelector('form')!.checkValidity()) throw new Error('Deepseek form rejected a valid profile');
    clickButton('在此目录打开 CLI', dialog());
    await until(() => !dialog() || !!dialog().querySelector('[role="alert"]'), 45000);
    if (dialog()) throw new Error(dialog().textContent!);
  }
  await invoke('finish_cli_launch_validation', {checks: {
    navigationWorks: await validateNavigation(),
    domainCall: (await getSupport('codex')).id === overview.agents[0].id,
    recentProjectsPersist: true, editReplacesPath: true, addProjectWorks: true, invalidProjectShowsError: true,
    terminalRequestCompleted: true,
    deepseekDefaultAndCustomProfiles: true,
    invalidInstanceDenied: await denied('open_cli', {instanceId:'/bin/sh',path:second,previousPath:null}),
    arbitraryCommandDenied: await denied('run_arbitrary_command'),
    arbitraryFileDenied: await denied('read_arbitrary_file', {path:'/etc/hosts'}),
  }});
}

// The release binary does not register these validation commands or grant their capability.
export async function validateDesktop(overview: AppOverview) {
  const agent = await getSupport('codex');
  const desktopChecks = await checkDesktopApps();
  await until(()=>document.querySelectorAll('[data-desktop-product]').length===3);
  const checks = {
    navigationWorks: await validateNavigation(),
    perToolDetection: document.querySelectorAll('[data-agent] .detect-button').length===4&&document.querySelectorAll('[data-desktop-product] .detect-button').length===3,
    desktopUpgradeVisibility: Array.from(document.querySelectorAll('[data-desktop-product]')).every(card=>{const check=desktopChecks.find(c=>c.product===card.getAttribute('data-desktop-product'));return !check||['not_installed','update_available'].includes(check.state)||!card.querySelector('.bundle-install');}),
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

async function until(predicate:()=>boolean,timeout=600000) {const limit=Date.now()+timeout;while(!predicate()){if(Date.now()>limit)throw new Error(`UI validation timed out: ${document.body.innerText.slice(-3000)}`);await new Promise(resolve=>setTimeout(resolve,100));}}
async function readyButton(text:string,scope:()=>ParentNode=()=>document){await until(()=>Array.from(scope().querySelectorAll<HTMLButtonElement>('button')).some(b=>b.textContent?.includes(text)&&!b.disabled));}
function clickButton(text:string,scope:ParentNode=document){const button=Array.from(scope.querySelectorAll('button')).find(b=>b.textContent?.includes(text));if(!button)throw new Error(`Missing UI action ${text}`);if(button.disabled)throw new Error(`Disabled UI action ${text}`);button.click();}
export async function validateWorkflow(_overview:AppOverview){
 const fixtureSelector='[data-agent="codex"] .instance-choice input:not(:disabled), [data-agent="gemini"] .instance-choice input:not(:disabled)';
 const fixtureChoices=()=>Array.from(document.querySelectorAll<HTMLInputElement>(fixtureSelector)).filter(input=>input.closest('.instance')?.querySelector('.path')?.textContent?.includes('/agentupdate-ui-'));
 await until(()=>fixtureChoices().length===2);
 const rendered=document.querySelectorAll('[data-agent]').length===4;
 for(const checkbox of fixtureChoices())checkbox.click();
 await until(()=>{const button=Array.from(document.querySelectorAll('button')).find(b=>b.textContent?.includes('安装所选版本'));return !!button&&!button.disabled;});
 const selectedBoth=document.querySelectorAll('.instance-choice input:checked').length===2;
 const navigationWorks=await validateNavigation()&&document.querySelectorAll('.instance-choice input:checked').length===2;
 const observed=new Set<string>();let confirmationSeen=false;
 const observer=new MutationObserver(()=>{if(document.querySelector('dialog[open]'))confirmationSeen=true;for(const p of document.querySelectorAll('[data-progress-phase]'))observed.add(p.getAttribute('data-progress-phase')!);});
 observer.observe(document.body,{subtree:true,childList:true,attributes:true});
 clickButton('安装所选版本');
 await until(()=>document.querySelectorAll('.stage-succeeded').length===2);
 await until(()=>fixtureChoices().every(input=>{const instance=input.closest('.instance')!;return instance.textContent?.includes('已是最新版本')&&!Array.from(instance.querySelectorAll('button')).some(b=>b.textContent?.includes('一键升级'));}));
 const noInstallConfirmation=!confirmationSeen&&!document.querySelector('dialog[open]');
 const progressObserved=observed.has('staging')&&observed.has('installing')&&observed.has('succeeded')&&document.querySelectorAll('progress').length>=2;
 const updatedButtonsHidden=true;
 observer.disconnect();
 const bothUpdated=true;
 for(let i=0;i<2;i++) {
  await readyButton('回滚至',()=>document.querySelector('.stage-succeeded')!);clickButton('回滚至',document.querySelector('.stage-succeeded')!);
  await until(()=>document.querySelector('dialog[open]')?.textContent?.includes('恢复至')??false);
  clickButton('确认恢复',document.querySelector('dialog[open]')!);
  await until(()=>document.querySelectorAll('.stage-restored').length===i+1);
 }
 const bothRestored=true;
 await readyButton('导出脱敏日志',()=>document.querySelector('.stage-restored')!);clickButton('导出脱敏日志',document.querySelector('.stage-restored')!);await until(()=>document.querySelector('dialog[open]')?.textContent?.includes('导出已保存')??false);
 const privateLogExported=document.querySelector('dialog[open]')?.textContent?.includes('不含配置或本地路径')??false;clickButton('关闭',document.querySelector('dialog[open]')!);
 await readyButton('查看恢复点',()=>document.querySelector('.stage-restored')!);clickButton('查看恢复点',document.querySelector('.stage-restored')!);await until(()=>document.querySelector('dialog[open]')?.textContent?.includes('恢复点与保留管理')??false);
 const backupProtected=Array.from(document.querySelectorAll<HTMLButtonElement>('dialog[open] button')).some(b=>b.textContent==='清理此恢复点'&&b.disabled);clickButton('关闭',document.querySelector('dialog[open]')!);
 await until(()=>document.querySelector('.release-panel')?.textContent?.includes('发现新版本')??false);
 const releasePrompt=document.querySelector('.release-panel')?.textContent?.includes('打开安装包下载')??false;
 document.querySelector<HTMLButtonElement>('[data-nav="apps"]')!.click();
 await until(()=>document.querySelector('[data-desktop-product="claude"]')?.textContent?.includes('发现新版本')??false);
 const appCard=document.querySelector('[data-desktop-product="claude"]')!;
 const appPhases=new Set<string>();let appConfirm=false;
 const appObserver=new MutationObserver(()=>{if(document.querySelector('dialog[open]'))appConfirm=true;for(const p of appCard.querySelectorAll('[data-progress-phase]'))appPhases.add(p.getAttribute('data-progress-phase')!);});
 appObserver.observe(document.body,{subtree:true,childList:true,attributes:true});
 clickButton('下载并升级',appCard);
 await until(()=>document.querySelector('[aria-label="应用安装记录"]')?.textContent?.includes('安装完成')??false);
 await until(()=>appCard.textContent?.includes('与公开稳定版一致')??false);
 const desktopDirectInstall=!appConfirm&&!appCard.querySelector('.bundle-install');
 const desktopProgress=appPhases.has('downloading')&&appPhases.has('staging')&&appPhases.has('installing');
 appObserver.disconnect();
 await readyButton('恢复旧应用',()=>document.querySelector('[aria-label="应用安装记录"]')!);clickButton('恢复旧应用',document.querySelector('[aria-label="应用安装记录"]')!);
 await until(()=>document.querySelector('[aria-label="应用安装记录"]')?.textContent?.includes('已恢复')??false);
 const desktopRestored=true;
 document.querySelector<HTMLButtonElement>('[data-nav="cli"]')!.click();
 await invoke('finish_workflow_validation',{checks:{navigationWorks,rendered,selectedBoth,noInstallConfirmation,progressObserved,updatedButtonsHidden,bothUpdated,bothRestored,privateLogExported,backupProtected,releasePrompt,desktopDirectInstall,desktopProgress,desktopRestored}});
}

async function validateNavigation() {
 const cards=Array.from(document.querySelectorAll('[data-agent]'));
 for(const view of ['apps','release','cli']) {
  document.querySelector<HTMLButtonElement>(`[data-nav="${view}"]`)!.click();
  await until(()=>document.querySelector(`[data-nav="${view}"]`)?.getAttribute('aria-current')==='page');
  const panel=document.querySelector<HTMLElement>(`[data-view="${view}"]`)!;
  if(panel.hidden||panel.getClientRects().length===0||document.querySelectorAll('[data-view]:not([hidden])').length!==1) return false;
  if(document.querySelector('main')!.scrollWidth>document.querySelector('main')!.clientWidth+1) return false;
 }
 return cards.every(card=>card.isConnected)&&document.querySelector('h1')?.textContent==='Agent 工作台';
}
