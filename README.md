# `dvs`

## note: this is a "develop-in-the-open" active rewrite targetting stabilization in ~March 2026, for the moment expect major breakage/instability as we
## consolidate the feature set we want to support in dvs for this next release.
## please contact us if these ideas or discussion points are of interest, we'd love to get more collaborative discussions going

Rewrite of `dvs`, the data-version-control system made by A2-AI.

DVS (Data Version System) is a tool for versioning large or sensitive files under Git without tracking the file content directly.


## TODOs

- Azure backend
- GC?
- dvs remove?
- integrity check? would need to read the file again after saving it
- compression?
- migrate from dvs1
