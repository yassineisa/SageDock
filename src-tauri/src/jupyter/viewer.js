// Called only in the authenticated, loopback JupyterLab webview. This gives JupyterLab
// commands, not notebook pages, control of editor state; no native IPC is exposed.
async (request) => {
  if (location.origin !== request.origin) return;
  let timer;
  let expired = false;
  try {
    await Promise.race([
      (async () => {
        while (!window.jupyterapp) {
          if (expired) return;
          await new Promise((resolve) => setTimeout(resolve, 100));
        }
        const app = window.jupyterapp;
        await app.restored;
        if (expired) return;
        if (request.path) {
          await app.commands.execute("docmanager:open", { path: request.path });
        } else {
          // Reveal a launcher created by a previous Home click, if it remains open.
          const previous = window.sageDockLauncher;
          if (previous && !previous.isDisposed) {
            previous.content.cwd = "";
            app.shell.activateById(previous.id);
          } else {
            window.sageDockLauncher = await app.commands.execute("launcher:create", {
              cwd: "",
              activate: true,
            });
          }
        }
      })(),
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          expired = true;
          reject(new Error("JupyterLab readiness timeout"));
        }, 90000);
      }),
    ]);
  } catch {
    alert(
      "SageDock couldn't open that view. Your open notebooks have been kept. Try again, or open JupyterLab in your browser from Settings.",
    );
  } finally {
    clearTimeout(timer);
  }
};
