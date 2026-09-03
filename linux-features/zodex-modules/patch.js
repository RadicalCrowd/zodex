"use strict";

const HANDLER_NAME = "zodex-module-status";
const RUNTIME_VERSION = "zodex-module-warning-v1";

function warn(message, patchName) {
  console.warn(`WARN: ${message} - skipping ${patchName}`);
}

function mainBundleHelpers() {
  return [
    `function zodexConfigFsModule(){return require(\`node:fs\`)}`,
    `function zodexConfigPathModule(){return require(\`node:path\`)}`,
    `function zodexConfigModulePath(){let p=zodexConfigPathModule(),fs=zodexConfigFsModule(),e=process.env.CODEX_LINUX_FEATURES_DIR;if(typeof e===\`string\`&&e.trim()){let target=p.join(e.trim(),\`zodex-modules\`,\`config.js\`);if(fs.existsSync(target))return target;throw Error(\`CODEX_LINUX_FEATURES_DIR is invalid: \`+e)}let app=process.env.CODEX_LINUX_APP_DIR;if(typeof app===\`string\`&&app.trim()){let target=p.join(app.trim(),\`.codex-linux\`,\`features\`,\`zodex-modules\`,\`config.js\`);if(fs.existsSync(target))return target}let res=process.resourcesPath;if(typeof res===\`string\`&&res.trim()){let target=p.join(res.trim(),\`..\`,\`.codex-linux\`,\`features\`,\`zodex-modules\`,\`config.js\`);if(fs.existsSync(target))return target}return null}`,
    `function zodexConfigModule(){let target=zodexConfigModulePath();return target?require(target):null}`,
    `function zodexModuleStatus(){try{let m=zodexConfigModule();if(!m){let cf=process.env.ZODEX_CONFIG_FILE;if(typeof cf===\`string\`&&cf.trim())return{ok:!1,state:\`invalid\`,file:cf.trim(),error:\`Zodex module configuration helper is unavailable\`,revision:\`invalid-helper\`,connections:[]};return{ok:!0,state:\`missing\`,file:null,revision:\`missing\`,connections:[]}}let r=m.readConfig(process.env);if(r.state!==\`valid\`)return{ok:r.state!==\`invalid\`,state:r.state,file:r.file,error:r.error,revision:r.revision,connections:[]};return{ok:!0,state:\`valid\`,file:r.file,revision:r.revision,connections:m.brokerConnectionStates(r.config)}}catch(e){return{ok:!1,state:\`invalid\`,file:null,error:String(e?.message||e),revision:\`invalid-runtime\`,connections:[]}}}`,
  ].join("");
}

function applyMainBundlePatch(source) {
  if (source.includes(`"${HANDLER_NAME}":async`)) return source;
  const needle = `"native-desktop-apps":`;
  const handlerIndex = source.indexOf(needle);
  if (handlerIndex === -1) {
    warn(`Could not find ${needle} handler map needle`, "Zodex module status main-bundle patch");
    return source;
  }
  const handler = `"${HANDLER_NAME}":async()=>zodexModuleStatus(),`;
  const withHandler = source.slice(0, handlerIndex) + handler + source.slice(handlerIndex);
  const strict = withHandler.startsWith(`"use strict";`) ? `"use strict";`.length : 0;
  return withHandler.slice(0, strict) + mainBundleHelpers() + withHandler.slice(strict);
}

