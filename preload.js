const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('electronAPI', {
  getMediaStructure: () => ipcRenderer.invoke('get-media-structure'),
  saveMetadata: (filePath, data) => ipcRenderer.invoke('save-metadata', filePath, data)
})
