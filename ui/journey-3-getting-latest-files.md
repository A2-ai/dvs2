# Journey 3: Getting Latest Files

Goal: Pull metadata from Git and restore the tracked data files.

## CLI flow
1. Pull the latest repo changes
   ```bash
   git pull
   ```
2. See what is missing
   ```bash
   dvs status
   ```
3. Restore tracked files
   ```bash
   dvs get data/derived/*
   ```
4. Verify everything is current
   ```bash
   dvs status
   ```

## R package flow
1. Pull the latest repo changes
   ```bash
   git pull
   ```
2. See what is missing
   ```r
   dvs_status()
   ```
3. Restore tracked files
   ```r
   dvs_get("data/derived/*")
   ```
4. Verify everything is current
   ```r
   dvs_status()
   ```