function webviewRuntimeSource() {
  return [
    `;(()=>{`,
    `const VERSION=${JSON.stringify(RUNTIME_VERSION)},METHOD=${JSON.stringify(HANDLER_NAME)};if(globalThis.zodexModuleWarningVersion===VERSION)return;globalThis.zodexModuleWarningVersion=VERSION;`,
    `let seq=0,pending=new Map,dialog=null,lastRevision=null;`,
    `function onMessage(e){let t=e?.data;if(!t||t.type!==\`fetch-response\`)return;let n=pending.get(t.requestId);if(!n)return;pending.delete(t.requestId);if(t.responseType===\`success\`){let v=null;try{v=t.bodyJsonString?JSON.parse(t.bodyJsonString):null}catch{}n.resolve(v)}else n.reject(Error(t.error||\`fetch failed\`))}`,
    `window.addEventListener(\`message\`,onMessage);`,
    `function dispatch(payload){let bridge=window.electronBridge,ev=new CustomEvent(\`codex-message-from-view\`,{detail:payload});if(bridge?.sendMessageFromView){ev.__codexForwardedViaBridge=!0;bridge.sendMessageFromView(payload).catch(()=>{})}window.dispatchEvent(ev)}`,
    `function status(){let requestId=\`zodex-module-status-\`+ ++seq,payload={type:\`fetch\`,hostId:\`local\`,requestId,method:\`POST\`,url:\`vscode://codex/\`+METHOD,body:\`{}\`};return new Promise((resolve,reject)=>{pending.set(requestId,{resolve,reject});setTimeout(()=>{pending.delete(requestId);reject(Error(\`timeout\`))},3000);dispatch(payload)})}`,
    `function style(){if(document.getElementById(\`zodex-module-warning-style\`))return;let s=document.createElement(\`style\`);s.id=\`zodex-module-warning-style\`;s.textContent=\`.zodex-module-warning{position:fixed;inset:0;z-index:2147483647;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.62);font:14px/1.45 -apple-system,BlinkMacSystemFont,Segoe UI,sans-serif}.zodex-module-warning-card{width:min(560px,calc(100vw - 40px));padding:22px;border:1px solid #d39a2c;border-radius:12px;background:#171717;color:#f5f5f5;box-shadow:0 22px 70px rgba(0,0,0,.55)}.zodex-module-warning-card h2{margin:0 0 12px;font-size:18px}.zodex-module-warning-card p{margin:0 0 16px;color:#ddd;white-space:pre-wrap}.zodex-module-warning-card code{font-family:ui-monospace,monospace}.zodex-module-warning-card button{float:right;padding:7px 12px;border:1px solid #777;border-radius:7px;background:#2b2b2b;color:#fff;cursor:pointer}.zodex-oauth-active{position:fixed;top:50%;left:50%;transform:translate(-50%,-50%);z-index:2147483000;width:min(520px,calc(100vw - 40px));padding:20px 24px;border:1px solid #d39a2c;border-radius:12px;background:#171717;color:#f5f5f5;box-shadow:0 22px 70px rgba(0,0,0,.55);font:14px/1.45 -apple-system,BlinkMacSystemFont,Segoe UI,sans-serif}.zodex-oauth-active-header{display:flex;align-items:center;justify-content:space-between;margin-bottom:12px}.zodex-oauth-active-header h2{margin:0;font-size:17px;color:#fff3c4}.zodex-oauth-active-close{background:none;border:none;color:#aaa;font-size:20px;line-height:1;cursor:pointer;padding:0 4px}.zodex-oauth-active-close:hover{color:#fff}.zodex-oauth-active p{margin:0 0 16px;color:#ddd;line-height:1.45;white-space:pre-wrap}.zodex-oauth-active-footer{display:flex;justify-content:flex-end}.zodex-oauth-active-footer button{padding:7px 14px;border:1px solid #777;border-radius:7px;background:#2b2b2b;color:#fff;cursor:pointer;font-size:13px}.zodex-oauth-active-footer button:hover{background:#3a3a3a}\`;document.head.appendChild(s)}`,
    `function show(item,file){style();dialog?.remove();let d=document.createElement(\`div\`);d.className=\`zodex-module-warning\`;d.setAttribute(\`role\`,\`alertdialog\`);d.setAttribute(\`aria-modal\`,\`true\`);let c=document.createElement(\`div\`);c.className=\`zodex-module-warning-card\`;let h=document.createElement(\`h2\`);h.textContent=item.title||\`Zodex configuration warning\`;let p=document.createElement(\`p\`);p.textContent=(item.message||\`The Zodex configuration is invalid.\`)+(file?\`\\n\\nConfig: \`+file:\`\`);let b=document.createElement(\`button\`);b.type=\`button\`;b.textContent=\`Close\`;b.addEventListener(\`click\`,()=>d.remove());c.append(h,p,b);d.append(c);(document.body||document.documentElement).appendChild(d);dialog=d}`,
    `function showActive(items,revision){let key=\`zodex_oauth_banner_dismissed:\`+revision;if(globalThis.sessionStorage?.getItem(key))return;let old=document.getElementById(\`zodex-oauth-active\`);if(!items.length){old?.remove();return}style();if(old)return;let b=document.createElement(\`div\`);b.id=\`zodex-oauth-active\`;b.className=\`zodex-oauth-active\`;b.setAttribute(\`role\`,\`status\`);let dismiss=()=>{b.remove();try{globalThis.sessionStorage?.setItem(key,\`1\`)}catch{}};let hdr=document.createElement(\`div\`);hdr.className=\`zodex-oauth-active-header\`;let h=document.createElement(\`h2\`);h.textContent=\`Third-Party OAuth Active\`;let xBtn=document.createElement(\`button\`);xBtn.type=\`button\`;xBtn.className=\`zodex-oauth-active-close\`;xBtn.innerHTML=\`&times;\`;xBtn.title=\`Close warning\`;xBtn.addEventListener(\`click\`,dismiss);hdr.append(h,xBtn);let p=document.createElement(\`p\`);p.textContent=\`Warning: third-party OAuth active via \`+items.map(e=>e.brokerId+\`/\`+e.providerId).join(\`, \`)+\`. Matching prompts and responses pass through the local broker; provider policy and quota still apply.\`;let ftr=document.createElement(\`div\`);ftr.className=\`zodex-oauth-active-footer\`;let cBtn=document.createElement(\`button\`);cBtn.type=\`button\`;cBtn.textContent=\`Close\`;cBtn.title=\`Close warning\`;cBtn.addEventListener(\`click\`,dismiss);ftr.appendChild(cBtn);b.append(hdr,p,ftr);(document.body||document.documentElement).appendChild(b)}`,
    `async function refresh(){try{let r=await status(),revision=typeof r?.revision===\`string\`?r.revision:\`unknown\`;if(revision===lastRevision)return;lastRevision=revision;dialog?.remove();dialog=null;document.getElementById(\`zodex-oauth-active\`)?.remove();if(r?.state===\`invalid\`){show({title:\`Zodex configuration is invalid\`,message:r.error},r.file);return}let all=Array.isArray(r?.connections)?r.connections:[];showActive(all.filter(e=>e.state===\`enabled\`),revision);let item=all.find(e=>e.state===\`warning-one\`||e.state===\`warning-two\`);if(item)show(item,r.file)}catch{}}`,
    `function start(){if(document.readyState===\`loading\`){document.addEventListener(\`DOMContentLoaded\`,start,{once:!0});return}refresh();setInterval(refresh,2000)}`,
    `start();`,
    `})();`,
  ].join("");
}

function applyWebviewRuntimePatch(source) {
  if (source.includes(`zodexModuleWarningVersion=`)) return source;
  return source.endsWith("\n") ? source + webviewRuntimeSource() : `${source}\n${webviewRuntimeSource()}`;
}

module.exports = {
  HANDLER_NAME,
  RUNTIME_VERSION,
  applyMainBundlePatch,
  applyWebviewRuntimePatch,
  descriptors: [
    {
      id: "module-status-handler",
      phase: "main-bundle",
      order: 20_930,
      ciPolicy: "optional",
      apply: applyMainBundlePatch,
    },
    {
      id: "oauth-warning-runtime",
      phase: "webview-asset",
      order: 20_931,
      ciPolicy: "optional",
      pattern: /^index-.*\.js$/u,
      missingDescription: "webview index bundle",
      skipDescription: "Zodex OAuth warning runtime patch",
      apply: applyWebviewRuntimePatch,
    },
  ],
};
