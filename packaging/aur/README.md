# AUR release-candidate flow

This directory is source-only. The AUR recipe never downloads, embeds, or publishes OpenAI's proprietary Desktop payload. The owner supplies the official `.deb` and the matching signed APT evidence outside the source archive. `verify-zodex-byob.js` checks the pinned repository signing key, `InRelease`, indexed `Packages`, package hash/size, and Debian identity before local extraction.

The resulting single `zodex` package contains the locally derived Desktop integration, `/usr/bin/zodex`, and `/usr/lib/zodex/zodex-control-plane`. Publication remains a separate approval.
