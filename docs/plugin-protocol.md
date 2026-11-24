# Experimental query protocol

This is the first executable part of [the plugin proposal](plugin-design.md),
tracked by #6, #7 and #96. It is not a stable SDK. Capability brokerage,
streaming, forms and state persistence remain separate work.

`dupin-plugin-protocol` has only Serde dependencies. [User extensions](./features/extensions.md)
answer in its items, views and actions: the extension host checks a JavaScript
extension's rows against these bounds, and `dupin ext run --json` prints them as a
`results` outcome. The message envelope below was the wire format of the Wasm guests
Dupin no longer runs; the historical calculator fixture in `experiments/plugin-protocol`
exercised it through Wasm memory.

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
`push` with a `view` ID, a non-empty `title` and optional JSON `context`,
which asks the host to query the same guest for that view; or `run` with an
`action` name of up to 64 bytes and optional JSON `data` bounded like `context`,
which asks the host to call the JavaScript guest's `actions[action]` when the
user chooses it. There is no other effect, so actions cannot carry executable
code, native UI objects or arbitrary OS operations; a `run` reaches only the
guest's own handler, and the host, not the guest, marks that call as one the user
chose. The calculator fixture emits no actions.

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
