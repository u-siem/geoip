# Security policy

Database files often come from a download job, and lookups run on addresses
taken from untrusted log data. A panic, unbounded memory or CPU use, or a
wrong answer on a malformed database or an odd address is a security issue.

The memory map is a documented caller contract, not a bug: a file must not be
modified or truncated while an `MmdbFile` has it open (see the crate docs).

## Reporting a vulnerability

Please report privately through
[GitHub security advisories](https://github.com/u-siem/geoip/security/advisories/new),
not in a public issue. Include the version or commit, and if you can, the
database file or a test that reproduces the problem. Problems inside the
`maxminddb` crate itself belong upstream, at
<https://github.com/oschwald/maxminddb-rust>.

You should get an answer within a week. Once a fix is released, the advisory
is published and credited to you unless you prefer otherwise.

## Supported versions

Before 1.0, only the latest release gets fixes.
