---
title: Working with the NASA add-on
nav: Working with it
summary: Day-to-day use: requirements and traceability, verification, reviews and RIDs, decision points, risks and controlled documents.
---

## Start a project

1. **Projects**, **New project**, template **{{nasa_template_name}}**.
2. Open the project library (the project's documentation space) and read **NASA project guide**. Edit it to describe your own conventions.
3. Fill in the **Compliance and tailoring matrix** with your program's tailoring. This is the record of what applies to you.
4. Open each phase on the list to see its reviews and decision point as sub-items; give each a date and an owner.

## Requirements and traceability

Create a **Requirement** for each *shall* statement.

- Put the statement in **Requirement (shall statement)**, one requirement per item, and give the item a short title.
- Set the **requirement level** (Level 1, program, down to Level 5, component), the **source**, the **rationale** and the **verification method** (test, analysis, inspection or demonstration).
- Link a child to its parent with **derives from**.

The workflow enforces the order of work: a requirement cannot be submitted for review without its statement and verification method, cannot be baselined without a rationale, and cannot be recorded as verified without verification evidence.

The requirement's page shows its parent, its children and what verifies it. From any requirement you can follow the chain up to the need it serves and down to the evidence that it is met; `GET /api/v1/items/KEY/graph` and the MCP tool `get_item_graph` return the same traceability for scripts and agents, for example to list requirements that nothing verifies.

**After the baseline**, every change to a requirement is in its history with who made it and when; use your project's change process (a *change request* item linked with **changes**) to record the approval.

**Departures.** Raise a **Waiver or deviation**, choose whether it is a waiver, a deviation or a tailoring request, and link it to the requirement with **waives**. It cannot be submitted without the requirement's source and a justification; approval and disapproval are separate final statuses.

## Verification

Create a **Verification activity** for each test, analysis, inspection or demonstration, and link it to the requirements it closes with **verifies**. It ends as *Passed* or *Failed*; recording a pass needs the **verification evidence** (the report, its location, the data).

A simple verification matrix is the list view filtered to requirements, with the *verified by* links on each; export it through the API for a formal matrix.

## Reviews and RIDs

For each life-cycle review:

1. Open the review item created by the template (or create one and choose the **review**: MCR, SRR, MDR, SDR, PDR, CDR, PRR, SIR, TRR, SAR, ORR, FRR, MRR, PLAR, CERR, PFAR, DR, DRR or another).
2. Set the **review date**; the readiness assessment cannot begin without it.
3. In the project library create a page from **Life-cycle review package** and link the review to it with **documented in**. The template lists purpose, entrance criteria and success criteria for you to tailor.
4. During the review, raise a **RID** for each discrepancy, with the originating organization, the affected documents and the recommended action, and link it to the review with **raised at**. The review's page then lists its RIDs and their status.
5. Screen and disposition each RID: accepted RIDs are worked and closed with the **corrective action** recorded; others are rejected or withdrawn.
6. Record the **board outcome** (success criteria met, met with actions, or not met with a delta review required), work off the closing actions, and complete the review.

## Key decision points

A **Key decision point** item records the gate at the end of a phase. Recording the decision needs the **KDP decision**: approved to proceed, approved with conditions, not approved with rework directed, or terminated. Link the preceding reviews to it so that the basis of the decision can be followed.

## Risks

**Risk (5x5)** items carry likelihood and consequence from 1 to 5, the **handling approach** (mitigate, watch, research, accept, elevate or close), the mitigation, and cost and schedule impact. Mitigation tasks link to the risk with **mitigates**. Sort the list by likelihood and consequence for the risk review; describe the scales and the matrix in the **Risk Management Plan**.

## Controlled products

Each plan or specification the project must produce is a **Controlled product** that moves from *Not started* through *Draft* and *In review* to *Baselined*. A product your tailoring does not require is marked *Not applicable (tailored out)*, which needs a justification, so that the omission is a recorded decision.

Keep the document itself as a page in the project library, or attach the released file, and link the item to it with **documented in**. Page history gives you the revision record; the item's workflow gives you the approval record.

## Technology and lessons

**Technology development** items carry a technology readiness level from 1 to 9. **Lesson learned** items record an observation and its recommendation during the project, ready to be submitted to your organization's lessons learned system at closeout.

## Adapting it

Everything the add-on installs is ordinary Gantry configuration. An administrator can rename statuses, add fields (a WBS element, a mission directorate, a hazard report number), add steps that need an approver, or restrict who may take the *Approve* steps by role. Changes you make are kept when the add-on is updated: see [Add-ons](../admin/addons.md#updating-and-your-changes).

For work that is not a space flight project (research and technology under NPR 7120.8, information technology under NPR 7120.7), start from the same types and adjust the template's phases and reviews to the governing document.
