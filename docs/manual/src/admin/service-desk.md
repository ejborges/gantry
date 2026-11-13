---
title: Service desk setup
summary: Request types, the help desk, service-level targets and the knowledge base.
---

Create a service desk from the **IT service desk** or **Business service desk** template. The template creates the project, its request types, default targets and a knowledge base space. Everything below is under the project's **Settings**.

## Who may raise requests

**Help desk portal** controls who may use this desk.

- Select **Any signed-in user may raise requests here** for a desk that serves everyone with an account: staff and customers alike.
- Leave it off to restrict the desk. Then only people given the **Service desk customer** role under **Members** can use it, individually or through a group.

People whose only workspace role is **Customer** see the help desk and nothing else in Gantry. Staff with the **Member** role can use the help desk as well, for example to ask IT for something. Everyone sees only their own requests there.

## Request types

A request type is what a customer chooses first: *Something is broken*, *I need access*, *Ask a question*. Each has a name written for customers and creates a work item of a particular type behind the scenes (for example *Incident* or *Service request*), which determines the workflow.

Add one by typing its name under **Request types**. Keep the list short and phrased as the customer would say it. Clear **Offered** rather than removing a type that is retired, so that old requests keep their meaning.

## Response and resolution targets

Targets are set per priority, in minutes.

| Priority | First response | Resolved within |
| --- | --- | --- |
| Critical | 30 | 240 |
| High | 60 | 480 |
| Medium | 240 | 1440 |
| Low | 480 | 2880 |

These are examples; set what your team can keep.

**Clock for response and resolution targets** chooses when time counts: *around the clock*, or *working hours* (Monday through Friday, 8:00 AM to 5:00 PM, in the time zone of the administrator who sets it). With working hours, a request raised on Friday at 16:30 with a 60-minute target is due on Monday at 08:30.

When a target is missed the request is marked, the assignee is notified, and a `sla.breached` event is sent to any integration that listens for it, so a missed target can reach a chat channel.

## Agents

Give the people who answer requests the **Service agent** role in the project. It lets them reply to customers, write internal notes, and assign and move requests. An AI agent can hold the same role, for example to label and route new requests.

## Knowledge base

The desk's space is of the *knowledge base* kind. Pages marked **Shown in the help desk** are suggested to customers while they type a request. Start with the ten questions you answer most often. A short article with the exact steps prevents more requests than a long one.
