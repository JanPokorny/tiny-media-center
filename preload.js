const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('electronAPI', {
  listMedia: () => ipcRenderer.invoke('list-media'),
  saveMetadata: (filePath, data) => ipcRenderer.invoke('save-metadata', filePath, data)
})
