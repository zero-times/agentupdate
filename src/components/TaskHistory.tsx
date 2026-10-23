import type { ReactNode } from 'react';

// Completed operations remain available as read-only history.
export function TaskHistory<T extends { id: string }>({items, completed, title, completedLabel, children}: {
  items: T[];
  completed: (item: T) => boolean;
  title: string;
  completedLabel: string;
  children: (item: T) => ReactNode;
}) {
  const pending = items.filter(item => !completed(item));
  const saved = items.filter(completed);
  if (!items.length) return null;
  return <section className="history" aria-label={title}>
    {!!pending.length && <><h2>{title}</h2>{pending.map(item => <div key={item.id}>{children(item)}</div>)}</>}
    {!!saved.length && <details className="recovery-history"><summary>{completedLabel}</summary>{saved.map(item => <div key={item.id}>{children(item)}</div>)}</details>}
  </section>;
}
