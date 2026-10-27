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
  // Show only the latest outcome per installation, so an eventual success clears old failures.
  const latest = new Map<string, UpdateTransaction>();
  for (const tx of plans.flatMap(plan => plan.transactions)) {
    const previous = latest.get(tx.installation.id);
    if (!previous || tx.createdAt > previous.createdAt) latest.set(tx.installation.id, tx);
  }
  const pending = [...latest.values()].filter(tx => ['needs_recovery','recovery_failed'].includes(tx.stage));
  if (!pending.length) return null;
  return <details className="recovery-history failure-history" data-recovery-notice>
    <summary>最近更新未完成 · {pending.length} 个安装实例</summary>
    <p>请重新检测后重试升级；命令不可用时可通过官方渠道重新安装。</p>
    {pending.slice(0,5).map(tx => {
      const material = startup?.items.find(item => item.transactionId === tx.id);
      return <div key={tx.id}>
        <span>{tx.installation.agentId === 'claude' ? 'Claude Code CLI' : tx.installation.agentId === 'codex' ? 'Codex CLI' : tx.installation.agentId === 'gemini' ? 'Gemini CLI' : 'Deepseek Harness'} · {tx.installation.version} → {tx.target.version}</span>{' '}
        <button disabled={disabled} onClick={() => onInspect(tx)}>查看失败记录</button>
        {material && !material.materialReady && <p>备份材料无法验证（{material.materialError}）。请保留诊断记录。</p>}
      </div>;
    })}
    {pending.length > 5 && <p className="muted">其余失败详情可在更新历史中查看。</p>}
  </details>;
}
