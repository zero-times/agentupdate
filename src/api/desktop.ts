import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
export { Channel };

export type ProgressPhase = 'queued'|'checking'|'backing_up'|'downloading'|'staging'|'installing'|'verifying'|'restarting'|'succeeded'|'failed'|'cancelled';
export type OperationProgress = {target:string;phase:ProgressPhase;downloadedBytes:number;totalBytes:number|null;errorCode:string|null};
export type ProgressChannel = Channel<OperationProgress>;

export type AgentId = 'codex' | 'gemini' | 'claude' | 'deepseek';
export type AgentSupport = {
  id: AgentId;
  name: string;
  installationSource: 'user_npm';
  detectionState: 'not_scanned';
  updateEnabled: false;
  description: string;
};
export type AppOverview = {
  appVersion: string;
  platform: { os: 'macos'; architecture: string; minimumSystemVersion: string };
  agents: AgentSupport[];
  backup: { requiredBeforeUpdate: boolean; state: 'available' };
  storage: { state: 'available' };
  updater: { state: 'available' };
  appRelease: { state: 'not_checked'; source: string };
  validationMode: boolean;
  validationWorkflow: boolean;
};

// Browser previews do not fabricate local machine state.
export async function getOverview(): Promise<AppOverview> {
  if (!isTauri()) throw new Error('请在 AgentUpdate 桌面窗口中打开此页面。');
  return invoke<AppOverview>('get_app_overview');
}
export function getSupport(agentId: AgentId): Promise<AgentSupport> {
  return invoke<AgentSupport>('get_agent_support', { agentId });
}

export type Installation = {
  id: string; agentId: AgentId; commandPath: string; resolvedCommand: string;
  packageRoot: string | null; prefix: string | null; nodePath: string | null;
  version: string | null; architecture: string; source: string;
  supported: boolean; versionVerified: boolean; reason: string | null;
};
export type DiscoveryReport = { instances: Installation[]; warnings: string[] };
export type AvailableVersion = { version: string; prerelease: boolean; tarball: string; integrity: string };
export type VersionCatalog = { agentId: AgentId; latestStable: string; versions: AvailableVersion[]; state:'update_available'|'up_to_date'|'local_newer'|'unknown_version' };
export function discoverAgents(extraPaths: string[] = []): Promise<DiscoveryReport> {
  return invoke('discover_agents', { extraPaths });
}
export function listVersions(agentId: AgentId, includePrerelease = false, instanceId?: string): Promise<VersionCatalog> {
  return invoke('list_agent_versions', { agentId, includePrerelease, instanceId: instanceId ?? null });
}
export function selectVersion(agentId: AgentId, target: string, allowPrerelease = false): Promise<AvailableVersion> {
  return invoke('select_agent_version', { agentId, target, allowPrerelease });
}
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null && 'message' in error) return String(error.message);
  return String(error);
}

export type Stage = 'queued' | 'preflight' | 'backing_up' | 'prepared' | 'installing' | 'verifying' | 'succeeded' | 'preparation_failed' | 'needs_recovery' | 'recovering' | 'restored' | 'recovery_failed' | 'cancelled';
export type UpdateTransaction = { id: string; planId: string; ordinal: number; installation: Installation; target: AvailableVersion; stage: Stage; restorePointId: string | null; failureCode: string | null; createdAt: number; updatedAt: number };
export type UpdatePlan = { id: string; state: string; cancelRequested: boolean; createdAt: number; transactions: UpdateTransaction[] };
export type PlanRequest = { instanceId: string; target: string; allowPrerelease: boolean };
export function createPlan(requests: PlanRequest[]): Promise<UpdatePlan> { return invoke('create_update_plan', { requests }); }
export function planHistory(): Promise<UpdatePlan[]> { return invoke('get_plan_history'); }

export function executeUpdate(transactionId: string): Promise<UpdateTransaction> { return invoke('execute_update', { transactionId, onProgress:new Channel<OperationProgress>() }); }

export type RollbackPreview = { transactionId: string; originalVersion: string; configurationChanged: boolean; restoresAbsentFiles: boolean; preservationLocation: string };
export function previewRollback(transactionId: string): Promise<RollbackPreview> { return invoke('preview_rollback', { transactionId }); }
export function rollbackUpdate(transactionId: string, confirmChangedConfig = false): Promise<UpdateTransaction> { return invoke('rollback_update', { transactionId, confirmChangedConfig }); }

