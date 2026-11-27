---
company: 
role: 
location: 
email: 
aliases: 
tags:
  - person
---
# General notes

---
# Follow-ups
```view
type: tasks
source: 'task-todo:#followup task:"{{title}}"'
columns: [task, file, due]
sort: file
```

---
# 1:1s


---
# Recent Meetings
```view
type: table
source: 'path:MeetingNotes/ [attendees:"[[{{title}}]]"]'
columns: [file, date]
sort: file
```
---
