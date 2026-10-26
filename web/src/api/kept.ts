// Reads that several views share and that change only with the server and account (scope.ts), such as the contact
// lists (#402). Views asking at the same time share one request, a view opened again finds the answer, until a new
// scope starts or a view asks for it fresh (a retry). A failure is not kept. The request is not cancelled when a view
// is left, as another may wait for it: a view that left lets the answer go instead.
import { onNewScope } from './scope';

const kept = new Map<string, Promise<unknown>>();
onNewScope(() => kept.clear());

export function keptRead<T>(key: string, read: () => Promise<T>, fresh = false): Promise<T> {
  const held = kept.get(key) as Promise<T> | undefined;
  if (held && !fresh) return held;
  const promise = read();
  kept.set(key, promise);
  promise.catch(() => { if (kept.get(key) === promise) kept.delete(key); });
  return promise;
}
