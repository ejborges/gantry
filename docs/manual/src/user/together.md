---
title: Working at the same time
nav: Working together, live
summary: What you see when colleagues are looking at or editing the same thing as you, and what happens to everyone's changes.
---

Gantry is live. When someone changes something you are looking at, your screen changes too, and you can see who else is there. No reload is needed.

## Who else is here

Small account pictures appear at the top of a page when other people have it open. Hover over one for the name and what that person is doing; an AI agent's picture carries the robot mark.

| Where | What you see of others |
| --- | --- |
| A work item | Who has it open. Next to a field: who is changing that field right now. In the description: their cursor, with their name. |
| A documentation page in the editor | Their cursor or selection in the text, with their name, in a color of their own. |
| A discussion | Who is writing a message right now. New messages appear as they are posted. |
| A project | Who has it open and which view they are on: board, list, Gantt chart, settings and so on. Cards move as people move them. |
| A dashboard | Who has it open, and which tile someone is moving, resizing or configuring. |
| A visualization's script | Their cursor in the script. |
| A Gantt chart | Which bar someone has hold of. |

Presence shows only people who are allowed to see the thing in question, and it shows them only to others who are allowed to see it.

## What happens to changes

**Text that is edited together.** Page content, a work item's description and a visualization's script are *shared documents*: everyone who has the editor open types into the same text, letter by letter, and all the changes are kept. Two people can write in the same paragraph without losing anything. The shared text becomes the saved version when someone saves it: **Review and save** for a page, **Save description** for an item, **Save** for a script. Until then it is a draft that everyone who opens the editor sees and continues; **Discard changes** throws the draft away and returns to the saved text.

**Single fields.** A field such as status, assignee, priority or a date holds one value. If two people change it, the later change wins, both changes are in the item's history with who and when, and both people see the result at once. The marker next to a field that someone else is changing is there so you can wait a moment instead.

**Boards, dashboards and Gantt charts.** Each move is saved as it is made and appears for everyone. If two people move the same card or tile, the later move wins. On a dashboard, tiles that would overlap after someone else's move are pushed down, never hidden.

**Workflows and project details** are saved as a whole. If someone else saved while you were editing, your save is refused with a message that says so, and your version stays on screen so you can compare and try again.

## Lists that keep up

My work, project lists, boards, queues, the hierarchy graph, the people directory, team pages and dashboards refresh themselves when something they show changes. The dot next to *The graph updates by itself when work changes* on the graph, and similar notes elsewhere, tell you the live connection is working.

## When the connection drops

The live connection is separate from ordinary page loading. When it is interrupted (a laptop waking up, a train tunnel, a proxy that closes idle connections) Gantry reconnects by itself, a little later after each failed attempt, and fetches what you missed.

While it is down, a small badge in the top bar reads **Live updates paused**, and:

- everything still works the ordinary way: you can read, edit and save;
- you do not see others' changes until you reload or the connection returns;
- an editor that was shared tells you *The live connection is not available, so you are editing on your own*. Saving then works as described under [Several people, one page](docs.md#several-people-one-page): if someone else saved first, your save is refused and your text stays put.

If the live connection never works for you while everything else does, tell your administrator: the web server in front of Gantry may not be passing WebSocket connections through.

## Limits

One account can hold twelve live connections at a time (tabs and devices together); your administrator can change the number. Opening more tabs than that leaves the extra ones working without live updates.
