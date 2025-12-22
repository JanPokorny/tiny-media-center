const { app, BrowserWindow, ipcMain } = require("electron")
const path = require("path")
const fs = require("fs").promises

const MEDIA_ROOT = path.join(__dirname, "media")
const VIDEO_EXTENSIONS = new Set([".mp4", ".mkv", ".avi", ".mov", ".wmv", ".webm"])

async function getMediaStructure(currentPath = MEDIA_ROOT) {
  const structure = []
  const dirents = await fs.readdir(currentPath, { withFileTypes: true })

  for (const dirent of dirents) {
    const fullPath = path.join(currentPath, dirent.name)
    if (dirent.isDirectory()) {
      structure.push({
        name: dirent.name,
        path: path.relative(MEDIA_ROOT, fullPath),
        type: 'directory',
        children: await getMediaStructure(fullPath)
      })
    } else if (dirent.isFile()) {
      const ext = path.extname(dirent.name).toLowerCase()
      if (VIDEO_EXTENSIONS.has(ext)) {
        structure.push({
          name: dirent.name,
          path: path.relative(MEDIA_ROOT, fullPath),
          fullPath: fullPath,
          stem: path.parse(dirent.name).name,
          type: 'file'
        })
      }
    }
  }
  return structure
}

async function saveMetadata(event, filePath, data) {
  try {
    const fullPath = path.isAbsolute(filePath) ? filePath : path.join(MEDIA_ROOT, filePath)
    const parsedPath = path.parse(fullPath)
    const metadataPath = path.join(parsedPath.dir, parsedPath.name + ".tsc")
    
    await fs.writeFile(metadataPath, JSON.stringify(data))
    return { success: true }
  } catch (error) {
    console.error("Error saving metadata:", error)
    return { success: false, error: error.message }
  }
}

app.whenReady().then(() => {
  ipcMain.handle('get-media-structure', () => getMediaStructure())
  ipcMain.handle('save-metadata', saveMetadata)

  const win = new BrowserWindow({
    show: false,
    fullscreen: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: false,
      contextIsolation: true,
      webSecurity: false // Disable webSecurity to allow loading local files (file://)
    }
  })

  // In development, we load from localhost. In production, we load index.html from build.
  if (process.env.NODE_ENV === 'development') {
    win.loadURL('http://localhost:5173')
  } else {
    win.loadFile(path.join(__dirname, 'dist', 'index.html'))
  }

  // Open the DevTools.
  // win.webContents.openDevTools()

  win.show()
})