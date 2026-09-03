# Zodex Control Plane View

Optional, disabled-by-default Desktop integration for the redesign Control Plane. It reads only sanitized agent identifiers and lifecycle states from an explicitly configured owner-private Unix socket.

The main-process bridge validates that the path is a socket owned by the current user with no group/other permissions. It enrolls as `zodex-desktop` with `inventory`/`status` capabilities only. It never receives prompts, responses, tool output, credentials, tmux handles, or mutation capabilities.
