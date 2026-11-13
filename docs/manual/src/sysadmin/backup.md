---
title: Backup and restore
summary: What to back up, the script that does it, and how to restore onto the same or a new machine.
---

## What must be backed up

| What | Where | Why |
| --- | --- | --- |
| The database | PostgreSQL database `gantry` | Everything: work, pages, accounts, audit log. |
| Attachments | `/var/lib/gantry/blobs`, or your object store | Files people uploaded. |
| Configuration and the secret key | `/etc/gantry` | Without the key, stored integration and single sign-on secrets cannot be decrypted. |

## Nightly backup

```bash
systemctl enable --now gantry-backup.timer
```

Each night `gantry-backup` writes a new directory under `/var/backups/gantry`, readable by root only:

```
/var/backups/gantry/20261110T021500Z/
  database.dump       pg_dump custom format
  blobs.tar.gz        attachments (when stored on local disk)
  etc-gantry.tar.gz   configuration and secret key
  versions.txt        installed package versions
  SHA256SUMS
```

The dump is taken in one transaction while the service runs; nobody is interrupted. The fourteen most recent backups are kept; change that with `GANTRY_BACKUP_KEEP` in a drop-in for the unit (`systemctl edit gantry-backup.service`, then `Environment=GANTRY_BACKUP_KEEP=30`).

Run one by hand at any time, for example before an upgrade:

```bash
gantry-backup
```

> **Important**: a backup on the same machine protects against mistakes, not against losing the machine. Copy `/var/backups/gantry` elsewhere every night, to storage that only administrators can read: the backup contains the secret key and every account's data.

When the database is on another server and its address carries a password, set `GANTRY_BACKUP_DATABASE_URL` for the unit in the same way, or give the `gantry` account a `~/.pgpass` entry. When attachments are in an object store, the script says so and skips them; back the bucket up with the store's own tools.

For a recovery point finer than one day, add PostgreSQL's own continuous archiving (WAL archiving with `pg_basebackup`, or a tool such as pgBackRest) alongside this script.

## Restore

These steps restore onto the same machine or a fresh one. They replace the current content.

1. Install the same Gantry version as in `versions.txt` (see [Installing](install.md)) and stop it:

   ```bash
   systemctl stop gantry gantry-worker
   ```

2. Restore the configuration and key:

   ```bash
   cd /var/backups/gantry/20261110T021500Z
   sha256sum -c SHA256SUMS
   tar -C /etc -xzf etc-gantry.tar.gz
   ```

3. Restore the database into an empty database:

   ```bash
   runuser -u postgres -- dropdb --if-exists gantry
   runuser -u postgres -- createdb --owner=gantry --encoding=UTF8 --template=template0 gantry
   runuser -u postgres -- psql -d gantry -c 'CREATE EXTENSION IF NOT EXISTS pg_trgm'
   runuser -u gantry -- pg_restore --no-owner --dbname=gantry database.dump
   ```

   On a new machine run `gantry-setup --hostname …` first: it prepares PostgreSQL, nginx and the firewall, and you then replace the empty database as above.

4. Restore attachments:

   ```bash
   rm -rf /var/lib/gantry/blobs && mkdir -p /var/lib/gantry/blobs
   tar -C /var/lib/gantry/blobs -xzf blobs.tar.gz
   chown -R gantry:gantry /var/lib/gantry
   restorecon -R /var/lib/gantry
   ```

5. Start and check:

   ```bash
   systemctl start gantry
   gantry doctor
   gantry audit verify
   ```

   If you use OpenSearch, rebuild its index: `gantry reindex`.

Rehearse this on a spare machine. A backup that has never been restored is a hope, not a backup.
