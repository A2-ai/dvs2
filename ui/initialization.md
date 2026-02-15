# dvs initialization

Goal: Prepare shared storage and initialize DVS in directory

dvs initialization will create a `dvs.toml` and a directory as specified by the
shared area in the init command. The shared dir may also need to chown the directory
to specify certain permissions. For example, for sensitive projects, setting
ownership to a particular group, allowing write access for the group, and limiting
read access to those not in the group.

## Journey 1: Initial Setup with defaults

Expected outcomes:

* dvs.toml created in working directory
* shared dir created in specified path, with default permissions of 664

Known Caveats:

* certain linux umask setups cause folders to have default permissions like 600, or 644
where other collaborators could not write by default, therefore, 

### CLI flow

1. initialize dvs from a project directory

```bash
dvs init /data/dvs/example-proj
```


### R package flow

1. Initialize DVS in the repo

```r
dvs_init("/data/shared/project-x-dvs")
```

## Journey 2: Initial Setup with shared folder locked down to group

* set permissions to writeable by group, not readable if not in group (660)
* group name projx

Expected outcomes:

* dvs.toml created in working directory
* shared dir created in specified path, with permissions of 660 and owned by group projx

Edge cases:

* group must resolve to known guid on system

### CLI flow

1. initialize dvs from a project directory

```bash
dvs init /data/dvs/sensitive-projx --permissions "660" --group projx
```


### R package flow

1. Initialize DVS in the repo

```r
dvs_init("/data/shared/project-x-dvs", permissions = "660", group = "projx")
```



