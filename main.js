const { app, BrowserWindow, ipcMain } = require("electron")
const path = require("path")
const fs = require("fs").promises
const { spawn } = require("child_process");

const MEDIA_ROOT = path.join(__dirname, "media");
const VIDEO_EXTENSIONS = new Set([
  ".mp4",
  ".mkv",
  ".avi",
  ".mov",
  ".wmv",
  ".webm",
]);

let scriptProcess = null;

async function getMediaStructure(currentPath = MEDIA_ROOT) {
  const structure = [];
  const dirents = await fs.readdir(currentPath, { withFileTypes: true });

  for (const dirent of dirents) {
    const fullPath = path.join(currentPath, dirent.name);
    if (dirent.isDirectory()) {
      structure.push({
        name: dirent.name,
        path: path.relative(MEDIA_ROOT, fullPath),
        type: "directory",
        children: await getMediaStructure(fullPath),
      });
    } else if (dirent.isFile()) {
      const ext = path.extname(dirent.name).toLowerCase();
      if (VIDEO_EXTENSIONS.has(ext)) {
        structure.push({
          name: dirent.name,
          path: path.relative(MEDIA_ROOT, fullPath),
          fullPath: fullPath,
          stem: path.parse(dirent.name).name,
          type: "file",
        });
      } else {
        try {
          await fs.access(fullPath, fs.constants.X_OK);
          structure.push({
            name: dirent.name,
            path: path.relative(MEDIA_ROOT, fullPath),
            fullPath: fullPath,
            type: "script",
          });
        } catch (err) {
          // Not executable, or other access error, ignore for now
        }
      }
    }
  }
  return structure;
}

app.whenReady().then(() => {
  const win = new BrowserWindow({
    show: false,
    fullscreen: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      nodeIntegration: false,
      contextIsolation: true,
      webSecurity: false, // Disable webSecurity to allow loading local files (file://)
    },
  });

  ipcMain.handle("get-media-structure", () => getMediaStructure());

  ipcMain.on("run-script", (event, scriptPath) => {
    scriptProcess = spawn(path.join(MEDIA_ROOT, scriptPath), []);

    scriptProcess.stdout.on("data", (data) => {
      win.webContents.send("script-output", data.toString());
    });

    scriptProcess.on("close", (code) => {
      console.log(`script process closed with code ${code}`);
      win.webContents.send("script-finished");
      scriptProcess = null;
    });

    scriptProcess.on("error", (err) => {
      console.error("Failed to start script:", err);
      win.webContents.send("script-finished");
      scriptProcess = null;
    });
  });

  ipcMain.on("kill-script", () => {
    if (scriptProcess) {
      scriptProcess.kill();
      scriptProcess = null;
    }
  });

  // In development, we load from localhost. In production, we load index.html from build.
  if (process.env.NODE_ENV === "development") {
    win.loadURL("http://localhost:5173");
  } else {
    win.loadFile(path.join(__dirname, "dist", "index.html"));
  }

  // Open the DevTools.
  win.webContents.openDevTools();

  win.show();
});