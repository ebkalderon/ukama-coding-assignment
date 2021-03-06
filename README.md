# Lightweight container engine

A very basic but lightweight [OCI](https://opencontainers.org/)-compatible
container engine.

## Getting started

To build this project on Linux, you will need a recent version of the
[Rust toolchain](https://www.rust-lang.org/): at least version 1.46.0, at
minimum, but 1.48.0 is strongly recommended because it performs better with
async code ([rust-lang/rust#78410]).

[rust-lang/rust#78410]: https://github.com/rust-lang/rust/pull/78410

The following binaries will also need to be available in `$PATH`:

* [containers/skopeo], for fetching container images from remote registries.
* [containers/umoci], for unpacking fetched OCI images into runtime bundles.
* [containers/crun], for instantiating and managing containers.
* [containers/conmon], for monitoring running containers.

[containers/skopeo]: https://github.com/containers/skopeo
[containers/umoci]: https://github.com/opencontainers/umoci
[containers/crun]: https://github.com/containers/crun
[containers/conmon]: https://github.com/containers/conmon

To compile the service in debug mode and start it, simply run one of the
following commands in your terminal:

```sh
# Serve on port 8080 by default
cargo run

# Serve on port 8080 with `info` logging
RUST_LOG=light_containerd=info cargo run

# Serve on alternate port
cargo run -- --port 4321
```

To execute the included unit test suite, run:

```sh
cargo test
```

To generate HTML documentation for the public crate API, run:

```sh
cargo doc --open
```

## Usage

The engine exposes a simplistic REST API for managing the container lifecycle:

### Endpoints

Route                           | Request body             | Description
--------------------------------|--------------------------|-----------------------------
`PUT /containers/<name>`        |                          | Fetch/create container
`GET /containers/<name>`        |                          | Get container status as JSON
`DELETE /containers/<name>`     |                          | Delete container
`PUT /containers/<name>/status` | `{ "state": "paused" }`  | Pause container execution
`PUT /containers/<name>/status` | `{ "state": "running" }` | Resume container execution

## Project layout

Like many idiomatic Rust projects, this service is split into a binary crate
(the `main.rs` file) and a library crate (`lib.rs` and the rest). This is to
facilitate simpler unit and integration testing under Cargo, should we require
it in the future.

The `main.rs` is a very thin shim over `light_containerd::Engine::serve()`,
which spawns an asynchronous service on the given TCP address.

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
* Find a lighter alternative to `warp` which uses `smol` instead of `tokio`.
* Try to eliminate more hidden heap allocator calls by either swapping out
  dependencies for `#![no_std]` alternatives, or rewriting certain functionality
  ourselves with adequate tests. Robust fallible memory allocation is
  unfortunately not available in the Rust standard library at the moment (at
  least, not on stable), so the `fallible-collections` wrapper crate should do
  the trick for now until stabilization.
* Add `conman` to a `cgroup` (V2) before spawning, which would allow us to
  gather rich service metrics about memory usage, CPU usage, thresholds, etc.
  We also would need to leverage this `cgroup` to enforce OOM limits the
  container itself ([see `internal/oci/runtime_oci.go` from CRI-O][rt_oci]).
* Add caching system for the `state()` method by storing some information on the
  `Container` side.
* Re-architect `Engine::new()` such that we can avoid using `tokio::spawn()` for
  running the signal handler task and rely on lighter primitives like `join!()`
  instead.
* Improve quality and moderate the frequency of log messages.
* Make the service generic over both TCP and UDS streams, so we may be able to
  write some automated integration or E2E tests in the future.

[rt_oci]: https://github.com/cri-o/cri-o/blob/f3390f3464d76c4b0dbaf565ba1fca3b67464276/internal/oci/runtime_oci.go#L1190-L1218

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
and/or `/etc/subgid` files, or perhaps they are configured incorrectly. For
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
