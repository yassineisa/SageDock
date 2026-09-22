import type { DragEventHandler } from "react";
import { Icon, type IconName } from "./Icon";

export interface NotebookRowAction {
  key: string;
  /** Used as both the accessible name and the native tooltip, see WorkspaceCard for the
   * same `title`-as-tooltip convention used throughout SageDock. */
  label: string;
  icon: IconName;
  onClick: () => void;
  disabled?: boolean;
  /** Marks the one action that removes something, so it reads differently before it's used. */
  danger?: boolean;
}

/**
 * One row in a notebook list (Recently Opened, Downloads): an icon, a name, a subtitle, and
 * a trailing area that shows the date until the row is hovered or a child gains focus, at
 * which point it swaps to a row of small action buttons.
 *
 * Built as a `<div>` with the open action as its own inner `<button>`, not as one big
 * button, because the action buttons are real `<button>` elements too, nesting a button
 * inside a button is invalid HTML and Chromium silently breaks it. `:focus-within` keeps the
 * actions reachable by keyboard even though they only *appear* on hover for a mouse.
 */
export function NotebookRow({
  name,
  subtitle,
  modified,
  busy,
  draggable,
  onDragStart,
  onOpen,
  actions,
}: {
  name: string;
  subtitle: string;
  /** Unix seconds. */
  modified: number;
  busy: boolean;
  draggable?: boolean;
  onDragStart?: DragEventHandler<HTMLButtonElement>;
  onOpen: () => void;
  actions: NotebookRowAction[];
}) {
  return (
    <div className="list-row notebook-row">
      <button
        className="list-row-open"
        disabled={busy}
        draggable={draggable}
        onDragStart={onDragStart}
        onClick={onOpen}
      >
        <Icon name="document" size={16} />
        <span className="list-grow">
          <strong>{name}</strong>
          <small>{subtitle}</small>
        </span>
      </button>
      <span className="notebook-row-trailing">
        <time className="notebook-row-date">
          {new Date(modified * 1000).toLocaleDateString(undefined, {
            month: "short",
            day: "numeric",
          })}
        </time>
        <span className="notebook-row-actions">
          {actions.map((action) => (
            <button
              key={action.key}
              className={`notebook-row-action${action.danger ? " is-danger" : ""}`}
              disabled={busy || action.disabled}
              onClick={action.onClick}
              aria-label={action.label}
              title={action.label}
            >
              <Icon name={action.icon} size={14} />
            </button>
          ))}
        </span>
      </span>
    </div>
  );
}
