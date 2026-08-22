#!/usr/bin/env bash
set -Eeuo pipefail

feature_dir="$SCRIPT_DIR/linux-features/privileged-exec-research"
plugin_template="$SCRIPT_DIR/plugins/openai-bundled/plugins/privileged-exec-research"
target_plugin="$INSTALL_DIR/resources/plugins/openai-bundled/plugins/privileged-exec-research"
target_marketplace="$INSTALL_DIR/resources/plugins/openai-bundled/.agents/plugins/marketplace.json"

resolve_backend() {
    local source_binary="$SCRIPT_DIR/target/release/codex-privileged-exec-linux"

    if [ -n "${CODEX_PRIVILEGED_EXEC_RESEARCH_SOURCE:-}" ]; then
        source_binary="$CODEX_PRIVILEGED_EXEC_RESEARCH_SOURCE"
    fi

    [ -x "$source_binary" ] || {
        echo "Privileged Exec Research requires an executable prebuilt backend: $source_binary" >&2
        echo "Build the research helper separately or set CODEX_PRIVILEGED_EXEC_RESEARCH_SOURCE; staging never invokes Cargo." >&2
        return 1
    }
    printf '%s\n' "$source_binary"
}

[ -d "$plugin_template" ] || {
    echo "Privileged Exec Research plugin template not found: $plugin_template" >&2
    exit 1
}

backend_binary="$(resolve_backend)"

rm -rf "$target_plugin"
mkdir -p "$target_plugin/bin"
cp -R "$plugin_template/." "$target_plugin/"
cp "$backend_binary" "$target_plugin/bin/codex-privileged-exec-linux"
chmod 0755 "$target_plugin/bin/codex-privileged-exec-linux"
find "$target_plugin" \( -name '*:com.apple.*' -o -name '.gitkeep' \) -delete

node - "$target_marketplace" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");

const marketplacePath = process.argv[2];
let marketplace = { plugins: [] };
try {
  marketplace = JSON.parse(fs.readFileSync(marketplacePath, "utf8"));
} catch (_error) {
  marketplace = { plugins: [] };
}
if (!Array.isArray(marketplace.plugins)) marketplace.plugins = [];
marketplace.plugins = marketplace.plugins.filter(
  (plugin) => plugin?.name !== "privileged-exec-research",
);
marketplace.plugins.push({
  name: "privileged-exec-research",
  source: { source: "local", path: "./plugins/privileged-exec-research" },
  policy: { installation: "AVAILABLE", authentication: "ON_INSTALL" },
  category: "Productivity",
});
fs.mkdirSync(path.dirname(marketplacePath), { recursive: true });
fs.writeFileSync(marketplacePath, `${JSON.stringify(marketplace, null, 2)}\n`);
NODE

echo "Privileged Exec Research plugin staged from a prebuilt artifact" >&2
