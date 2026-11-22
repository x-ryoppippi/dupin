# Experimental query protocol

This is the first executable part of [the plugin proposal](plugin-design.md),
tracked by #6, #7 and #96. It is not a stable SDK or an external-plugin loader.
The launcher does not yet use it. Registration, capability brokerage, action
invocation, streaming, forms and production scheduling remain separate work.

`dupin-plugin-protocol` has only Serde dependencies and is shared by hosts and
guests. The calculator fixture in `experiments/plugin-protocol` exercises it
through actual Wasm memory rather than passing native Rust objects across an ABI.

## Messages

Messages are UTF-8 JSON with `version: 1`. Unknown versions, fields, view kinds
and error codes are rejected. Request IDs and generations are opaque strings,
so authors do not need JSON numbers to represent a host's full-width counters.

A request contains `request_id`, `generation`, `command`, `query`, `limit` and
optional versioned opaque `state`. A response echoes both correlation tags and
contains an `outcome` tagged by `kind`:

- `results`: keyed items and optional state;
- `error`: a defined error code and display message.

Items have a stable ID, a view and action descriptors. View kinds are `text`
(title and subtitle), `detail` (title and body), and `pair` (source and result).
A text subtitle may be empty. Action descriptors contain only an ID and title;
they cannot carry executable code, native UI objects or arbitrary OS operations.
This first query fixture emits no actions because action invocation is not yet
implemented.

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
| Encoded opaque state data    |      8 KiB, depth 64 |

State schema versions must be positive; the feature owns their meaning. Strings
are measured in UTF-8 bytes, and IDs cannot contain control characters. A zero
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

Fuel bounds guest execution, not total parsing, translation, rendering or host
memory. This prototype has no wall-clock deadline, permission-bearing imports,
host operation completions or external-plugin installation. It is not a claim
that the production isolation and permission design is complete.
