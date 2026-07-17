# Additive DIMSE streaming foundation

This document defines the bounded Query/Retrieve path added to
`dicom-toolkit-net` without replacing the public 0.5 networking API.

The target workload is a DICOM gateway that retrieves encoded instances from
external storage and serves desktop viewers through C-GET or C-MOVE. Studies
may contain tens of thousands of instances, and an individual encoded dataset
may approach one gigabyte. Dataset size must therefore not determine process
memory usage.

## Compatibility boundary

The original types, public struct literals, provider traits, handlers, builder
methods, and re-exports remain available. In particular:

- `AssociationConfig` retains its original fields;
- `StoreRequest::dataset_bytes` and `RetrieveItem::dataset` remain `Vec<u8>`;
- `FindServiceProvider`, `GetServiceProvider`, and `MoveServiceProvider` retain
  their original method signatures;
- `handle_find_rq`, `handle_get_rq`, `handle_move_rq`, and `handle_store_rq`
  retain their original signatures;
- `Association::request`, `Association::accept`, and `Association::release`
  retain their legacy defaults;
- a server using only legacy providers retains the legacy association and
  shutdown defaults.

New functionality is selected through parallel APIs. The source-compatibility
fixture in `tests/legacy_api_compat.rs` compiles as an external consumer, and
`cargo-semver-checks` is used against the upstream baseline.

## Public streaming APIs

The additive provider traits are:

- `StreamingFindServiceProvider`;
- `StreamingGetServiceProvider`;
- `StreamingMoveServiceProvider`.

They are registered with `streaming_find_provider`,
`streaming_get_provider`, and `streaming_move_provider`. Registering any
streaming provider enables the bounded `AssociationOptions` defaults. A caller
can also opt in explicitly with `DicomServerBuilder::association_options`,
`Association::request_with_options`, or `Association::accept_with_options`.

The original and streaming paths are separate deliberately. Adding fields to
`AssociationConfig`, `RetrieveItem`, or other types commonly created through
public struct literals would be source-breaking even if default values existed.

## Dataset sources and memory bounds

`DatasetSource` represents an encoded DIMSE dataset without Part 10 File Meta
Information:

- `DatasetSource::bytes` stores immutable in-memory bytes;
- `DatasetSource::file_to_end` streams from an offset through EOF;
- `DatasetSource::file_region` streams an exact file region.

The association opens a file lazily, validates its current length, seeks to the
declared offset, and reads it through a reusable buffer. Normal outgoing
streaming uses at most a 64 KiB dataset buffer per active association, further
reduced when the effective PDU limit is smaller. The P-DATA writer emits its
header and payload separately, avoiding a second payload-sized copy.

```rust
use dicom_toolkit_net::DatasetSource;

let source = DatasetSource::file_region(
    "/var/cache/instances/1.2.3.dcm",
    512,
    900 * 1024 * 1024,
);
```

The offset must point to the encoded dataset. Sending a Part 10 preamble or
File Meta Information in a C-STORE dataset is invalid.

`dicom-toolkit-data` provides the additive bounded inspection APIs
`read_part10_file_layout` and `read_part10_file_index`. The layout reader loads
only a caller-limited File Meta Information block and returns the exact dataset
offset and length. The index reader additionally scans a caller-limited,
decompressed dataset prefix for SOP Class, SOP Instance, Study, and Series
UIDs. It stops after Series Instance UID, validates the File Meta/Dataset SOP
identity, and never reads Pixel Data. File Meta bytes, dataset-prefix bytes, UID
length, and undefined-sequence depth are all caller-configurable through
`Part10ReadLimits`.

## Lazy retrieve contracts

`StreamingRetrieveItem` contains the SOP Class UID, SOP Instance UID, actual
Transfer Syntax UID, and `DatasetSource`. A provider stream yields
`RetrieveSubOperation` values:

- `Ready(item)` for a deliverable instance;
- `Failed { sop_instance_uid, reason }` for an isolated pre-C-STORE failure.

An error from the outer stream is reserved for a fatal provider failure that
prevents further enumeration. This distinction lets independent downloads
continue after one signed URL, validation, cache, or file failure.

The plan declares a `u16` total because DIMSE sub-operation counters are
16-bit. A stream that ends early, yields too many outcomes, or fails fatally is
reported deterministically instead of silently corrupting counters.

### C-GET: late-bound exact contexts

`GetRetrievePlan` intentionally contains no predeclared storage contexts.
C-GET uses the association that is already negotiated with the viewer. When an
item becomes ready, the handler selects an accepted context matching both its
SOP Class UID and its actual Transfer Syntax UID and verifies the negotiated
local SCU role.

This supports providers that know the SOP Class from cloud metadata but learn
the Transfer Syntax only from the downloaded Part 10 File Meta Information. It
does not add a context after negotiation and does not guess or transcode. A
missing exact pair fails only that instance, and subsequent independent items
continue.

Registering a streaming C-GET provider lets the association accept a
requestor-SCP role proposal. The viewer must still propose that role and the
required Storage presentation contexts.

