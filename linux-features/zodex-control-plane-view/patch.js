"use strict";

const HANDLER = "zodex-control-plane-status";

function mainHelpers() {
  return [
    `function zodexControlSnapshot(){return new Promise(resolve=>{let p=process.env.ZODEX_CONTROL_SOCKET;if(typeof p!==\`string\`||!p.startsWith(\`/\`)){resolve({state:\`inactive\`,agents:[],panicked:!1});return}try{let fs=require(\`node:fs\`),st=fs.lstatSync(p);if(!st.isSocket()||st.uid!==process.getuid()||(st.mode&63)!==0){resolve({state:\`refused\`,agents:[],panicked:!1});return}let net=require(\`node:net\`),s=net.createConnection(p),chunks=[],size=0,done=!1,finish=v=>{if(done)return;done=!0;s.destroy();resolve(v)},timer=setTimeout(()=>finish({state:\`unavailable\`,agents:[],panicked:!1}),1500);s.on(\`connect\`,()=>{let n=process.pid+\`-\`+Date.now();s.write(JSON.stringify({api_major:1,request_id:n,idempotency_key:\`desktop-\`+n,client_id:\`zodex-desktop\`,client_version:\`1\`,nonce:n,method:\`inventory\`,params:{}})+\`\\n\`)});s.on(\`data\`,b=>{size+=b.length;if(size>65536){clearTimeout(timer);finish({state:\`refused\`,agents:[],panicked:!1});return}chunks.push(b);let text=Buffer.concat(chunks).toString(\`utf8\`),i=text.indexOf(\`\\n\`);if(i<0)return;clearTimeout(timer);try{let r=JSON.parse(text.slice(0,i));finish(r.ok?{state:\`ready\`,agents:Array.isArray(r.result?.agents)?r.result.agents:[],panicked:r.result?.panicked===!0}:{state:\`denied\`,agents:[],panicked:!1})}catch{finish({state:\`invalid\`,agents:[],panicked:!1})}});s.on(\`error\`,()=>{clearTimeout(timer);finish({state:\`unavailable\`,agents:[],panicked:!1})})}catch{resolve({state:\`refused\`,agents:[],panicked:!1})}})}`,
  ].join("");
}

function applyMainBundlePatch(source) {
  if (source.includes(`"${HANDLER}":async`)) return source;
  const needle = `"native-desktop-apps":`;
  const index = source.indexOf(needle);
  if (index < 0) return source;
  const strict = source.startsWith(`"use strict";`) ? `"use strict";`.length : 0;
  const withHelper = source.slice(0, strict) + mainHelpers() + source.slice(strict);
  const handlerIndex = withHelper.indexOf(needle);
  return withHelper.slice(0, handlerIndex) + `"${HANDLER}":async()=>zodexControlSnapshot(),` + withHelper.slice(handlerIndex);
}

function runtimeSource() {
  return `;(()=>{if(globalThis.zodexControlPlaneViewV1)return;globalThis.zodexControlPlaneViewV1=!0;let q=0,p=new Map;window.addEventListener(\`message\`,e=>{let d=e?.data;if(d?.type!==\`fetch-response\`)return;let r=p.get(d.requestId);if(!r)return;p.delete(d.requestId);try{r(d.responseType===\`success\`?JSON.parse(d.bodyJsonString||\`null\`):null)}catch{r(null)}});function fetchStatus(){let requestId=\`zodex-control-\`+ ++q,payload={type:\`fetch\`,hostId:\`local\`,requestId,method:\`POST\`,url:\`vscode://codex/${HANDLER}\`,body:\`{}\`};return new Promise(resolve=>{p.set(requestId,resolve);let ev=new CustomEvent(\`codex-message-from-view\`,{detail:payload});if(window.electronBridge?.sendMessageFromView){ev.__codexForwardedViaBridge=!0;window.electronBridge.sendMessageFromView(payload).catch(()=>{})}window.dispatchEvent(ev);setTimeout(()=>{if(p.delete(requestId))resolve(null)},2000)})}function render(v){let old=document.getElementById(\`zodex-control-plane-view\`);if(!v||v.state===\`inactive\`){old?.remove();return}let e=old||document.createElement(\`aside\`);e.id=\`zodex-control-plane-view\`;e.setAttribute(\`role\`,\`status\`);e.style.cssText=\`position:fixed;right:16px;bottom:16px;z-index:2147482000;padding:10px 12px;border:1px solid #555;border-radius:8px;background:#171717;color:#eee;font:12px/1.4 ui-monospace,monospace\`;let agents=Array.isArray(v.agents)?v.agents:[];e.textContent=\`Zodex: \`+(v.panicked?\`PANICKED\`:v.state)+\` · \`+agents.length+\` managed agent\`+(agents.length===1?\`\`:\`s\`);if(!old)(document.body||document.documentElement).appendChild(e)}async function start(){render(await fetchStatus())}document.readyState===\`loading\`?document.addEventListener(\`DOMContentLoaded\`,start,{once:!0}):start()})();`;
}

function applyWebviewPatch(source) {
  if (source.includes("zodexControlPlaneViewV1")) return source;
  return `${source}\n${runtimeSource()}`;
}

module.exports = {
  HANDLER,
  applyMainBundlePatch,
  applyWebviewPatch,
  descriptors: [
    { id: "control-plane-status-handler", phase: "main-bundle", order: 20940, ciPolicy: "optional", apply: applyMainBundlePatch },
    { id: "control-plane-view-runtime", phase: "webview-asset", order: 20941, ciPolicy: "optional", pattern: /^index-.*\.js$/u, missingDescription: "webview index bundle", skipDescription: "Zodex control-plane view", apply: applyWebviewPatch }
  ],
};
