# Which automations do we run?

This file is meant to provide an overview and explainer on what the up-rust workflow automation strategy is, and what the different workflow elements do.

**A general note:** All workflows will use the `stable` version of the Rust toolchain, unless the GitHub actions variable `RUST_TOOLCHAIN` is set to pin a specific Rust version (e.g. `RUST_TOOLCHAIN=1.76.0`).

At this time, there are three events that will initiate a workflow run:

## PRs and merges to main

We want a comprehensive but also quick check & test workflow. This should be testing all relevant/obvious feature sets, run on all major OSes, and of course include all the Rust goodness around cargo check, fmt, clippy and so on.

This is implemented in [`check.yaml`](check.yaml) and [`check-dependencies.yaml`](check-dependencies.yaml)

## Release publication

We want exhaustive tests and all possible checks, as well as creation of license reports, collection of quality artifacts and publication to crates.io. This workflow pulls in other pieces like the build workflow. An actual release is triggered by pushing a tag that begins with `v`. The GitHub Release for that tag must exist before the workflow uploads its release artifacts. After the checks and artifact uploads succeed, the package is verified with a dry run and published automatically using crates.io Trusted Publishing.

This is implemented in [`release.yaml`](release.yaml)

### Trusted Publishing setup

Configure a GitHub Trusted Publisher for `up-transport-mqtt5` in its
[crates.io settings](https://crates.io/crates/up-transport-mqtt5/settings):

| Field             | Value                     |
| ----------------- | ------------------------- |
| Repository owner  | `eclipse-uprotocol`       |
| Repository name   | `up-transport-mqtt5-rust` |
| Workflow filename | `release.yaml`            |
| Environment       | Leave blank               |

Only the publishing and authentication-test jobs can request an OIDC token.
The official crates.io authentication action exchanges the GitHub identity for
a short-lived publication token and revokes it at the end of the job. No
long-lived crates.io secret or second-person deployment approval is required.

After the workflow is merged, manually dispatch **Release** from `main` to test
authentication. Manual dispatch runs only the authentication-test job: it does
not run tests, upload release artifacts, or publish a crate. Dispatching from
another branch skips all jobs.

After the first successful OIDC publication, enable trusted-publishing-only for
the crate. The organization-level `CRATES_IO_TOKEN` is still used by other
repositories and should not be removed as part of this repository's migration.

## Nightly, out of everyone's way

All the tests we can think of, however long they might take. For instance, we can build up-rust for different architectures - this might not really create many insights, but doesn't hurt to try either, and fits nicely into a nightly build schedule.

This is implemented in [`nightly.yaml`](nightly.yaml)

## uProtocol specification compatibility

The uProtocol specification is evolving over time. In order to discover any discrepancies between the currently implemented version and the changes being introduced to the specification, we perform a nightly check that verifies if the current up-rust code base on the main branch can be compiled and test can be run successfully using the most recent revision of the uProtocol specification.

This is implemented in [`latest-up-spec-compatibility.yaml`](latest-up-spec-compatibility.yaml)

## Further workflow modules

In addition to the main workflows described above, there exist a number of modules that are used by these main workflows. These live in the [uProtocol CI/CD repository](https://github.com/eclipse-uprotocol/ci-cd)
