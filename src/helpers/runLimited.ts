/**
 * Runs `task` over `items` with at most `limit` in flight (IPC fan-out for big playlists), in order.
 * `stop()` returning true ends the run early (e.g. the user cancelled while it was starting).
 * Resolves with each task's result, undefined for items never started.
 */
export async function runLimited<T, R>(
  items: readonly T[],
  limit: number,
  task: (item: T) => Promise<R>,
  stop: () => boolean = () => false,
): Promise<(R | undefined)[]> {
  const results: (R | undefined)[] = new Array(items.length).fill(undefined);
  let next = 0;
  async function worker() {
    while (next < items.length && !stop()) {
      const index = next++;
      try {
        results[index] = await task(items[index]);
      } catch {
        results[index] = undefined;
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(Math.max(1, limit), items.length) }, worker));
  return results;
}

/** In-flight cap for per-download engine calls started by one action. */
export const ENGINE_FANOUT = 8;

/**
 * A queue that runs at most `limit` tasks at once, in call order. Callers get their own task's promise,
 * so a burst of engine calls (cancel a 1000-item playlist) stays bounded without changing callers' flow.
 */
export function createLimiter(limit: number) {
  let running = 0;
  const waiting: (() => void)[] = [];
  return function schedule<R>(task: () => Promise<R>): Promise<R> {
    return new Promise<R>((resolve, reject) => {
      const start = () => {
        running++;
        task().then(resolve, reject).finally(() => {
          running--;
          waiting.shift()?.();
        });
      };
      if (running < Math.max(1, limit)) start();
      else waiting.push(start);
    });
  };
}
