import type { ReactNode } from 'react';

// Successful installs stay out of the panel. Recovery remains available on demand.
export function RecoveryHistory<T extends { id: string }>({items, completed, title, recoveryLabel, children}: {
  items: T[];
  completed: (item: T) => boolean;
  title: string;
  recoveryLabel: string;
  children: (item: T) => ReactNode;
}) {
  const pending = items.filter(item => !completed(item));
  const saved = items.filter(completed);
  if (!items.length) return null;
  return <section className="history" aria-label={title}>
    {!!pending.length && <><h2>{title}</h2>{pending.map(item => <div key={item.id}>{children(item)}</div>)}</>}
    {!!saved.length && <details className="recovery-history"><summary>{recoveryLabel}</summary>{saved.map(item => <div key={item.id}>{children(item)}</div>)}</details>}
  </section>;
}
