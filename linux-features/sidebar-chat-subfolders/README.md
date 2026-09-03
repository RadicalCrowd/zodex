# Sidebar Chat Subfolders

Organize project chats into collapsible client-side subfolders in the left sidebar.

This feature is disabled by default and stores subfolder structures strictly on the local client (`~/.config/zodex/chat-folders.json`, mode `0600`), without affecting cloud project synchronization.

## Enabling the Feature

Enable it in `linux-features/features.json`:

```json
{
  "enabled": [
    "sidebar-chat-subfolders"
  ]
}
```

## Features

- **Nested Tree Layout**: Shows collapsible folder rows with chevron expand/collapse toggles and indented chat items.
- **Client-Side Store**: Keeps folder taxonomy locally in `~/.config/zodex/chat-folders.json`.
- **AI Agent Automation**: Provides local IPC tools for agents to create folders and organize conversations upon prompt instruction.

## Testing

Run tests with:

```bash
node --test linux-features/sidebar-chat-subfolders/test.js
```
