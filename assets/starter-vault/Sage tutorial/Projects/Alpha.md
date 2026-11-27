---
title: Project Alpha
tags: [project/alpha]
status: active
---
# Project Alpha

Migration plan for the legacy billing service. See [[Architecture notes]].

## Tasks

- [ ] Draft the cutover runbook #status/in-progress 📅 2026-12-01 ⏫
- [ ] Review APIM policies with the team #status/backlog
- [x] Inventory existing consumers ✅ 2026-11-10

> Keep the old queue alive until every subscriber is confirmed.

```view
type: kanban
source: "tag:#project/alpha"
group_by: status
columns: [Backlog, In Progress, Done]
```
