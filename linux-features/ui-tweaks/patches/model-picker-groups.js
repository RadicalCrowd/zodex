"use strict";

const MODEL_PICKER_GROUPS_ASSET_PATTERN = /^app-initial-[^.]+\.js$/;
const MODEL_PICKER_GROUPS_RUNTIME_MARKER = "codex-linux-grouped-model-list-runtime";
const JS_IDENT = "[A-Za-z_$][\\w$]*";

function warn(message) {
  console.warn(`WARN: ${message} - skipping ui-tweaks model picker groups patch`);
}

function modelPickerGroupsConfig(context) {
  const defaults = context?.feature?.manifest?.tweaks?.modelPicker?.showModelsByDefault;
  const settings = context?.feature?.settings?.tweaks?.modelPicker?.showModelsByDefault;
  const groupSettings = context?.feature?.settings?.tweaks?.modelPicker?.groupedModels;
  return {
    ...(defaults != null && typeof defaults === "object" && !Array.isArray(defaults) ? defaults : {}),
    ...(settings != null && typeof settings === "object" && !Array.isArray(settings) ? settings : {}),
    ...(groupSettings != null && typeof groupSettings === "object" && !Array.isArray(groupSettings) ? groupSettings : {}),
  };
}

function enabled(context) {
  return modelPickerGroupsConfig(context).enabled !== false;
}

