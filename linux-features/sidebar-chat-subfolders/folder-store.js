"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const STORE_VERSION = 1;

function storePath(env = process.env) {
  if (typeof env.ZODEX_CHAT_FOLDERS_FILE === "string" && env.ZODEX_CHAT_FOLDERS_FILE.trim()) {
    return path.resolve(env.ZODEX_CHAT_FOLDERS_FILE.trim());
  }
  const configHome = env.XDG_CONFIG_HOME || path.join(env.HOME || os.homedir(), ".config");
  return path.join(configHome, "zodex", "chat-folders.json");
}

function emptyStore() {
  return {
    version: STORE_VERSION,
    projects: {},
  };
}

function readStore(env = process.env) {
  const file = storePath(env);
  if (!fs.existsSync(file)) {
    return emptyStore();
  }
  try {
    const raw = fs.readFileSync(file, "utf8");
    const data = JSON.parse(raw);
    if (!data || typeof data !== "object" || data.version !== STORE_VERSION || typeof data.projects !== "object") {
      return emptyStore();
    }
    return data;
  } catch {
    return emptyStore();
  }
}

function writeStore(store, env = process.env) {
  const file = storePath(env);
  const dir = path.dirname(file);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  }
  const serialized = JSON.stringify(store, null, 2);
  const tempFile = `${file}.${Date.now()}.${Math.random().toString(36).slice(2)}.tmp`;
  fs.writeFileSync(tempFile, serialized, { mode: 0o600 });
  fs.renameSync(tempFile, file);
}

function listFolders(projectId, env = process.env) {
  const store = readStore(env);
  return store.projects[projectId]?.folders || [];
}

function createFolder(projectId, name, env = process.env) {
  if (!projectId || !name || !name.trim()) {
    throw new Error("projectId and valid folder name are required");
  }
  const store = readStore(env);
  if (!store.projects[projectId]) {
    store.projects[projectId] = { folders: [] };
  }
  const id = `fld_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;
  const folder = {
    id,
    name: name.trim(),
    collapsed: false,
    threadIds: [],
  };
  store.projects[projectId].folders.push(folder);
  writeStore(store, env);
  return folder;
}

function moveThread(projectId, threadId, targetFolderId, env = process.env) {
  if (!projectId || !threadId) {
    throw new Error("projectId and threadId are required");
  }
  const store = readStore(env);
  const project = store.projects[projectId];
  if (!project) return false;

  // Remove threadId from any current folder in this project
  for (const f of project.folders) {
    f.threadIds = f.threadIds.filter((t) => t !== threadId);
  }

  // If moving into a specific folder
  if (targetFolderId) {
    const target = project.folders.find((f) => f.id === targetFolderId);
    if (!target) return false;
    target.threadIds.push(threadId);
  }

  writeStore(store, env);
  return true;
}

function renameFolder(projectId, folderId, newName, env = process.env) {
  if (!projectId || !folderId || !newName || !newName.trim()) {
    throw new Error("projectId, folderId, and non-empty newName are required");
  }
  const store = readStore(env);
  const folder = store.projects[projectId]?.folders?.find((f) => f.id === folderId);
  if (!folder) return false;
  folder.name = newName.trim();
  writeStore(store, env);
  return true;
}

function deleteFolder(projectId, folderId, env = process.env) {
  const store = readStore(env);
  const project = store.projects[projectId];
  if (!project) return false;
  const initialCount = project.folders.length;
  project.folders = project.folders.filter((f) => f.id !== folderId);
  if (project.folders.length !== initialCount) {
    writeStore(store, env);
    return true;
  }
  return false;
}

function toggleFolderCollapse(projectId, folderId, env = process.env) {
  const store = readStore(env);
  const folder = store.projects[projectId]?.folders?.find((f) => f.id === folderId);
  if (!folder) return null;
  folder.collapsed = !folder.collapsed;
  writeStore(store, env);
  return folder.collapsed;
}

module.exports = {
  STORE_VERSION,
  createFolder,
  deleteFolder,
  emptyStore,
  listFolders,
  moveThread,
  readStore,
  renameFolder,
  storePath,
  toggleFolderCollapse,
  writeStore,
};
