# Lightweight container engine (WIP)

A simplistic but lightweight [OCI](https://opencontainers.org/)-compatible
container engine.

## Getting started

To build this project on Linux, you will need the latest version of the
[Rust toolchain](https://www.rust-lang.org/) (at least version 1.46.0, but
1.48.0 compiles async code faster, as provided in the `rust-toolchain` file).

The following binaries will also need to be available in `$PATH`:

* [containers/skopeo], for fetching container images from remote registries.
* [containers/umoci], for unpacking fetched OCI images into runtime bundles.
* [containers/crun], for instantiating containers.
* [containers/conmon], for monitoring running containers.

[containers/skopeo]: https://github.com/containers/skopeo
[containers/umoci]: https://github.com/opencontainers/umoci
[containers/crun]: https://github.com/containers/crun
[containers/conmon]: https://github.com/containers/conmon

To compile the service in debug mode and start it, simply run the following
command in your terminal:

```sh
cargo run
```

To execute the included unit test suite, run:

```sh
cargo test
```

## Usage

TODO

## Project layout

TODO

## Assumptions

* This service will run on a Linux system with a system allocator available.
* This service will run in a resource-constrained environment, and as such,
  efforts should be made where possible to handle out-of-memory errors (within
  reason, as this capacity in stable Rust is currently limited).
* This service will run on hardware which fully supports atomics.
* This service will be queried by multiple clients at once and may require some
  form of async concurrency in order to scale efficiently without relying too
  heavily on OS threads.
* Containers will be referenced by their name/tag pair, as per the Google Doc,
  meaning only one unique instance of this combination can be created at any
  given time.
* Containers will _not_ persist in between individual runs of the application.

## Possible improvements

* Find (or write) an alternative async executor which allows for explicit
  handling out-of-memory errors.
* Try to eliminate more hidden heap allocator calls by either swapping out
  dependencies for `#![no_std]` alternatives, or rewriting certain functionality
  ourselves with adequate tests.
* TODO: Add more as we go along...

## Troubleshooting

This container engine leverages [rootless containers] for increased security,
convenience, and flexibility. Like other rootless container engines, e.g.
[rootless Podman], there are several Linux kernel features that must be
available for this engine to run:

[rootless containers]: https://rootlesscontaine.rs/
[rootless Podman]: https://github.com/containers/podman/blob/master/rootless.md

### 1) System must have `/etc/subuid` and `/etc/subguid`

> Required for _container creation_

If you see an error like this when starting containers:

```text
writing file `/proc/7109/gid_map`: Invalid argument
setresuid(0): Invalid argument
```

then it is possible that your Linux distribution may not come with `/etc/subuid`
and/or `etc/subgid` files, or perhaps they are configured incorrectly. For
example, Arch Linux's version of `shadow` does not come with either file by
default.

If neither `/etc/subuid` nor `/etc/subgid` exist, you can create them like so:

```bash
USERNAME=$(whoami) # Alternatively, use a user group that you belong to.
echo "$USERNAME:165536-169631" | sudo tee /etc/subuid /etc/subgid
```

On the other hand, if both `/etc/subuid` and `/etc/subgid` exist, but your user
or member group is missing, you can dedicate a range of sub-UIDs and GIDs to
yourself for use with `light-containerd`:

```bash
USERNAME=$(whoami) # Alternatively, use a user group that you belong to.
sudo usermod --add-subuids 165536-169631 --add-subgids 165536-169631 "$USERNAME"
```

### 2) System must support `cgroup` V2

> Required for _container pause/resume_

At the time of writing, only Fedora Linux ≥31 adopts `cgroup` V2 by default.
Provided you are running `systemd` ≥226 with Linux ≥4.2, you may add the
following kernel boot parameter and restart to enable `cgroup` V2:

```text
systemd.unified_cgroup_hierarchy=1
```

This mounts both `cgroupfs` and `cgroupfs2` in a unified filesystem hierarchy,
safely allowing any existing `cgroup` V1 applications to continue working.
