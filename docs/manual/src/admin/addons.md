---
title: Add-ons
summary: Installing content packs for a discipline and understanding what they change.
---

An add-on is a **pack**: a set of fields, workflows, work item types, project templates and page templates for a particular way of working. Packs only add definitions; they contain no program code.

Open **Administration**, **Add-ons**.

- **Installed** lists the packs in use, with their versions.
- **Available on this server** lists the packs the system administrator has placed on the server. Press **Install**, or **Update to …** when a newer version is on disk.

Add-ons delivered as system packages, such as the NASA add-on, are installed for you when the package is installed. If one is missing from the list, ask your system administrator to install its package; see [Installing](../sysadmin/install.md#optional-add-ons).

## What installing does

The pack's definitions appear under [Types and workflows](schemes.md) marked with the add-on's name, and its templates appear when creating projects and pages. Nothing existing changes: your projects keep their types until you add the new ones to them.

## Updating and your changes

Updating a pack refreshes the definitions that are still as the pack delivered them. Any definition you edited is marked *Customized* and is left alone, so a local adjustment to a workflow survives every update.

## Removing

Packs are not uninstalled, because items and projects may depend on their definitions. To retire a type, remove it from the projects that offer it.

## Writing your own

A pack is a single JSON file. If you maintain several Gantry installations, or want your own conventions under version control, see [Packs](../reference/packs.md).