export function executePlan(planId: string, continueAfterFailure = false, onProgress?:ProgressChannel): Promise<UpdatePlan> { return invoke('execute_plan', { planId, continueAfterFailure, onProgress:onProgress??new Channel<OperationProgress>() }); }
export function cancelPlan(planId: string): Promise<UpdatePlan> { return invoke('cancel_plan', { planId }); }

export type RecoveryReport = {busy:boolean; items:{transactionId:string;stage:Stage;materialReady:boolean;materialError:string|null;currentVersion:string|null;installationError:string|null}[]};
export function inspectInterrupted(): Promise<RecoveryReport> { return invoke('inspect_interrupted_tasks'); }

export type RestoreDetails = {transactionId:string;restorePointId:string|null;verified:boolean;errorCode:string|null;bytes:number;configRoot:string|null;configPaths:string[];canDelete:boolean;protectionReason:string|null};
export type ExportResult = {id:string;directory:string;sensitive:boolean};
export function restoreDetails(transactionId:string):Promise<RestoreDetails> {return invoke('get_restore_details',{transactionId});}
export function exportTransaction(transactionId:string,kind:'diagnostics'|'restore',confirmedSensitive=false):Promise<ExportResult> {return invoke('export_transaction',{transactionId,kind,confirmedSensitive});}
export function cleanupRestore(transactionId:string):Promise<void> {return invoke('cleanup_restore',{transactionId});}
export function revealExport(exportId:string):Promise<void> {return invoke('reveal_export',{exportId});}

export type ReleaseCheck = {state:'not_checked'|'no_releases'|'update_available'|'up_to_date'|'local_newer'|'failed';currentVersion:string;latest:{version:string;tag:string;publishedAt:string;notes:string;url:string}|null;asset:{name:string;url:string}|null;checkedAt:number|null;successfulAt:number|null;cached:boolean;errorCode:string|null;retryAfter:number};
export function checkRelease(manual=false):Promise<ReleaseCheck> {return invoke('check_app_release',{manual});}
export function openRelease(asset=false):Promise<void> {return invoke('open_app_release',{asset});}

export type DesktopProduct = 'codex'|'claude'|'chatgpt';
export type DesktopAppCheck = {product:DesktopProduct;name:string;installed:{path:string;version:string|null;build:string|null;state:string}[];latestVersion:string|null;state:string;checkedAt:number;metadataSource:string;officialPage:string;downloadAvailable:boolean;errorCode:string|null;channelNote:string};
export function checkDesktopApps(product?:DesktopProduct):Promise<DesktopAppCheck[]> {return invoke('check_desktop_apps',{product:product??null});}
export function openDesktopChannel(product:DesktopProduct,download=false):Promise<void> {return invoke('open_desktop_app_channel',{product,download});}

export type CliInstall = {id:string;agent:AgentId;version:string;state:string;command:string;package:string;failureCode:string|null;createdAt:number};
export function installCli(agent:AgentId,allowPrerelease=false,onProgress?:ProgressChannel):Promise<CliInstall>{return invoke('install_cli',{agent,allowPrerelease,onProgress:onProgress??new Channel<OperationProgress>()});}
export function cliInstalls():Promise<CliInstall[]>{return invoke('cli_install_history');}
export function rollbackCliInstall(id:string):Promise<CliInstall>{return invoke('rollback_cli_install',{id});}
export type BundleProduct=DesktopProduct|'agentupdate';
export type AppInstall={id:string;product:BundleProduct;version:string;destination:string;state:string;hasBackup:boolean;failureCode:string|null;createdAt:number};
export function prepareAppInstall(product:BundleProduct):Promise<AppInstall>{return invoke('prepare_app_install',{product});}
export function installApp(product:BundleProduct,onProgress?:ProgressChannel):Promise<AppInstall>{return invoke('install_app',{product,onProgress:onProgress??new Channel<OperationProgress>()});}
export function finishAppInstall(id:string,onProgress?:ProgressChannel):Promise<AppInstall>{return invoke('finish_app_install',{id,onProgress:onProgress??new Channel<OperationProgress>()});}
export function appInstalls():Promise<AppInstall[]>{return invoke('app_install_history');}
export function rollbackAppInstall(id:string):Promise<AppInstall>{return invoke('rollback_app_install',{id});}