### C-MOVE: contexts declared before the destination association

`MoveRetrievePlan` requires context candidates because the gateway must open a
separate Storage association before consuming the item stream. Each candidate
is an exact SOP-Class/Transfer-Syntax pair.

`build_retrieve_presentation_contexts` validates UIDs, creates the Cartesian
product of SOP Classes and configured Transfer Syntax candidates, deduplicates
it, and enforces the DICOM maximum of 128 presentation contexts. The actual
item still requires an accepted exact pair at delivery time.

Product-specific candidate policy remains outside the toolkit. The toolkit
does not silently add transfer syntaxes or transcode an object.

## C-CANCEL behavior

Streaming C-GET and C-MOVE handlers demultiplex C-CANCEL while they own the
association. Cancellation is accepted only when `Message ID Being Responded
To` matches the active retrieve request. A mismatched cancel is ignored for
that operation.

The handler observes cancellation while:

- waiting for the next lazy provider outcome;
- between C-STORE sub-operations;
- waiting for a C-STORE response;
- writing a large C-GET dataset;
- awaiting a C-MOVE destination C-STORE operation.

On cancellation it stops requesting provider outcomes, drops the stream,
starts no new C-STORE sub-operation, and returns final status `0xFE00` with the
current remaining/completed/failed/warning counters. A response identifier is
absent when there are no failed SOP Instance UIDs.

For C-GET, an already-started C-STORE-RQ is completed in full and its matching
C-STORE-RSP is received and accounted before the final C-GET cancel response.
This avoids marking a truncated dataset as the last fragment solely because
C-CANCEL-GET was received. Consequently, cancel latency can include the
remaining bytes and storage response for the current object; an application
should report a pending cancellation rather than promise an immediate stop.

For C-MOVE, the Storage transfer is on a different association. Cancellation
keeps the active write future alive through the current complete P-DATA-TF PDU
boundary, sends A-ABORT on the Storage association, and then returns the final
C-MOVE cancel response on the original association. It never appends A-ABORT
inside a partially written PDU and never relies on a raw TCP close.

## Association options and negotiation

`AssociationOptions` holds new resource and negotiation controls separately
from `AssociationConfig`:

- maximum incoming PDU variable-field length;
- maximum outgoing P-DATA variable-field length;
- requested and accepted Asynchronous Operations Window;
- requested and accepted SCP/SCU Role Selection.

The legacy PDU decoders retain their permissive treatment of unknown or
malformed extended user-information items. New
`*_with_user_information` functions retain and validate the extended items.

The ready-made server currently accepts only a synchronous asynchronous
operations window `(1, 1)`. It rejects a larger configured window during build
instead of advertising concurrency that its dispatcher does not implement.

## Lifecycle behavior

`Association::release` retains its permissive best-effort behavior.
`Association::release_strict` is additive and validates the requestor/acceptor
release handshake and reports timeouts.

Server shutdown retains detached active associations by default.
`DicomServerBuilder::graceful_shutdown_timeout` opts into tracked draining and
aborts remaining connection tasks after the configured deadline.

## Gateway concurrency model

DICOM message fragments cannot be arbitrarily interleaved on one association.
Cloud download concurrency and DIMSE write concurrency are therefore separate:

1. a bounded download/cache scheduler fetches and validates instances in
   parallel, then atomically publishes complete cache files;
2. a lazy retrieve stream yields a file only when it is ready;
3. each association serializes its DIMSE messages and applies TCP backpressure;
4. `DicomServer::max_associations` bounds concurrent viewer associations.

This supports hundreds of concurrent cloud transfers without representing
them as hundreds of interleaved datasets on one DICOM association. Provider
queues, download permits, cache leases, and disk quotas remain application
responsibilities.

## Validation and remaining limits

The automated suite covers:

- legacy external-consumer source compatibility;
- bounded file regions and PDU pre-allocation limits;
- role and extended-negotiation round trips;
- exact late-bound C-GET context selection with an isolated incompatible item;
- matching and mismatched C-CANCEL;
- provider stream drop on C-GET and C-MOVE cancellation;
- C-GET cancellation during a 32 MiB file stream, including the complete
  C-STORE response boundary;
- C-MOVE cancellation during an active C-STORE on a separate Storage
  association, including A-ABORT at a complete PDU boundary, FE00 counters,
  Failed SOP Instance UID List, and a clean release of the request association;
- bounded Part 10 inspection for explicit, implicit, big-endian, and deflated
  datasets, including a sparse 900 MiB file;
- concurrent loopback associations and configurable shutdown deadlines.

The following remain outside the completed claim:

- interoperability certification against RadiAnt, Horos, OsiriX, and 3D
  Slicer;
- multiple outstanding DIMSE operations on one association;
- asynchronous-window values larger than `(1, 1)`;
- bounded streaming for incoming C-STORE datasets, which are still fully
  materialized and decoded;
- transcoding;
- application-level download scheduling, cache quotas, retry policy, and
  observability.
