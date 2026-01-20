# Journey 1: Initial Setup

Goal: Prepare shared storage and initialize DVS in a Git repo.

## CLI flow
1. Create and secure shared storage
   ```bash
   mkdir -p /data/shared/project-x-dvs
   chgrp project-team /data/shared/project-x-dvs
   chmod 2770 /data/shared/project-x-dvs
   ```
2. Initialize DVS in the repo
   ```bash
   dvs init /data/shared/project-x-dvs --permissions 664 --group project-team
   ```
3. Commit configuration
   ```bash
   git add dvs.yaml
   git commit -m "Initialize DVS"
   git push
   ```

## R package flow
1. Initialize DVS in the repo
   ```r
   dvs_init("/data/shared/project-x-dvs", permissions = 664, group = "project-team")
   ```
2. Commit configuration
   ```bash
   git add dvs.yaml
   git commit -m "Initialize DVS"
   git push
   ```
