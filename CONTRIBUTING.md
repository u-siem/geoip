# Contributing

Thanks for helping. Issues and pull requests are welcome.

## Before you open a pull request

CI runs all of these; running them first saves a round trip.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo deny check                # https://github.com/EmbarkStudios/cargo-deny
```

The minimum supported Rust version is 1.89. CI tests it, along with Linux,
Windows and macOS.

## Working against a local uSIEM checkout

This crate depends on [uSIEM](https://github.com/u-siem/usiem) by git, pinned
to a commit in `Cargo.toml`. To build against a checkout next to this one
(`../usiem`) instead, override the source on the command line:

```sh
cargo test --config "patch.'https://github.com/u-siem/usiem'.usiem.path='../usiem'"
```

That rewrites `Cargo.lock` to point at the local path. Never commit that
lockfile: restore it with `git checkout Cargo.lock` before you commit. When a
change needs a newer uSIEM, bump the `rev` in `Cargo.toml` and run
`cargo update -p usiem` in the same pull request.

## Test data

The tests are self-contained and use no network. `tests/data` holds MaxMind's
synthetic test databases from
[maxmind/MaxMind-DB](https://github.com/maxmind/MaxMind-DB) (MIT or
Apache-2.0, see `tests/data/README.md`); a few tests also build tiny databases
of their own. Never add real GeoLite2, GeoIP2 or DB-IP data to the
repository: their licenses do not allow it.

## Fuzzing

There are no fuzz targets. The only untrusted input is the `.mmdb` file
itself, and the `maxminddb` crate does all of its parsing; this crate reads
the decoded values through `serde`.

## Releases

The crate is distributed from GitHub only. Maintainers release by bumping
`version` in `Cargo.toml`, moving the `Unreleased` changelog entries under the
new version, and pushing a tag that is exactly the version, with no prefix:
`git tag 0.2.0 && git push origin 0.2.0`. The release workflow checks the
tag, runs CI and creates the GitHub release.
