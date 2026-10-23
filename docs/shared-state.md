# Shared state

The dev databases and `/data` (including user home directories) are shared by every
cluster that uses them. A change to one reaches all of those clusters at once, whatever
version each one runs.

## When this applies

- Adds a DB migration.
- Changes anything on the shared data filesystem (`/data`, or the cluster's configured
  data directory): creating, writing, deleting, moving, chmodding or chowning files.

## What to do

1. Add the `shared-state` label to the PR.
2. Tell the developer to post in #2-tech-devops before deploying anywhere: what changes
   and which clusters share it.
3. Prefer an additive form (nullable column, backfill, enforce later) so clusters that
   haven't upgraded keep working. If it can't be additive, upgrade every cluster on the
   shared resource together.
4. Patch releases can't carry a breaking shared-state change.

## Also call out in the PR description

When nothing above applies, these only affect the cluster you deploy to and don't need
the label:

- Adds or renames a config key the cookbook writes.
- Needs a matching cookbook or peer-component change.
- Restarts services or drops sessions when installed.

Background: [Cluster compatibility guardrails](https://a2-ai.atlassian.net/wiki/spaces/~712020021369d67a7547c79bfc25d4fa03a0ee/pages/2542960646)
