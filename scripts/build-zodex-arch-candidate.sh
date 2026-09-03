#!/bin/bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_DIR="${ZODEX_APP_DIR:-$REPO_DIR/zodex-app-next}"
REPORT_DIR="${ZODEX_REPORT_DIR:-$REPO_DIR/dist-next/zodex-rebuild}"
DIST_DIR="${ZODEX_DIST_DIR:-$REPO_DIR/dist-next/zodex-pacman}"
ICON_SOURCE="${ZODEX_ICON_SOURCE:-$REPO_DIR/assets/zodex.png}"
FEATURE_CONFIG="${CODEX_LINUX_FEATURES_CONFIG:-$REPO_DIR/linux-features/features.json}"
TEMP_FEATURE_CONFIG=""

if [ "${ZODEX_DEVELOPER_UPDATER:-0}" = "1" ]; then
    TEMP_FEATURE_CONFIG="$(mktemp)"
    node - "$FEATURE_CONFIG" "$TEMP_FEATURE_CONFIG" <<'NODE'
const fs = require("node:fs");
const [input, output] = process.argv.slice(2);
let value = { enabled: [] };
if (fs.existsSync(input)) value = JSON.parse(fs.readFileSync(input, "utf8"));
if (!Array.isArray(value.enabled)) throw new Error("Linux feature configuration must have an enabled array");
value.enabled = [...new Set([...value.enabled, "zodex-developer-updater"])];
fs.writeFileSync(output, `${JSON.stringify(value)}\n`, { mode: 0o600 });
NODE
    FEATURE_CONFIG="$TEMP_FEATURE_CONFIG"
    trap 'rm -f "$TEMP_FEATURE_CONFIG"' EXIT
fi

[ -f "$ICON_SOURCE" ] || {
    echo "[zodex][ERROR] Missing Zodex icon: $ICON_SOURCE" >&2
    exit 1
}

CODEX_APP_ID=zodex \
CODEX_APP_DISPLAY_NAME=Zodex \
CODEX_APP_ICON_SOURCE="$ICON_SOURCE" \
CODEX_NEXT_APP_DIR="$APP_DIR" \
REBUILD_REPORT_DIR="$REPORT_DIR" \
CODEX_LINUX_FEATURES_CONFIG="$FEATURE_CONFIG" \
    "$REPO_DIR/scripts/rebuild-candidate.sh" "$@"

package_version="$(node - "$APP_DIR/.codex-linux/build-info.json" <<'NODE'
const fs = require("node:fs");
const buildInfo = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const upstream = buildInfo.upstreamLinuxPackage?.version;
const commit = buildInfo.source?.shortCommit;
if (!/^[0-9][0-9.]*$/.test(upstream ?? "") || !/^[0-9a-f]{7,40}$/.test(commit ?? "")) {
  throw new Error("Zodex build info has invalid upstream version or source commit");
}
const snapshot = process.env.ZODEX_SOURCE_SNAPSHOT_ID;
if (snapshot != null && !/^[a-f0-9]{8,64}$/.test(snapshot)) throw new Error("Zodex source snapshot id is invalid");
process.stdout.write(`${upstream}+zodex.${commit}${snapshot ? `.dev.${snapshot}` : ""}`);
NODE
)"

APP_DIR_OVERRIDE="$APP_DIR" \
DIST_DIR_OVERRIDE="$DIST_DIR" \
PACKAGE_NAME=zodex \
PACKAGE_DISPLAY_NAME=Zodex \
PACKAGE_COMMENT="Zodex, an Arch-first coding assistant desktop fork" \
PACKAGE_MAINTAINER="RadicalCrowd Zodex Maintainers" \
PACKAGE_DESCRIPTION="Zodex desktop for Arch Linux, built from OpenAI's official Linux package" \
PACKAGE_URL="https://github.com/RadicalCrowd/zodex" \
PACKAGE_PACKAGER="RadicalCrowd Zodex Maintainers <RadicalCrowd@users.noreply.github.com>" \
PACKAGE_ICON_SOURCE="$ICON_SOURCE" \
PACKAGE_VERSION="$package_version" \
PACKAGE_WITH_UPDATER=0 \
CODEX_LINUX_FEATURES_CONFIG="$FEATURE_CONFIG" \
    "$REPO_DIR/scripts/build-pacman.sh"

printf '[zodex] Candidate complete\n  App: %s\n  Reports: %s\n  Packages: %s\n  Version: %s\n' \
    "$APP_DIR" "$REPORT_DIR" "$DIST_DIR" "$package_version"
