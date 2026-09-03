---
name: zodex-chat-subfolders
description: Create, organize, and manage client-side subfolders for project chats in Zodex. Use when the user asks to create a subfolder, organize conversations into a category, or when suggesting folder grouping for complex multi-part tasks.
---

# Zodex Chat Subfolders Skill

This skill enables AI agents in Zodex to interact with client-side project subfolders via local IPC or file storage (`~/.config/zodex/chat-folders.json`).

## Trigger Conditions

Use this skill when:
- The user requests: "Create a subfolder named X", "Move this chat to folder Y", "Organize my chats into folders"
- Proactively suggesting subfolder organization for multi-stage tasks (e.g. "I can organize our investigation and refactoring threads into dedicated subfolders").

## Architecture & Storage

Subfolders are stored locally at:
`~/.config/zodex/chat-folders.json` (File permission `0600`).

Format:
```json
{
  "version": 1,
  "projects": {
    "<projectId>": {
      "folders": [
        {
          "id": "fld_xyz",
          "name": "Backend Tasks",
          "collapsed": false,
          "threadIds": ["thread_123"]
        }
      ]
    }
  }
}
```

## Available Operations

1. **Create Subfolder**: Add a new subfolder object under the target project.
2. **Move Conversation**: Append the `threadId` to the destination folder's `threadIds` and remove it from prior folders.
3. **Toggle Collapse**: Set `collapsed: true` or `false` to expand/hide items.
