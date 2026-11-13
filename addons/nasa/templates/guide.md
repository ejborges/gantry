> This project was created from the **NASA space flight project** template. It gives you the structure of
> NPR 7120.5 (program and project management) and NPR 7123.1 (systems engineering) as work items and pages.
> It is a starting point. The directives expect tailoring: adjust it to your project's category and class,
> record what you changed in the [Compliance and tailoring matrix], and check requirements against the
> current revisions on NODIS. Nothing here replaces the directives.

## Life cycle

| Phase | Purpose | Reviews in this phase | Gate out of the phase |
|---|---|---|---|
| Pre-Phase A | Concept Studies | MCR | KDP A |
| Phase A | Concept and Technology Development | SRR, MDR or SDR | KDP B |
| Phase B | Preliminary Design and Technology Completion | PDR | KDP C |
| Phase C | Final Design and Fabrication | CDR, PRR, SIR | KDP D |
| Phase D | System Assembly, Integration and Test, Launch and Checkout | TRR, SAR, ORR, FRR or MRR | KDP E |
| Phase E | Operations and Sustainment | PLAR, CERR, PFAR, DR | KDP F |
| Phase F | Closeout | DRR | — |

Each phase is a **Milestone** work item. Its reviews (**Life-cycle review**) and its gate (**Key decision point**)
are child items, so the phase shows progress as they close.

## What each work item type is for

| Type | Use it for | Workflow highlights |
|---|---|---|
| Requirement | One verifiable shall statement | Cannot be baselined without a rationale; cannot be verified without evidence |
| Verification activity | A test, analysis, inspection or demonstration | Link to requirements with **verifies** |
| Life-cycle review | SRR, PDR, CDR and the rest | Needs a date to start and a board outcome to close |
| RID | A discrepancy raised against review material | Screening → accepted → response → closed with concurrence |
| Key decision point | The decision authority's gate | Needs a recorded decision |
| Waiver or deviation | Relief from, or tailoring of, a requirement | Needs the source requirement and a justification |
| Risk (5x5) | Likelihood and consequence, 1–5 each | Needs scores to assess and a plan to mitigate |
| Controlled product | A plan or document to baseline | Draft → review → baselined, or tailored out with justification |
| Technology development | A technology being matured | Tracks TRL |
| Lesson learned | What to do differently next time | — |

## Traceability

Use links between items to build the trace, then open **Graph** on any item to see it.

| From | Relation | To |
|---|---|---|
| Requirement | `derives_from` | Parent requirement |
| Verification activity | `verifies` | Requirement |
| Risk (5x5) | `threatens` | Requirement, milestone or review |
| RID | `raised_at` | Life-cycle review |
| Waiver or deviation | `waives` | Requirement |
| Change request | `changes` | Requirement or controlled product |

## Where the documents live

Plans are pages in this space, each tracked by a **Controlled product** item so its state (draft, in review,
baselined) is visible on the board. Page history keeps every saved version with its author and date.
