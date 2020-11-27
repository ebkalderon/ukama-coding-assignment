# Lightweight container engine (WIP)

A simplistic but lightweight [OCI](https://opencontainers.org/)-compatible
container engine.

## Getting started

To build this project on Linux, you will need the latest version of the
[Rust toolchain](https://www.rust-lang.org/) (at least version 1.46.0, as
provided with the `rust-toolchain` file).

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
* Containers will be referenced by their name/tag pair, as per the design
  document, meaning only one unique instance of this combination can be created
  at any given time.
* Containers will not persist in between individual runs of the application.

## Possible improvements

* Find (or write) an alternative async executor which allows for explicit
  handling out-of-memory errors.
* Try to eliminate more hidden heap allocator calls by either swapping out
  dependencies for `#![no_std]` alternatives, or rewriting certain functionality
  ourselves with adequate tests.
* TODO: Add more as we go along...
