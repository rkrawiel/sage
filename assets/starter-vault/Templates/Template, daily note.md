# Notes



---
# Tasks due today
```tasks
status != done
status != cancelled
due <= {{date:YYYY-MM-DD}}
sort due
```
---
# Daily
- 
---
# Meetings from today
```view
type: table
source: "path:MeetingNotes/ [date:{{date:YYYY-MM-DD}}]"
columns: [file]
sort: file
```
---
# Tasks completed today
```view
type: tasks
source: "task-done: done:{{date:YYYY-MM-DD}}"
columns: [task, file]
```
---