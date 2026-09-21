import { useState } from "react";
import { formatLastOpened, type WorkspaceView } from "../lib/commands";
import { Icon } from "./Icon";

/**
 * One workspace — an ordinary Windows folder for a course.
 *
 * The card shows the real path and offers to open it in File Explorer, because nothing
 * here is hidden inside the app. Availability is never signalled by colour alone: a folder
 * that has gone missing says so in words and disables the actions that would fail.
 *
 * Management actions are always visible rather than revealed on hover, so they are
 * reachable by keyboard and touch and discoverable at a glance.
 */
export function WorkspaceCard({
  workspace,
  busy,
  launchDisabled = false,
  onLaunch,
  onRename,
  onAddFiles,
  onReveal,
  onForget,
  canForget,
}: {
  workspace: WorkspaceView;
  busy: boolean;
  launchDisabled?: boolean;
  onLaunch: () => void;
  onRename: (name: string) => void;
  onAddFiles: () => void;
  onReveal: () => void;
  onForget: () => void;
  canForget: boolean;
}) {
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(workspace.name);

  const available = workspace.status === "available";

  const submitRename = () => {
    const trimmed = draft.trim();
    setRenaming(false);
    if (trimmed && trimmed !== workspace.name) onRename(trimmed);
  };

  return (
    <article className={`workspace-card${workspace.is_active ? " is-active" : ""}`}>
      <div className="ws-top">
        <span className="ws-icon">
          <Icon name="folder" size={16} />
        </span>
        <span className="ws-head">
          {/* The pencil sits directly after the name rather than in the action row, so it
              stays attached to what it renames and follows the name when it changes. */}
          <span className="ws-title">
            <h3>{workspace.name}</h3>
            <button
              className="ws-edit"
              disabled={busy}
              onClick={() => {
                setDraft(workspace.name);
                setRenaming(true);
              }}
              aria-label={`Rename ${workspace.name}`}
            >
              <Icon name="edit" size={12} />
            </button>
          </span>
          <span className="ws-meta">{formatLastOpened(workspace.last_opened)}</span>
        </span>
        {workspace.is_active ? <span className="ws-badge">In use</span> : null}
      </div>

      {renaming ? (
        <div className="ws-rename">
          <label className="field" htmlFor={`rename-${workspace.id}`}>
            New name for this workspace
            <input
              id={`rename-${workspace.id}`}
              className="input"
              value={draft}
              autoFocus
              disabled={busy}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submitRename();
                if (e.key === "Escape") setRenaming(false);
              }}
            />
          </label>
          <p className="small">This also renames the folder in Windows.</p>
          <div className="btn-row">
            <button
              className="btn btn-accent"
              disabled={busy || !draft.trim()}
              onClick={submitRename}
            >
              Save name
            </button>
            <button className="btn" disabled={busy} onClick={() => setRenaming(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <>
          <p className="ws-path">{workspace.path}</p>

          {workspace.status === "missing" ? (
            <p className="ws-note">
              <Icon name="warning" size={14} />
              <span>
                This folder isn't where SageDock expects it. It may have been moved, renamed, or be
                on a drive that isn't connected. Removing it from this list won't delete anything.
              </span>
            </p>
          ) : null}
          {workspace.status === "unreadable" ? (
            <p className="ws-note">
              <Icon name="warning" size={14} />
              <span>
                SageDock can't read this folder right now. If it's on a network or removable drive,
                make sure it's connected.
              </span>
            </p>
          ) : null}

          <div className="ws-actions">
            <button
              className="btn"
              disabled={busy || !available || launchDisabled}
              onClick={onLaunch}
              title={available ? undefined : "This folder needs to be available first"}
            >
              <Icon name="play" size={16} />
              Launch workspace
            </button>
            <button
              className="btn btn-subtle"
              disabled={busy || !available}
              onClick={onAddFiles}
              aria-label={`Add files to ${workspace.name}`}
              title={available ? undefined : "This folder needs to be available first"}
            >
              <Icon name="add" size={16} />
              Add files
            </button>
            <button
              className="btn btn-subtle"
              disabled={busy || !available}
              onClick={onReveal}
              aria-label={`Open the folder for ${workspace.name} in File Explorer`}
            >
              <Icon name="folderOpen" size={16} />
              Open folder
            </button>
            {canForget ? (
              <button
                className="btn btn-subtle"
                disabled={busy}
                onClick={onForget}
                aria-label={`Remove ${workspace.name} from this list`}
              >
                <Icon name="remove" size={16} />
                Remove
              </button>
            ) : null}
          </div>
        </>
      )}
    </article>
  );
}
