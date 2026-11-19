# MCP server

> **Status**: Draft. Only its place in the architecture is decided. The detailed design will be written when it follows the process for a new feature. For the overall structure, see [AI backend architecture](0-ai-architecture.md).
>
> **Related**: #259

MCP (Model Context Protocol) is the protocol an AI Agent uses to call external tools. After Hindsight becomes an MCP server, external Agents such as Claude Desktop and Cursor can look up the data Hindsight recorded. They can also save the daily report they have written back into Hindsight. The user can then see it on the daily report page (#259).

---

## 1. Its place in the architecture

It is one entry point of ①. It does not go through ②. The model is in the external app. Hindsight does not call the model.

| What the external Agent does | Which part of Hindsight is used |
|---|---|
| Search titles | `Tools`, the same definition as chat |
| Get the material for writing a daily report | `ReportContext`, the same one Hindsight itself uses to write a daily report |
| Save the daily report back | The same place where the daily report entry point writes to the database |

- Search titles and getting material both go through the same `AiDataPolicy`. What the external Agent sees is the same as what Hindsight's own model sees.
- It can only write daily reports. Activity records, categories, and settings cannot be changed.
- Search titles uses a read-only database connection. Writing a daily report is a separate place. It only touches the daily report table.

---

## 2. Two forms

| | stdio | Local HTTP |
|---|---|---|
| How to connect | The external Agent starts the Hindsight program with the `--mcp` parameter. Communication uses standard input and output. | The external Agent connects to a port on `127.0.0.1`. |
| Does Hindsight need to be running | No | Yes |
| Authentication | Not needed. Whoever can start the program is the user of this machine. | Needed. Other programs on this machine can also connect to this port, and it can write. |
| Existing example | The same program already has an `--ocr-worker` mode. | None |

---

## 3. Daily reports written back

- **Where it is stored**: It is stored in the same table as the daily reports Hindsight itself writes. The daily report page shows it as usual. If the user has turned on daily report sync, it syncs as usual too.
- **Who wrote it**: The table already has a field that records which model wrote it. When an external Agent writes, record the name it reports for itself (proposal).
- Whether to store by segment or by day, and what to do when a daily report already exists, see [To be decided](#4-to-be-decided) 4 and 5.

---

## 4. To be decided

1. Use stdio or local HTTP?
2. On or off by default? External Agents usually use a cloud model. The data that is looked up will leave this machine.
3. Should what the external Agent can see be less than what Hindsight's own model can see? For example, the full text in screenshots is not given by default.
4. Should the daily report written back be stored by segment or by day? An external Agent usually writes a whole day each time. Hindsight now stores by segment.
5. When that day already has a daily report: overwrite, keep both, or reject?
6. Should we use the prompts feature of MCP, and make "Write today's daily report" a command that can be chosen directly in the external app?
7. Later, should we make an MCP client, so Hindsight's own model can call external tools?
