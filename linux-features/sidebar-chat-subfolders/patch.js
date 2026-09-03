"use strict";

const MAIN_BUNDLE_PATTERN = /^main-[^.]+\.js$/;
const SIDEBAR_ASSET_PATTERN = /^app-initial-[^.]+\.js$/;
const SUBFOLDERS_RUNTIME_MARKER = "codex-linux-sidebar-chat-subfolders-runtime";
const SUBFOLDERS_IPC_MARKER = "codex-linux-sidebar-chat-subfolders-ipc";

function warn(message) {
  console.warn(`WARN: ${message} - skipping sidebar-chat-subfolders patch`);
}

function subfolderIpcRuntimeSource() {
  return [
    `/*${SUBFOLDERS_IPC_MARKER}*/`,
    `;(()=>{`,
    `if(typeof require!=="undefined"&&typeof electron!=="undefined"||typeof require!=="undefined"){`,
    `try{`,
    `const {ipcMain}=require("electron");`,
    `const fs=require("node:fs");`,
    `const os=require("node:os");`,
    `const path=require("node:path");`,
    `function getStorePath(){`,
    `return process.env.ZODEX_CHAT_FOLDERS_FILE||path.join(process.env.XDG_CONFIG_HOME||path.join(process.env.HOME||os.homedir(),".config"),"zodex","chat-folders.json");`,
    `}`,
    `function readStore(){`,
    `try{let f=getStorePath();if(fs.existsSync(f))return JSON.parse(fs.readFileSync(f,"utf8"));}catch{}return{version:1,projects:{}};`,
    `}`,
    `function writeStore(s){`,
    `try{let f=getStorePath();let d=path.dirname(f);if(!fs.existsSync(d))fs.mkdirSync(d,{recursive:true,mode:0o700});`,
    `let tmp=f+"."+Date.now()+".tmp";fs.writeFileSync(tmp,JSON.stringify(s,null,2),{mode:0o600});fs.renameSync(tmp,f);return true;}catch{return false;}`,
    `}`,
    `ipcMain.handle("zodex-chat-folders",async(event,action,payload)=>{`,
    `let store=readStore();let pId=payload?.projectId;if(!pId)return{error:"missing projectId"};`,
    `if(!store.projects[pId])store.projects[pId]={folders:[]};`,
    `let p=store.projects[pId];`,
    `switch(action){`,
    `case "list": return{folders:p.folders};`,
    `case "create": {`,
    `let f={id:"fld_"+Date.now().toString(36)+"_"+Math.random().toString(36).slice(2,7),name:(payload.name||"New Folder").trim(),collapsed:false,threadIds:[]};`,
    `p.folders.push(f);writeStore(store);return{folder:f};`,
    `}`,
    `case "move": {`,
    `for(let f of p.folders){f.threadIds=f.threadIds.filter(t=>t!==payload.threadId);}`,
    `if(payload.folderId){let t=p.folders.find(f=>f.id===payload.folderId);if(t)t.threadIds.push(payload.threadId);}`,
    `writeStore(store);return{success:true};`,
    `}`,
    `case "rename": {`,
    `let f=p.folders.find(f=>f.id===payload.folderId);if(f){f.name=(payload.name||f.name).trim();writeStore(store);return{folder:f};}return{error:"not found"};`,
    `}`,
    `case "delete": {`,
    `p.folders=p.folders.filter(f=>f.id!==payload.folderId);writeStore(store);return{success:true};`,
    `}`,
    `case "toggle_collapse": {`,
    `let f=p.folders.find(f=>f.id===payload.folderId);if(f){f.collapsed=!f.collapsed;writeStore(store);return{collapsed:f.collapsed};}return{error:"not found"};`,
    `}`,
    `default: return{error:"unknown action"};`,
    `}`,
    `});`,
    `}catch(e){console.warn("WARN: zodex-chat-folders ipc init failed",e);}`,
    `}`,
    `})();`,
  ].join("");
}

