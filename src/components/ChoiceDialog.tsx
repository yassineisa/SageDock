import { useEffect, useRef, type ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

export interface ChoiceOption {
  key: string;
  label: string;
  detail?: string;
  icon?: IconName;
  disabled?: boolean;
  /** Marks an option that replaces or discards something, styled apart from the others. */
  danger?: boolean;
}

/**
 * A modal that asks the student to pick one of several named things to do, rather than to
 * confirm or cancel a single one. Built alongside {@link ConfirmDialog} rather than folded
 * into it: a picker's options are the content, not a footer button, and each needs its own
 * detail line and disabled state.
 *
 * Cancel is focused on open and Escape cancels, matching every other dialog in the app.
 */
export function ChoiceDialog({
  title,
  description,
  options,
  busy,
  onChoose,
  onCancel,
}: {
  title: string;
  description?: ReactNode;
  options: ChoiceOption[];
  busy?: boolean;
  onChoose: (key: string) => void;
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
      aria-labelledby="choice-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) onCancel();
      }}
    >
      <h2 id="choice-title">{title}</h2>
      <div className="dialog-body">
        {description}
        <div className="list" style={{ marginTop: description ? "var(--sp-3)" : 0 }}>
          {options.map((option) => (
            <button
              key={option.key}
              className={`list-row${option.danger ? " is-danger" : ""}`}
              disabled={busy || option.disabled}
              onClick={() => onChoose(option.key)}
            >
              {option.icon && <Icon name={option.icon} size={16} />}
              <span className="list-grow">
                <strong>{option.label}</strong>
                {option.detail && <small>{option.detail}</small>}
              </span>
            </button>
          ))}
        </div>
      </div>
      <div className="dialog-actions">
        <button className="btn" onClick={onCancel} disabled={busy} autoFocus>
          Cancel
        </button>
      </div>
    </dialog>
  );
}
