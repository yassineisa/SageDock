import { useEffect, useRef, type ReactNode } from "react";

/**
 * A modal confirmation, used for every destructive or irreversible action.
 *
 * Transient surface, so it takes the 8px radius and the only elevation in the app.
 * Cancel is focused on open and Escape cancels: the safe choice is always the default.
 */
export function ConfirmDialog({
  title,
  children,
  confirm,
  busy,
  dismissOnly = false,
  onConfirm,
  onCancel,
}: {
  title: string;
  children: ReactNode;
  confirm: string;
  busy?: boolean;
  /**
   * The dialog only reports something and has nothing to decide, so the single button
   * closes it. Offering "Cancel" beside "Close" would ask the user to choose between two
   * words for the same outcome.
   */
  dismissOnly?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby="confirm-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) onCancel();
      }}
    >
      <h2 id="confirm-title">{title}</h2>
      <div className="dialog-body">{children}</div>
      <div className="dialog-actions">
        {dismissOnly ? null : (
          <button className="btn" onClick={onCancel} disabled={busy} autoFocus>
            Cancel
          </button>
        )}
        <button
          className="btn btn-accent"
          onClick={onConfirm}
          disabled={busy}
          autoFocus={dismissOnly}
        >
          {busy ? "Working…" : confirm}
        </button>
      </div>
    </dialog>
  );
}
