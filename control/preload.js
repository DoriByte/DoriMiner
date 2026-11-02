const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('control', {
  init: () => ipcRenderer.invoke('init'),
  status: () => ipcRenderer.invoke('status'),
  start: () => ipcRenderer.invoke('start'),
  stop: () => ipcRenderer.invoke('stop'),
  save: (config) => ipcRenderer.invoke('save', config),
  browseMiner: () => ipcRenderer.invoke('browse-miner'),
  view: (next) => ipcRenderer.invoke('view', next),
  elevate: () => ipcRenderer.invoke('elevate'),
  mineWithoutOc: () => ipcRenderer.invoke('mine-without-oc'),
  restoreCards: () => ipcRenderer.invoke('restore-cards'),
  onStatus: (fn) => ipcRenderer.on('status', (_e, s) => fn(s)),
  onConfig: (fn) => ipcRenderer.on('config', (_e, c) => fn(c)),
  remoteInfo: () => ipcRenderer.invoke('remote-info'),
  remoteServe: () => ipcRenderer.invoke('remote-serve'),
  remotePair: (name) => ipcRenderer.invoke('remote-pair', name),
  remoteRevoke: (id) => ipcRenderer.invoke('remote-revoke', id),
  benchPlan: (scope) => ipcRenderer.invoke('bench-plan', scope),
  benchStart: (scope, useDev) => ipcRenderer.invoke('bench-start', scope, useDev),
  benchCancel: () => ipcRenderer.invoke('bench-cancel'),
});
