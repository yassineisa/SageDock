import { createContext, useContext, useState, useRef, type ReactNode } from "react";
import { friendlyError } from "../lib/commands";
type Notice = ReturnType<typeof friendlyError>;
/** How long a success notice stays up before it clears itself. Errors are exempt, they
 * often carry a next step (a retry button, a reason), and hiding one on a timer would take
 * that away before the student has necessarily read it. */
const MESSAGE_LIFETIME_MS = 5000;
interface TaskState {
  busy: string | null;
  error: Notice | null;
  message: string | null;
  dismiss: () => void;
  run: <T>(label: string, action: () => Promise<T>) => Promise<T | undefined>;
  /** Shows a success notice outside of `run`, for events SageDock observes rather than
   * causes, such as a new file appearing in Downloads. Auto-dismisses the same way. */
  notify: (text: string) => void;
}
const Context = createContext<TaskState | null>(null);
/** Keeps operations across route changes. The ref closes the same-render double-click
 * window before React updates `busy`; failures always release the lock in `finally`. */
export function TaskProvider({ children }: { children: ReactNode }) {
  const [busy, setBusy] = useState<string | null>(null);
  const lock = useRef(false);
  const [error, setError] = useState<Notice | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const messageTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  function clearMessageTimer() {
    if (messageTimer.current) {
      clearTimeout(messageTimer.current);
      messageTimer.current = null;
    }
  }

  function showMessage(text: string) {
    setMessage(text);
    clearMessageTimer();
    messageTimer.current = setTimeout(() => {
      setMessage(null);
      messageTimer.current = null;
    }, MESSAGE_LIFETIME_MS);
  }

  async function run<T>(label: string, action: () => Promise<T>): Promise<T | undefined> {
    if (lock.current) return;
    lock.current = true;
    setBusy(label);
    setError(null);
    setMessage(null);
    clearMessageTimer();
    try {
      const result = await action();
      if (typeof result === "string") showMessage(result);
      return result;
    } catch (err) {
      setError(friendlyError(err));
    } finally {
      lock.current = false;
      setBusy(null);
    }
  }
  return (
    <Context.Provider
      value={{
        busy,
        error,
        message,
        run,
        notify: showMessage,
        dismiss: () => {
          setError(null);
          setMessage(null);
          clearMessageTimer();
        },
      }}
    >
      {children}
    </Context.Provider>
  );
}
export function useTask() {
  const context = useContext(Context);
  if (!context) throw new Error("TaskProvider missing");
  return context;
}