function escapedPattern(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function classifyModelGroup(option, customCategories = null) {
  const id = String(option?.id || option?.value || option?.model || option?.slug || "").toLowerCase();
  const label = String(option?.label || option?.displayName || option?.name || "");
  const desc = String(option?.description || "").toLowerCase();
  const raw = `${id} ${label.toLowerCase()} ${desc}`;

  // Check custom categories if provided
  if (customCategories && typeof customCategories === "object") {
    for (const [catName, patterns] of Object.entries(customCategories)) {
      if (Array.isArray(patterns)) {
        for (const pat of patterns) {
          const p = String(pat).toLowerCase();
          if (p.endsWith("*") && (id.startsWith(p.slice(0, -1)) || label.toLowerCase().startsWith(p.slice(0, -1)))) {
            return catName;
          }
          if (id === p || label.toLowerCase() === p || raw.includes(p)) {
            return catName;
          }
        }
      }
    }
  }

  // Broker-first automatic grouping
  if (
    raw.includes("omniroute") ||
    raw.includes("antigravity") ||
    raw.startsWith("agy/") ||
    id.startsWith("omniroute-oauth/")
  ) {
    return "OmniRoute / Antigravity";
  }
  if (raw.includes("kilo-free") || raw.includes("[kilofree]")) {
    return "Kilo Free";
  }
  if (raw.includes("opencode-free") || raw.includes("[opencodefree]")) {
    return "OpenCode Free";
  }
  if (
    id.startsWith("gpt-") ||
    id.startsWith("o1") ||
    id.startsWith("o3") ||
    id.startsWith("o4") ||
    id.startsWith("chatgpt") ||
    id.startsWith("5.") ||
    id.startsWith("4.") ||
    label.startsWith("5.") ||
    label.startsWith("4.") ||
    id === "codex-auto-review" ||
    id === "gpt-reserve"
  ) {
    return "OpenAI Native";
  }
  if (raw.includes("claude")) {
    return "Anthropic Claude";
  }
  if (raw.includes("gemini")) {
    return "Google Gemini";
  }
  return "Other Models";
}

function groupModelOptions(options, customCategories = null) {
  if (!Array.isArray(options)) return [];
  const groups = new Map();
  const defaultOrder = [
    "OmniRoute / Antigravity",
    "OpenAI Native",
    "Anthropic Claude",
    "Google Gemini",
    "Kilo Free",
    "OpenCode Free",
    "Other Models",
  ];

  if (customCategories && typeof customCategories === "object") {
    for (const name of Object.keys(customCategories)) {
      groups.set(name, []);
    }
  }

  for (const name of defaultOrder) {
    if (!groups.has(name)) {
      groups.set(name, []);
    }
  }

  for (const option of options) {
    const groupName = classifyModelGroup(option, customCategories);
    if (!groups.has(groupName)) {
      groups.set(groupName, []);
    }
    groups.get(groupName).push(option);
  }

  const result = [];
  for (const [name, items] of groups.entries()) {
    if (items.length > 0) {
      result.push({ name, items });
    }
  }
  return result;
}

function modelGroupsRuntimeSource() {
  return [
    `function zodexGetCustomCategories(){`,
    `try{let local=globalThis.localStorage?.getItem("zodex_model_custom_categories");if(local)return JSON.parse(local);}catch{}`,
    `return globalThis.__ZODEX_CUSTOM_MODEL_CATEGORIES__||null;`,
    `}`,
    `globalThis.__ZODEX_SET_MODEL_CATEGORIES__=function(cats){`,
    `try{if(cats){globalThis.localStorage?.setItem("zodex_model_custom_categories",JSON.stringify(cats));}else{globalThis.localStorage?.removeItem("zodex_model_custom_categories");}console.log("[Zodex] Custom categories updated",cats);}catch(e){console.error(e);}`,
    `};`,
    `function zodexClassifyModelGroup(option){`,
    `let id=String(option?.id||option?.value||option?.model||option?.slug||\`\`).toLowerCase();`,
    `let label=String(option?.label||option?.displayName||option?.name||\`\`);`,
    `let desc=String(option?.description||\`\`).toLowerCase();`,
    `let raw=id+\` \`+label.toLowerCase()+\` \`+desc;`,
    `let custom=zodexGetCustomCategories();`,
    `if(custom&&typeof custom==="object"){`,
    `for(let[cat,pats]of Object.entries(custom)){`,
    `if(Array.isArray(pats)){for(let pat of pats){let p=String(pat).toLowerCase();if(p.endsWith("*")&&(id.startsWith(p.slice(0,-1))||label.toLowerCase().startsWith(p.slice(0,-1))))return cat;if(id===p||label.toLowerCase()===p||raw.includes(p))return cat;}}`,
    `}`,
    `}`,
    `if(raw.includes(\`omniroute\`)||raw.includes(\`antigravity\`)||raw.startsWith(\`agy/\`)||id.startsWith(\`omniroute-oauth/\`))return\`OmniRoute / Antigravity\`;`,
    `if(raw.includes(\`kilo-free\`)||raw.includes(\`[kilofree]\`))return\`Kilo Free\`;`,
    `if(raw.includes(\`opencode-free\`)||raw.includes(\`[opencodefree]\`))return\`OpenCode Free\`;`,
    `if(id.startsWith(\`gpt-\`)||id.startsWith(\`o1\`)||id.startsWith(\`o3\`)||id.startsWith(\`o4\`)||id.startsWith(\`chatgpt\`)||id.startsWith(\`5.\`)||id.startsWith(\`4.\`)||label.startsWith(\`5.\`)||label.startsWith(\`4.\`)||id===\`codex-auto-review\`||id===\`gpt-reserve\`)return\`OpenAI Native\`;`,
    `if(raw.includes(\`claude\`))return\`Anthropic Claude\`;`,
    `if(raw.includes(\`gemini\`))return\`Google Gemini\`;`,
    `return\`Other Models\`;`,
    `}`,
    `function zodexGroupModelOptions(options){`,
    `if(!Array.isArray(options))return[];`,
    `let custom=zodexGetCustomCategories();`,
    `let groups=new Map;`,
    `if(custom&&typeof custom==="object"){for(let name of Object.keys(custom))groups.set(name,[]);}let order=[\`OmniRoute / Antigravity\`,\`OpenAI Native\`,\`Anthropic Claude\`,\`Google Gemini\`,\`Kilo Free\`,\`OpenCode Free\`,\`Other Models\`];`,
    `for(let o of order){if(!groups.has(o))groups.set(o,[]);}`,
    `for(let opt of options){let g=zodexClassifyModelGroup(opt);if(!groups.has(g))groups.set(g,[]);groups.get(g).push(opt);}`,
    `let out=[];for(let[name,items]of groups.entries()){if(items.length)out.push({name,items});}return out;`,
    `}`,
    `function zodexRenderGroupedModelOptions(jsx,options,optionRenderer,menuNamespace){`,
    `if(!Array.isArray(options)||!options.length)return options?options.map(optionRenderer):[];`,
    `let isEffortList=options.every(opt=>{let v=String(opt?.value||opt?.id||opt?.effort||opt?.label||\`\`).toLowerCase();return[\`low\`,\`medium\`,\`high\`,\`xhigh\`,\`ultra\`,\`max\`,\`standard\`,\`fast\`,\`extra high\`].includes(v);});`,
    `if(isEffortList)return options.map(optionRenderer);`,
    `let groups=zodexGroupModelOptions(options);`,
    `if(!groups.length)return options.map(optionRenderer);`,
    `return groups.map((grp,idx)=>{`,
    `let storageKey=\`zodex_model_group_\`+grp.name.replace(/\\s+/g,\`_\`);`,
    `let isExpanded=globalThis.sessionStorage?.getItem(storageKey)===\`open\`;`,
    `let toggleCollapse=(e)=>{`,
    `e.stopPropagation();`,
    `let list=e.currentTarget?.nextElementSibling;`,
    `let icon=e.currentTarget?.querySelector(\`.zodex-group-chevron\`);`,
    `let currentlyHidden=list?(list.style.display===\`none\`):true;`,
    `let willOpen=currentlyHidden;`,
    `try{if(willOpen){globalThis.sessionStorage?.setItem(storageKey,\`open\`)}else{globalThis.sessionStorage?.removeItem(storageKey)}}catch{}`,
    `if(list)list.style.display=willOpen?\`flex\`:\`none\`;`,
    `if(icon)icon.textContent=willOpen?\`▾\`:\`▸\`;`,
    `};`,
    `let header=(0,jsx.jsxs)(\`button\`,{type:\`button\`,onClick:toggleCollapse,className:\`flex w-full items-center justify-between px-2.5 py-1.5 text-[11px] font-bold uppercase tracking-wider text-token-text-tertiary hover:bg-token-surface-hover hover:text-token-text-secondary rounded cursor-pointer select-none bg-token-surface-secondary/40 my-0.5\`,children:[`,
    `(0,jsx.jsxs)(\`span\`,{className:\`flex items-center gap-1.5\`,children:[(0,jsx.jsx)(\`span\`,{className:\`zodex-group-chevron font-mono text-[10px]\`,children:isExpanded?\`▾\`:\`▸\`}),grp.name]}),`,
    `(0,jsx.jsx)(\`span\`,{className:\`rounded-full bg-token-surface-secondary px-1.5 py-0.2 text-[10px] font-semibold text-token-text-secondary\`,children:String(grp.items.length)})`,
    `]});`,
    `let list=(0,jsx.jsx)(\`div\`,{className:\`zodex-model-group-list flex flex-col pl-1\`,style:{display:isExpanded?\`flex\`:\`none\`},children:grp.items.map(optionRenderer)});`,
    `return(0,jsx.jsxs)(\`div\`,{className:\`zodex-model-group-container flex flex-col\`,children:[header,list]},grp.name);`,
    `});`,
    `}`,
  ].join("");
}

function applySubmenuGroupsPatch(source) {
  // Replace options.map in submenu component with zodexRenderGroupedModelOptions
  const modelSubmenuPattern = new RegExp(
    `([,;])(${JS_IDENT});(${JS_IDENT})\\[(\\d+)\\]!==(${JS_IDENT})\\.model\\|\\|` +
      `\\3\\[(\\d+)\\]!==(${JS_IDENT})\\?\\(\\2=\\7\\|\\|\\5\\.model==null\\?null:` +
      `\\(0,(${JS_IDENT})\\.jsx\\)\\((${JS_IDENT}),\\{submenu:\\5\\.model\\}\\)`,
    "g",
  );
  const submenuMatches = [...source.matchAll(modelSubmenuPattern)];
  if (submenuMatches.length !== 1) return source;

  const match = submenuMatches[0];
  const submenuComponent = match[9];
  const submenuMarker = `function ${submenuComponent}(`;
  const submenuStart = source.indexOf(submenuMarker);
  if (submenuStart < 0) return source;

  const submenuEnd = source.indexOf("function ", submenuStart + submenuMarker.length);
  if (submenuEnd < 0) return source;

  const submenuSection = source.slice(submenuStart, submenuEnd);
  const jsxNamespace = match[8];
  const optionMapPattern = new RegExp(`\\.options\\.map\\((${JS_IDENT})\\)`, "g");
  const optionMapMatches = [...submenuSection.matchAll(optionMapPattern)];
  if (optionMapMatches.length !== 1) return source;

  const optionRenderer = optionMapMatches[0][1];
  const patchedSection = submenuSection.replace(
    new RegExp(`\\.options\\.map\\(${escapedPattern(optionRenderer)}\\)`),
    `.options?typeof zodexRenderGroupedModelOptions!=="undefined"?zodexRenderGroupedModelOptions(${jsxNamespace},arguments[0].submenu.options,${optionRenderer}):arguments[0].submenu.options.map(${optionRenderer}):[]`,
  );

  return source.slice(0, submenuStart) + patchedSection + source.slice(submenuEnd);
}

function applyModelPickerGroupsPatch(source, context = {}) {
  try {
    if (typeof source !== "string") {
      warn("Asset source is not a string");
      return source;
    }
    if (!enabled(context) || source.includes(MODEL_PICKER_GROUPS_RUNTIME_MARKER)) {
      return source;
    }

    const needle = ".options.map(";
    if (!source.includes(needle)) {
      return source;
    }

    const patchedSubmenu = applySubmenuGroupsPatch(source);
    const runtime = `/*${MODEL_PICKER_GROUPS_RUNTIME_MARKER}*/\n${modelGroupsRuntimeSource()}\n`;
    const strict = patchedSubmenu.startsWith(`"use strict";`) ? `"use strict";`.length : 0;
    return patchedSubmenu.slice(0, strict) + runtime + patchedSubmenu.slice(strict);
  } catch (error) {
    warn(`Unexpected error: ${error instanceof Error ? error.message : String(error)}`);
    return source;
  }
}

const descriptors = [
  {
    id: "model-picker-groups-runtime",
    phase: "webview-asset",
    order: 20_798,
    ciPolicy: "optional",
    enabled,
    pattern: MODEL_PICKER_GROUPS_ASSET_PATTERN,
    missingDescription: "composer model picker menu bundle",
    skipDescription: "ui-tweaks model picker groups runtime patch",
    apply: (source, context = {}) =>
      applyModelPickerGroupsPatch(source, {
        ...context,
        warnOnMissingMarkers: true,
      }),
  },
];

module.exports = {
  MODEL_PICKER_GROUPS_ASSET_PATTERN,
  MODEL_PICKER_GROUPS_RUNTIME_MARKER,
  applyModelPickerGroupsPatch,
  applySubmenuGroupsPatch,
  classifyModelGroup,
  descriptors,
  groupModelOptions,
  modelGroupsRuntimeSource,
  modelPickerGroupsConfig,
};
