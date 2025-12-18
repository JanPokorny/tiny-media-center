const { app, BrowserWindow, ipcMain } = require("electron")
const path = require("path")
const fs = require("fs").promises

// Ensure this points to where your media files are
const MEDIA_DIR = path.join(__dirname, "media", "movies")
const VIDEO_EXTENSIONS = new Set([".mp4", ".mkv", ".avi", ".mov", ".wmv", ".webm"])

async function listMedia() {
  try {
    console.log("Listing media from:", MEDIA_DIR)
    const files = await fs.readdir(MEDIA_DIR)
    console.log("Found files:", files)
    const mediaFiles = files.filter(file => {
      const ext = path.extname(file).toLowerCase()
      return VIDEO_EXTENSIONS.has(ext)
    })
    console.log("Filtered media files:", mediaFiles)
    return mediaFiles.map(file => ({
      name: file,
      path: file, 
      fullPath: path.join(MEDIA_DIR, file),
      stem: path.parse(file).name
    }))
  } catch (error) {
    console.error("Error reading media directory:", error)
    return []
  }
}

async function saveMetadata(event, filePath, data) {
  try {
    // Determine the correct path for the metadata file
    // If filePath is absolute, use it. If relative, join with MEDIA_DIR.
    const fullPath = path.isAbsolute(filePath) ? filePath : path.join(MEDIA_DIR, filePath)
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
  ipcMain.handle('list-media', listMedia)
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