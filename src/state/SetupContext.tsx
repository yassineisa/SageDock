import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, isActivePhase, type SetupSnapshot } from "../lib/commands";

/**
 * Watches the setup operation for the whole app.
 *
 * Setup used to be owned by Home: Home subscribed to `setup-progress` in an effect and
 * held the result in its own state. Navigating away unmounted that subscription, so every
 * event emitted while the user was elsewhere was lost with nothing to recover it from, and
 * coming back showed a disabled screen with no progress at all.
 *
 * This provider sits above the router, so it mounts once and stays mounted. Screens read
 * from it and never subscribe themselves, which makes navigating away from setup a no-op.
 *
 * Two rules keep a reconnecting screen honest:
 *
 * 1. **Never move backwards within an operation.** A snapshot is applied only if its `seq`
 *    is newer than what we hold. A query issued before an event can easily answer after
 *    it; without this, that late answer would erase the newer progress.
 * 2. **Always accept a different operation.** A new run starts its `seq` fresh, so a lower
 *    number from a different `operation_id` is newer information, not older.
 */
interface SetupState {
  snapshot: SetupSnapshot | null;
  /** Re-reads the authoritative state. Safe to call at any time. */
  refresh: () => Promise<void>;
  /** Starts setup. Returns once it has *started*, not once it has finished. */
  start: () => Promise<void>;
  /** Whether a start is in flight, so the button can be disabled without a global lock. */
  starting: boolean;
  /** Why the last start attempt was refused, if it was. */
  startError: string | null;
  /** Marks an interrupted run as seen. */
  acknowledgeInterruption: () => Promise<void>;
}

const Context = createContext<SetupState | null>(null);

/** Applies `next` only if it genuinely is newer. See the rules above. */
function isNewer(current: SetupSnapshot | null, next: SetupSnapshot): boolean {
  if (!current) return true;
  if (current.operation_id !== next.operation_id) return true;
  return next.seq > current.seq;
}

export function SetupProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<SetupSnapshot | null>(null);
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  // Mirrors `snapshot` for the event handler, which closes over its first render's value.
  // Without this the sequence check would always compare against `null` and let a stale
  // event through.
  const latest = useRef<SetupSnapshot | null>(null);

  const apply = useCallback((next: SetupSnapshot) => {
    if (!isNewer(latest.current, next)) return;
    latest.current = next;
    setSnapshot(next);
  }, []);

  const refresh = useCallback(async () => {
    try {
      apply(await commands.setupSnapshot());
    } catch {
      // Being unable to read the snapshot is not worth interrupting anyone over: the
      // event stream is the primary source and this is the catch-up path. Leaving the
      // last known state on screen beats replacing it with an error.
    }
  }, [apply]);

  // One subscription for the life of the app.
  useEffect(() => {
    const subscription = listen<SetupSnapshot>("setup-progress", (event) => apply(event.payload));
    // Asked for immediately as well as subscribed to: a run already in flight when this
    // mounts (after a reload, or a reconciled interruption from a previous launch) has no
    // event pending, and would otherwise stay invisible until something changed.
    void refresh();
    return () => {
      subscription.then((stop) => stop()).catch(() => {});
    };
  }, [apply, refresh]);

  // A safety net, not the mechanism. Events drive the UI; this catches the case where one
  // is dropped while a long stage is otherwise silent, so the screen cannot sit on a stale
  // snapshot indefinitely. It stops as soon as the operation reaches a terminal phase.
  useEffect(() => {
    if (!snapshot || !isActivePhase(snapshot.phase)) return;
    const timer = setInterval(() => void refresh(), 5000);
    return () => clearInterval(timer);
  }, [snapshot, refresh]);

  const start = useCallback(async () => {
    setStarting(true);
    setStartError(null);
    try {
      apply(await commands.runSetup());
    } catch (err) {
      // The backend refuses an overlapping run, which is a normal answer to a double
      // click rather than a failure of setup itself.
      const message =
        err && typeof err === "object" && "message" in err
          ? String((err as { message: unknown }).message)
          : "SageDock couldn't start setup. Try again in a moment.";
      setStartError(message);
    } finally {
      setStarting(false);
    }
  }, [apply]);

  const acknowledgeInterruption = useCallback(async () => {
    try {
      await commands.acknowledgeSetupInterruption();
    } catch {
      // Only affects whether the interruption is mentioned again next launch.
    }
    await refresh();
  }, [refresh]);

  return (
    <Context.Provider
      value={{ snapshot, refresh, start, starting, startError, acknowledgeInterruption }}
    >
      {children}
    </Context.Provider>
  );
}

export function useSetup() {
  const context = useContext(Context);
  if (!context) throw new Error("SetupProvider missing");
  return context;
}
