const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('ipcRenderer', {
  invoke: (...args) => ipcRenderer.invoke(...args),
  send: (...args) => ipcRenderer.send(...args),
  on: (channel, listener) => {
    ipcRenderer.on(channel, listener)
    return () => ipcRenderer.removeListener(channel, listener)
  },
  removeAllListeners: (channel) => ipcRenderer.removeAllListeners(channel),
})
