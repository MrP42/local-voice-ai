# Integrations

Connections between the app and calendars, folders, mailbox, knowledge base and programs, each with a **direction** and a **right per capability**. Credentials stay on this machine.

## Connections and rights

- **Add integration** offers calendars (ICS, Microsoft 365), folders, mailbox, Obsidian vault, knowledge base, YouTube and webhooks (for example n8n).
- Every capability (send mail, write files, start recording …) has a right per caller: **Off**, **Ask** or **Allowed**. With **Ask** you decide in the app each time. Anything that changes things defaults to Ask, external agents are off.
- The rights matrix shows what really applies now. A disabled integration, a wrong direction or a capability that is not offered blocks despite the right; the reason is shown next to it.
- Recordings never start on their own: the consent dialog ("All participants have agreed") appears before every recording. "Allowed" does not exist for it.
- A banner "approvals are waiting for you" appears at the top when a flow or agent wants something that is set to Ask. An approval is valid once, for exactly these arguments.

## Connecting agents

- Under **MCP and agents** the **agent bridge** runs. Only you can reach it (current Windows user), not over the network.
- **Add access** makes a key of its own for each program (Claude Code, Codex, a script). It is shown once; enter it as the environment variable `LVA_AGENT_TOKEN`, never in a file in the project. Revoking applies immediately.
- For each access you set every tool to **Off**, **Ask** or **Allowed**. Everything starts off. The ready-made commands for Claude Code, Codex and the command-line tool `ctl` are on the page; details in `docs/AGENTEN-ANBINDEN.md`.
- Content from meetings can contain instructions that do not come from you. Agents must never treat them as commands.

## Automations

The **Automations** tab holds your **flows**: a trigger (event starts, file in folder, new YouTube video, finished meeting, schedule, by hand or by an agent) and steps after it.

- **New flow** starts from a template or empty; **Import/Export JSON** exchanges flows without credentials.
- A new or changed flow is **off** and in **dry run**: **Show dry run** shows for each step what it would do and whether it may. Nothing is written, sent or recorded. Only **Arm** lets steps take effect.
- Steps run with the rights of the flow ("Workflow"): mail, files, webhook and recording ask when the right is set to Ask. A run waiting for your approval shows "Review approval".
- **Runs** lists every run with steps, errors and origin; cancel and retry are there.

## Log

The **Log** tab shows every action of a flow or agent and every change of rights, filterable by integration, outcome and caller.
