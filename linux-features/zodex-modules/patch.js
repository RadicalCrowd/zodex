"use strict";

const HANDLER_NAME = "zodex-module-status";
const RUNTIME_VERSION = "zodex-module-warning-v1";

function warn(message, patchName) {
  console.warn(`WARN: ${message} - skipping ${patchName}`);
}

function mainBundleHelpers() {
  return [
    `function zodexConfigPathModule(){return require(\`node:path\`)}`,
    `function zodexConfigModule(){let r=process.env.CODEX_LINUX_FEATURES_DIR;if(typeof r!==\`string\`||!r.trim())throw Error(\`CODEX_LINUX_FEATURES_DIR is unavailable\`);return require(zodexConfigPathModule().join(r,\`zodex-modules\`,\`config.js\`))}`,
    `function zodexModuleStatus(){try{let m=zodexConfigModule(),r=m.readConfig(process.env);if(r.state!==\`valid\`)return{ok:r.state!==\`invalid\`,state:r.state,file:r.file,error:r.error,connections:[]};return{ok:!0,state:\`valid\`,file:r.file,connections:m.brokerConnectionStates(r.config)}}catch(e){return{ok:!1,state:\`invalid\`,file:null,error:String(e?.message||e),connections:[]}}}`,
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
    `let seq=0,pending=new Map,dialog=null;`,
    `function onMessage(e){let t=e?.data;if(!t||t.type!==\`fetch-response\`)return;let n=pending.get(t.requestId);if(!n)return;pending.delete(t.requestId);if(t.responseType===\`success\`){let v=null;try{v=t.bodyJsonString?JSON.parse(t.bodyJsonString):null}catch{}n.resolve(v)}else n.reject(Error(t.error||\`fetch failed\`))}`,
    `window.addEventListener(\`message\`,onMessage);`,
    `function dispatch(payload){let bridge=window.electronBridge,ev=new CustomEvent(\`codex-message-from-view\`,{detail:payload});if(bridge?.sendMessageFromView){ev.__codexForwardedViaBridge=!0;bridge.sendMessageFromView(payload).catch(()=>{})}window.dispatchEvent(ev)}`,
    `function status(){let requestId=\`zodex-module-status-\`+ ++seq,payload={type:\`fetch\`,hostId:\`local\`,requestId,method:\`POST\`,url:\`vscode://codex/\`+METHOD,body:\`{}\`};return new Promise((resolve,reject)=>{pending.set(requestId,{resolve,reject});setTimeout(()=>{pending.delete(requestId);reject(Error(\`timeout\`))},3000);dispatch(payload)})}`,
    `function style(){if(document.getElementById(\`zodex-module-warning-style\`))return;let s=document.createElement(\`style\`);s.id=\`zodex-module-warning-style\`;s.textContent=\`.zodex-module-warning{position:fixed;inset:0;z-index:2147483647;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.62);font:14px/1.45 -apple-system,BlinkMacSystemFont,Segoe UI,sans-serif}.zodex-module-warning-card{width:min(560px,calc(100vw - 40px));padding:22px;border:1px solid #d39a2c;border-radius:12px;background:#171717;color:#f5f5f5;box-shadow:0 22px 70px rgba(0,0,0,.55)}.zodex-module-warning-card h2{margin:0 0 12px;font-size:18px}.zodex-module-warning-card p{margin:0 0 16px;color:#ddd;white-space:pre-wrap}.zodex-module-warning-card code{font-family:ui-monospace,monospace}.zodex-module-warning-card button{float:right;padding:7px 12px;border:1px solid #777;border-radius:7px;background:#2b2b2b;color:#fff;cursor:pointer}.zodex-oauth-active{position:fixed;left:16px;right:16px;top:8px;z-index:2147483000;padding:8px 12px;border:1px solid #d39a2c;border-radius:8px;background:#4b3409;color:#fff3c4;font:12px/1.35 -apple-system,BlinkMacSystemFont,Segoe UI,sans-serif;text-align:center;pointer-events:none}\`;document.head.appendChild(s)}`,
    `function show(item,file){style();dialog?.remove();let d=document.createElement(\`div\`);d.className=\`zodex-module-warning\`;d.setAttribute(\`role\`,\`alertdialog\`);d.setAttribute(\`aria-modal\`,\`true\`);let c=document.createElement(\`div\`);c.className=\`zodex-module-warning-card\`;let h=document.createElement(\`h2\`);h.textContent=item.title||\`Zodex configuration warning\`;let p=document.createElement(\`p\`);p.textContent=(item.message||\`The Zodex configuration is invalid.\`)+(file?\`\\n\\nConfig: \`+file:\`\`);let b=document.createElement(\`button\`);b.type=\`button\`;b.textContent=\`Close\`;b.addEventListener(\`click\`,()=>d.remove());c.append(h,p,b);d.append(c);(document.body||document.documentElement).appendChild(d);dialog=d}`,
    `function showActive(items){let old=document.getElementById(\`zodex-oauth-active\`);if(!items.length){old?.remove();return}style();let b=old||document.createElement(\`div\`);b.id=\`zodex-oauth-active\`;b.className=\`zodex-oauth-active\`;b.setAttribute(\`role\`,\`status\`);b.textContent=\`Warning: third-party OAuth active via \`+items.map(e=>e.brokerId+\`/\`+e.providerId).join(\`, \`)+\`. Matching prompts and responses pass through the local broker; provider policy and quota still apply.\`;if(!old)(document.body||document.documentElement).appendChild(b)}`,
    `async function refresh(){try{let r=await status();if(r?.state===\`invalid\`){show({title:\`Zodex configuration is invalid\`,message:r.error},r.file);return}let all=Array.isArray(r?.connections)?r.connections:[];showActive(all.filter(e=>e.state===\`enabled\`));let item=all.find(e=>e.state===\`warning-one\`||e.state===\`warning-two\`);if(item)show(item,r.file)}catch{}}`,
    `function start(){if(document.readyState===\`loading\`){document.addEventListener(\`DOMContentLoaded\`,start,{once:!0});return}refresh()}`,
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
