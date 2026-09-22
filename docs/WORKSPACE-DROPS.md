# Workspace card drops

Files dragged from Windows are copied only into the available workspace card under the final drop position. The active workspace is unchanged. Dropping outside cards or on other pages copies nothing. Busy, missing, and unreadable workspaces do not advertise a drop target. Existing files keep the copy-without-overwrite behavior of `library::add_file`.

The main window's native handler emits `workspace-drag` with a physical pointer position. On drop, it retains the source paths in Rust and sends an opaque random identifier. Home converts physical coordinates to CSS coordinates using the current device pixel ratio and hit-tests the card, including any overlay above it. It calls `add_dropped_files` with the identifier and the selected workspace ID, or no workspace to discard the drop.

Pending drops expire after 30 seconds and can be consumed only once. A newer drop replaces the previous pending drop, and a delayed response cannot consume the newer one. Paths from the frontend are never accepted. The copy command resolves the selected workspace again under the operation lock; there is no fallback to the active workspace. Leaving Home removes its event listener, including when subscription registration completes after unmount.

Regression coverage includes card selection, outside-card and missing-card drops, navigation away from Home, one-use/expired identifiers, busy operation rejection, actual destination file copying, and preservation of existing files and the active workspace. UI tests simulate native events; a manual Explorer-to-WebView2 drag at Windows scaling settings remains a separate native integration check.

The changes are source-only and require a new desktop build to appear in an installed app.
