# Interactive Console & Remote Terminal

Interactive bottom console dock with live streaming terminal output, interactive stdin prompts, sudo password intervention, and ChatGPT mobile remote control mirroring.

This feature is disabled by default. Enable it in `linux-features/features.json`:

```json
{
  "enabled": [
    "interactive-console"
  ]
}
```

## Features

- **Live Streaming Console Dock**: Bottom dock panel with real-time ANSI terminal rendering, auto-scroll, resize, and minimize/maximize.
- **Interactive Stdin & Sudo Intervention**: Detects interactive prompts (`[sudo] password for ...`, `[y/N]`) and presents an inline prompt and auto-detected masked password bar.
- **Zero-Secret Memory Guarantee**: Password inputs are immediately zeroed in memory upon submission and never recorded to disk or chat transcripts.
- **ChatGPT Mobile Mirror**: Streams live terminal chunks and status cards to the mobile app with remote stdin input support.

## Testing

Run tests with:

```bash
node --test linux-features/interactive-console/test.js
```
