---
title: NASA add-on
nav: Overview
summary: Project management and systems engineering content aligned with NPR 7120.5 and NPR 7123.1, for teams that run projects the NASA way.
---

The NASA add-on is an optional package (`gantry-addon-nasa`). It adds work item types, workflows, a project template and document templates that follow two NASA procedural requirements and the handbook that explains them:

| Document | Subject | Used here for |
| --- | --- | --- |
| NPR 7120.5 | NASA Space Flight Program and Project Management Requirements | Life-cycle phases, key decision points, life-cycle reviews, the Project Plan and control plans. |
| NPR 7123.1 | NASA Systems Engineering Processes and Requirements | The 17 common technical processes, the SEMP, verification and validation. |
| NASA/SP-2016-6105 | NASA Systems Engineering Handbook | Review purposes and entrance and success criteria; guidance text in templates. |

> **Important**: the add-on gives you structure and templates. It does not make a project compliant, and it is not a NASA product. Your program's own tailoring, approved by its governing authority, decides what applies; always work from the current revision of each document in the NASA Online Directives Information System (NODIS). Where a template quotes a requirement number, check it against the revision you are bound to.

## Installing

A system administrator installs the package; see [Installing](../sysadmin/install.md#optional-add-ons). The content is then listed under **Administration**, **Add-ons**, and its definitions appear under **Types and workflows** marked *NASA*.

## What it adds

### Work item types

{{nasa_types}}

### Workflows

{{nasa_workflows}}

### Fields

{{nasa_fields}}

### Relations between items

The add-on uses the standard link relations so that traceability can be followed in both directions.

| Relation | Typical use |
| --- | --- |
| derives from / source of | A child requirement to its parent. |
| verifies / verified by | A verification activity to the requirements it closes. |
| raised at / raised here | A RID to its review. |
| waives / waived by | A waiver to the requirement it departs from. |
| mitigates, threatens | Actions and risks. |
| documented in / documents | An item to the controlled product or page that records it. |

### The project template

Creating a project from **{{nasa_template_name}}** lays the life cycle out as work, with each phase holding its life-cycle reviews and the key decision point that ends it, and the plans and control documents listed as controlled products ready to be assigned and scheduled:

{{nasa_template_items}}

It also creates a documentation space, the **project library**, with a guide and the templates below.

### Document templates

{{nasa_page_templates}}

Next: [working with the add-on](working.md).