function subfolderWebviewRuntimeSource() {
  return [
    `/*${SUBFOLDERS_RUNTIME_MARKER}*/`,
    `;(()=>{`,
    `if(typeof window==="undefined")return;`,
    `window.__ZODEX_CHAT_SUBFOLDERS__={`,
    `async list(projectId){return window.electron?.ipcRenderer?.invoke("zodex-chat-folders","list",{projectId})||{folders:[]};},`,
    `async create(projectId,name){return window.electron?.ipcRenderer?.invoke("zodex-chat-folders","create",{projectId,name});},`,
    `async move(projectId,threadId,folderId){return window.electron?.ipcRenderer?.invoke("zodex-chat-folders","move",{projectId,threadId,folderId});},`,
    `async rename(projectId,folderId,name){return window.electron?.ipcRenderer?.invoke("zodex-chat-folders","rename",{projectId,folderId,name});},`,
    `async delete(projectId,folderId){return window.electron?.ipcRenderer?.invoke("zodex-chat-folders","delete",{projectId,folderId});},`,
    `async toggleCollapse(projectId,folderId){return window.electron?.ipcRenderer?.invoke("zodex-chat-folders","toggle_collapse",{projectId,folderId});},`,
    `};`,
    `function initSidebarSubfoldersObserver(){`,
    `if(document.getElementById("zodex-subfolders-style"))return;`,
    `let style=document.createElement("style");style.id="zodex-subfolders-style";`,
    `style.textContent=\`.zodex-folder-header{display:flex;align-items:center;justify-content:space-between;padding:4px 8px;margin:2px 0;font-size:12px;font-weight:600;color:var(--text-tertiary,#888);cursor:pointer;border-radius:6px;user-select:none;}\`,`,
    `\`.zodex-folder-header:hover{background:rgba(255,255,255,0.06);color:var(--text-primary,#fff);}\`,`,
    `\`.zodex-folder-add-btn{opacity:0;transition:opacity 0.15s;padding:2px 6px;border-radius:4px;cursor:pointer;font-size:12px;}\`,`,
    `\`.group\\/folder-row:hover .zodex-folder-add-btn{opacity:1;}\`,`,
    `\`.zodex-subfolder-items{display:flex;flex-direction:column;padding-left:10px;margin-left:4px;border-left:1px solid rgba(255,255,255,0.08);}\`;`,
    `document.head.appendChild(style);`,
    `let observer=new MutationObserver(()=>{`,
    `let projectRows=document.querySelectorAll(".group\\\\/folder-row");`,
    `for(let row of projectRows){`,
    `if(row.dataset.zodexBound)continue;`,
    `row.dataset.zodexBound="1";`,
    `let nameEl=row.querySelector(".text-fade-truncate");`,
    `let projectName=nameEl?.textContent?.trim()||"";`,
    `if(!projectName)continue;`,
    `let addBtn=document.createElement("button");`,
    `addBtn.className="zodex-folder-add-btn text-token-text-tertiary hover:text-token-text-primary";`,
    `addBtn.title="Create Subfolder";`,
    `addBtn.textContent="+";`,
    `addBtn.onclick=async(e)=>{`,
    `e.stopPropagation();`,
    `let folderName=prompt("Subfolder name for "+projectName+":");`,
    `if(folderName&&folderName.trim()){`,
    `await window.__ZODEX_CHAT_SUBFOLDERS__.create(projectName,folderName.trim());`,
    `window.location.reload();`,
    `}`,
    `};`,
    `row.appendChild(addBtn);`,
    `}`,
    `});`,
    `observer.observe(document.body,{childList:true,subtree:true});`,
    `}`,
    `if(document.readyState==="loading"){document.addEventListener("DOMContentLoaded",initSidebarSubfoldersObserver,{once:true});}else{initSidebarSubfoldersObserver();}`,
    `})();`,
  ].join("");
}

function applyMainBundlePatch(source) {
  try {
    if (typeof source !== "string") return source;
    if (source.includes(SUBFOLDERS_IPC_MARKER)) return source;
    return `${source}\n${subfolderIpcRuntimeSource()}\n`;
  } catch (error) {
    warn(`Main bundle patch error: ${error.message}`);
    return source;
  }
}

function applyWebviewAssetPatch(source) {
  try {
    if (typeof source !== "string") return source;
    if (source.includes(SUBFOLDERS_RUNTIME_MARKER)) return source;
    return `${source}\n${subfolderWebviewRuntimeSource()}\n`;
  } catch (error) {
    warn(`Webview asset patch error: ${error.message}`);
    return source;
  }
}

const descriptors = [
  {
    id: "sidebar-chat-subfolders-main",
    phase: "main-bundle",
    order: 20_810,
    ciPolicy: "optional",
    pattern: MAIN_BUNDLE_PATTERN,
    missingDescription: "main process bundle",
    skipDescription: "sidebar chat subfolders main ipc patch",
    apply: (source) => applyMainBundlePatch(source),
  },
  {
    id: "sidebar-chat-subfolders-webview",
    phase: "webview-asset",
    order: 20_811,
    ciPolicy: "optional",
    pattern: SIDEBAR_ASSET_PATTERN,
    missingDescription: "sidebar webview bundle",
    skipDescription: "sidebar chat subfolders webview runtime patch",
    apply: (source) => applyWebviewAssetPatch(source),
  },
];

module.exports = {
  MAIN_BUNDLE_PATTERN,
  SIDEBAR_ASSET_PATTERN,
  SUBFOLDERS_IPC_MARKER,
  SUBFOLDERS_RUNTIME_MARKER,
  applyMainBundlePatch,
  applyWebviewAssetPatch,
  descriptors,
  subfolderIpcRuntimeSource,
  subfolderWebviewRuntimeSource,
};
