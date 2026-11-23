# Tasks dashboard

## Open, by due date

Task notes and checkbox lines from every note. Checking a box here checks it in its note.

```tasks
status != done
status != cancelled
sort due
```

## Task notes in Tasks/

```view
type: table
source: "path:Tasks/ [type:task] -[status:done]"
columns: [file, title, status, due, tags]
sort: due
```
[[testing-task-923942343]]