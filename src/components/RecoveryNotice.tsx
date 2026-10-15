import type { RecoveryReport, UpdatePlan, UpdateTransaction } from '../api/desktop';

const activeStages = new Set(['queued', 'preflight', 'backing_up', 'prepared', 'installing', 'verifying', 'needs_recovery', 'recovering', 'recovery_failed']);
export function blockingTransaction(plans: UpdatePlan[], instanceId: string) {
  return plans.flatMap(plan => plan.transactions).find(tx => tx.installation.id === instanceId && activeStages.has(tx.stage));
}

export function RecoveryNotice({ plans, startup, disabled, onInspect }: {
  plans: UpdatePlan[];
  startup?: RecoveryReport;
  disabled: boolean;
  onInspect: (transaction: UpdateTransaction) => void;
}) {
  const pending = plans.flatMap(plan => plan.transactions).filter(tx => ['needs_recovery', 'recovery_failed'].includes(tx.stage));
  if (!pending.length) return null;
  return <div className="notice" role="status" data-recovery-notice>
    <strong>有 {pending.length} 项更新需要先恢复旧版本</strong>
    <p>旧任务仍占用该工具，请先恢复，再重新检测并升级。</p>
    {pending.map(tx => {
      const material = startup?.items.find(item => item.transactionId === tx.id);
      return <div key={tx.id}>
        <span>{tx.installation.agentId === 'claude' ? 'Claude Code CLI' : tx.installation.agentId === 'codex' ? 'Codex CLI' : tx.installation.agentId === 'gemini' ? 'Gemini CLI' : 'Deepseek Harness'} · 恢复至 {tx.installation.version}</span>{' '}
        <button disabled={disabled} onClick={() => onInspect(tx)}>查看恢复任务</button>
        {material && !material.materialReady && <p>恢复材料无法验证（{material.materialError}）。请保留恢复材料，按人工恢复说明处理。</p>}
      </div>;
    })}
  </div>;
}
