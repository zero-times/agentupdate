/** Loading primitives for AGE-26: spinner + skeleton shimmer.
 *  All motion is disabled under prefers-reduced-motion by the global CSS rule. */

export function Spinner({ size = 14, label = '加载中' }: { size?: number; label?: string }) {
  return <span className="spinner" role="status" aria-label={label} style={{ width: size, height: size }} />;
}

export function Skeleton({ lines = 3, label }: { lines?: number; label?: string }) {
  return <div className="skeleton-block" role="status" aria-label={label ?? '内容加载中'}>
    {Array.from({ length: lines }, (_, i) => <span key={i} className="skeleton-line" style={{ width: `${94 - i * 14}%` }} />)}
  </div>;
}
