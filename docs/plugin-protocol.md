# Experimental query protocol

This is the first executable part of [the plugin proposal](plugin-design.md),
tracked by #6, #7 and #96. It is not a stable SDK. The launcher runs it for
[user extensions](extensions.md), which have no host imports. Capability
brokerage, streaming, forms and state persistence remain separate work.

`dupin-plugin-protocol` has only Serde dependencies and is shared by hosts and
guests. The calculator fixture in `experiments/plugin-protocol` exercises it
through actual Wasm memory rather than passing native Rust objects across an ABI.

## Messages

Messages are UTF-8 JSON with `version: 1`. Unknown versions, fields, view kinds
and error codes are rejected. Request IDs and generations are opaque strings,
so authors do not need JSON numbers to represent a host's full-width counters.

A request contains `request_id`, `generation`, `command`, `query`, `limit` and
optional versioned opaque `state`. A request for a pushed view also has `view`,
the view ID, and the `context` its push carried; `context` without `view` is
rejected. Both are omitted outside a view, so those requests are unchanged from
the first version of this protocol and guests built against it keep working. A
response echoes both correlation tags and contains an `outcome` tagged by `kind`:

- `results`: keyed items and optional state;
- `detail`: a title, a body and action descriptors, the whole of a pushed view.
  Only a request with `view` may be answered this way;
- `error`: a defined error code and display message.

Items have a stable ID, a view and action descriptors. View kinds are `text`
(title and subtitle), `detail` (title and body), and `pair` (source and result).
A text subtitle may be empty. An action descriptor has an ID, a title and an
`effect` tagged by `kind`: `copy` with non-empty `text`; `open_url` with an
`http` or `https` `url` that has a host and no whitespace or control characters;
or `push` with a `view` ID, a non-empty `title` and optional JSON `context`,
which asks the host to query the same guest for that view. There is no other
effect, so actions cannot carry executable code, native UI objects or arbitrary
OS operations. The calculator fixture emits no actions.

The host checks request/response correlation and result count after decoding.
Item IDs must be unique within a response; action IDs must be unique within an
item. Plugin identity is assigned outside the wire message, never supplied by
the guest. Correlation checks do not replace the shell's cancellation and
current-generation checks.

## Bounds

| Value                        |                Limit |
| ---------------------------- | -------------------: |
| Encoded request              |               16 KiB |
| Encoded response             |              256 KiB |
| Query and pair source        |           4 KiB each |
| Requested and returned items |                  500 |
| IDs and command              | 128 UTF-8 bytes each |
| Title and action label       | 512 UTF-8 bytes each |
| Subtitle and error message   |           2 KiB each |
| Detail body                  |               64 KiB |
| Pair result                  |               16 KiB |
| Actions per item             |                    8 |
| Copy text                    |               16 KiB |
| URL                          |    2,048 UTF-8 bytes |
| Encoded opaque state data    |      8 KiB, depth 64 |
| Encoded view context         |      4 KiB, depth 16 |
| View ID                      |      128 UTF-8 bytes |

State schema versions must be positive; the feature owns their meaning. Depth
counts nested arrays and objects, with at most 64 containers on a path. State
holds parsed JSON values: duplicate object keys inside `state.data` keep their
last value. Typed protocol fields reject duplicates. Strings are measured in
UTF-8 bytes, and IDs cannot contain control characters. A zero
result limit is valid. The calculator returns an empty result before evaluation
in that case. Data is rejected rather than silently truncated.

Input byte caps are checked before JSON decoding. Encoding uses a capped
writer, including when escaping expands a string. Errors report categories and
static field names without echoing message contents or JSON parser error text.

## Private Wasm transport

The fixture uses a separate ABI version (`1`) and requires these exports:

- `dupin_abi_version() -> i32`;
- `dupin_alloc(length: i32) -> i32`;
- `dupin_query(input: i32, input_length: i32, output: i32, output_capacity: i32) -> i32`;
- one exported linear memory named `memory`.

The host allocates and validates disjoint input/output regions, copies in the
encoded request, and supplies a 256 KiB output buffer. The returned length must
be positive and fit the buffer before any output is read. Negative statuses
mean invalid buffers/request or insufficient output capacity. Returned bytes
must then pass protocol decoding, semantic limits and correlation checks.

The module may be reused, but each request has a fresh store/instance. Buffers
live for that request; dropping the store releases them on both success and
failure. There is no guest `free` callback or retained pointer. The guest's
unsafe Rust exports require buffers allocated through its allocator; a hostile
host is outside this fixture's threat model. Host access to guest memory uses
checked Wasmi APIs.

The host rejects imports and modules larger than 4 MiB. It caps linear memory at
16 MiB, tables at one with 1,024 elements, memories at one and instances at one.
Initialisation/version/allocation share 1M fuel; the query receives a separate
20M budget. Both are experimental budgets. Wasmi's `auto-dispatch` feature keeps
portable dispatch available when native tail calls cannot be guaranteed, notably
in debug builds; increasing the native stack is not a substitute for it.

The shared `dupin-plugin-runtime` host can poll a host cancellation predicate and absolute
deadline between resumable fuel slices. The CLI uses a five-second fixture
deadline. These checks do not pre-empt module loading, instantiation/start,
host parsing/translation, allocations or individual charged operations; the
start function remains bounded by startup fuel. See the fixture README
for the execution policy and its limits.

Fuel bounds guest execution, not eager host parsing/translation, rendering or
total host memory. `MAX_MODULE_BYTES` caps the input size for parsing/translation;
neither fuel nor query deadlines bound their work. This prototype has no
permission-bearing imports, host operation completions or external-plugin
installation. Production cancellation/deadline integration is still required;
this is not a claim that the production isolation and permission design is complete.
