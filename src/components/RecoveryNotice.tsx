import type { RecoveryReport, UpdatePlan, UpdateTransaction } from '../api/desktop';

const activeStages = new Set(['queued', 'preflight', 'backing_up', 'prepared', 'installing', 'verifying', 'recovering']);
export function blockingTransaction(plans: UpdatePlan[], instanceId: string) {
  return plans.flatMap(plan => plan.transactions).find(tx => tx.installation.id === instanceId && activeStages.has(tx.stage));
}

export function RecoveryNotice({ plans, startup, disabled, onInspect }: {
  plans: UpdatePlan[];
  startup?: RecoveryReport;
  disabled: boolean;
  onInspect: (transaction: UpdateTransaction) => void;
}) {
  const pending = plans.flatMap(plan => plan.transactions).filter(tx => ['needs_recovery','recovery_failed'].includes(tx.stage));
  if (!pending.length) return null;
  return <div className="notice" role="status" data-recovery-notice>
    <strong>有 {pending.length} 项更新未完成</strong>
    <p>请重新检测后重试升级；命令不可用时可通过官方渠道重新安装。</p>
    {pending.map(tx => {
      const material = startup?.items.find(item => item.transactionId === tx.id);
      return <div key={tx.id}>
        <span>{tx.installation.agentId === 'claude' ? 'Claude Code CLI' : tx.installation.agentId === 'codex' ? 'Codex CLI' : tx.installation.agentId === 'gemini' ? 'Gemini CLI' : 'Deepseek Harness'} · {tx.installation.version} → {tx.target.version}</span>{' '}
        <button disabled={disabled} onClick={() => onInspect(tx)}>查看失败记录</button>
        {material && !material.materialReady && <p>备份材料无法验证（{material.materialError}）。请保留诊断记录。</p>}
      </div>;
    })}
  </div>;
}
