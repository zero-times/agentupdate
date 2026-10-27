import { useEffect, useRef, useState, type ReactNode } from 'react';

// Only actionable operations stay expanded; terminal outcomes share bounded history.
export function TaskHistory<T extends { id: string }>({items, completed, title, completedLabel, revealId, children}: {
  items: T[];
  completed: (item: T) => boolean;
  title: string;
  completedLabel: string;
  revealId?: string;
  children: (item: T) => ReactNode;
}) {
  const [visible, setVisible] = useState(5);
  const history = useRef<HTMLDetailsElement>(null);
  useEffect(() => {if (revealId && history.current) history.current.open = true;}, [revealId]);
  const pending = items.filter(item => !completed(item));
  const saved = items.filter(completed);
  const limit = Math.max(visible, saved.findIndex(item => item.id === revealId) + 1);
  if (!items.length) return null;
  return <section className="history" aria-label={title}>
    {!!pending.length && <><h2>{title}</h2>{pending.map(item => <div key={item.id}>{children(item)}</div>)}</>}
    {!!saved.length && <details ref={history} className="recovery-history"><summary>{completedLabel} · {saved.length} 条</summary>{saved.slice(0, limit).map(item => <div key={item.id}>{children(item)}</div>)}{limit < saved.length && <button className="history-more" onClick={() => setVisible(limit + 5)}>再显示 5 条</button>}</details>}
  </section>;
}
