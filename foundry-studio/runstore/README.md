# runstore

Where a run's objects live — the spec it was submitted with and the
`.worldview` its frames were written to — and the one interface everything
reads and writes them through.

The interface exists so that nothing above it knows which storage it has. In
deployment that is a Google Cloud Storage bucket; in development it is a folder
on disk, so the whole system runs without a cloud credential. The folder keeps
the promises the recording format depends on: an object appears whole or not at
all, nothing is ever appended to, and a listing is complete the moment a write
returns.

The keys are a layout, not a convention to remember:
`runs/<run>/spec.json` and `runs/<run>/attempts/<n>/recording.worldview`, and
the crate reads a key back into the run and attempt it names.

## Building and testing

```sh
cargo test -p runstore                 # the contract, over the local folder
cargo test -p runstore --features gcs  # the same, over a bucket too, when an emulator is named
```

`tests/backend.rs` is one contract — whole objects appearing atomically, no
clobbering under `create`, lexical prefix-exact listings, exact ranged reads,
removal, the keys that are refused — written once and run over every backend
the build can reach. The folder is always tested. With the `gcs` feature on, a
bucket is tested too when one is named, and each test sweeps what it wrote:

| Set | Bucket tested | Reached with |
|---|---|---|
| `RUNSTORE_GCS_EMULATOR=http://host:port` (and optionally `RUNSTORE_GCS_BUCKET`, default `runstore-test`) | an emulator's | no credential (`RunStore::emulated_bucket`) |
| `RUNSTORE_GCS_BUCKET=<bucket>` alone | a real one | the host's own credentials, as a deployment is (`RunStore::bucket`) |
| neither | none; the suite says so | — |

The emulator is [`tustvold/fake-gcs-server`](https://hub.docker.com/r/tustvold/fake-gcs-server),
the fork `object_store`'s own suite runs against; `fsouza/fake-gcs-server`
itself refuses the XML API `PUT` the client writes with
([#1164](https://github.com/fsouza/fake-gcs-server/pull/1164), open since
2023). `studio/runs/dev.sh gcs-emulator` starts it with the bucket made and
prints the variables. The emulator leaves three parts of the service out, and
the tests of exactly those parts say so on it rather than fail: the `create`
precondition ([#994](https://github.com/fsouza/fake-gcs-server/issues/994))
and the multipart upload `put_file` streams a large file with
([#852](https://github.com/fsouza/fake-gcs-server/issues/852)) are skipped,
and a write's token is not held to name a generation, since the emulator sends
no `x-goog-generation` on a write. All of it runs in full against a real
bucket:

```sh
gcloud auth application-default login
RUNSTORE_GCS_BUCKET=<a bucket you may write to> cargo test -p runstore --features gcs
```

A bucket that is named and cannot be used fails the suite rather than
skipping it.

## Choosing a backend

`CRUCIBLE_STORAGE_DIR` picks the folder, `CRUCIBLE_STORAGE_BUCKET` the bucket;
the Crucible server sets the first from `dev.sh` and takes one of the two.
