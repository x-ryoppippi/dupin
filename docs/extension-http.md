# The `http` capability

`dupin-extension-http` is the host side of the extension `http` capability from the [extension platform design](./design/extension-platform.md#capabilities-and-consent). It is not yet connected to a guest: the broker bridge will call it once a guest's `http.fetch` reaches the host. It never prompts, stores grants or knows which extension is asking. Consent, revocation and delivery of the result stay with the broker.

## What an extension declares

An extension asks for HTTPS origins and the methods it uses against each, in the `dupin` key of its `package.json`:

```json
"capabilities": {
	"http": [
		{ "origin": "https://api.github.com", "methods": ["GET"] },
		{ "origin": "https://example.com:8443", "methods": ["GET", "POST"] }
	]
}
```

`HttpScope::from_manifest` reads that array. An origin is `https://host` with an optional port and nothing else: no path, query, user info or other scheme. Methods are `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE` and `OPTIONS`. Unknown keys are errors. The broker passes the scope the user granted, which may be narrower than the declaration.

## What a fetch may do

The broker calls `HttpService::fetch(request, &scope, &cancel)` on a worker thread. It returns a `Response` or an `HttpError` whose `code()` is stable for the guest's error object.

| Check            | Rule                                                                                                                                                                                                                                                                                                                                                       |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Scope            | The request's origin and method must match one granted entry exactly, before anything is resolved or sent (`out-of-scope`).                                                                                                                                                                                                                                |
| URLs             | Only absolute ASCII `https` URLs. User info, backslashes, spaces, a trailing dot and numeric host shorthands such as `127.1` or `0x7f.0.0.1` are refused, so the host Dupin checks is the host curl connects to (`invalid-request`).                                                                                                                       |
| Redirects        | Up to 5, each checked against the scope with the method it will use (`redirect-out-of-scope`). 303, and 301 or 302 after a POST, become GET without a body. A redirect to another origin drops the guest's `Authorization` and `Cookie`.                                                                                                                   |
| Addresses        | Every hop's name is resolved by Dupin, and every address must be public: loopback, private, carrier-grade NAT, link-local (including cloud metadata), multicast and reserved addresses are refused, as are IPv6 forms that embed them (`blocked-address`). curl then connects only to the checked addresses, so a second DNS answer can't rebind the name. |
| Private networks | `PrivateNetworks::Allow` admits RFC 1918, carrier-grade NAT and IPv6 unique local addresses, for a future user grant to a LAN service. Loopback and link-local stay blocked.                                                                                                                                                                               |
| Ambient state    | No cookies, saved credentials, `.curlrc`, `.netrc`, proxy settings or environment reach the request. The user agent is Dupin's.                                                                                                                                                                                                                            |
| Request headers  | Up to 64, each a valid token with a value of at most 8 KiB and no line breaks. `Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Expect`, `Upgrade`, `TE`, `Trailer`, `Keep-Alive` and `Proxy-*` belong to the transport, and `User-Agent` is Dupin's.                                                                                         |
| Response headers | Names are lowercase. `Set-Cookie`, `Set-Cookie2` and hop-by-hop headers are removed, and trailers are dropped.                                                                                                                                                                                                                                             |
| Cancellation     | Cancelling the `CancelToken` stops the transfer within about 20 ms and returns `cancelled`; a later completion can't be delivered.                                                                                                                                                                                                                         |

| Limit                          | Default |
| ------------------------------ | ------- |
| Total time, redirects included | 30 s    |
| Connection                     | 10 s    |
| Request body                   | 1 MiB   |
| Response headers               | 32 KiB  |
| Response body                  | 8 MiB   |

The broker can lower any limit through `Limits`. The body is read as a stream and the transfer stops as soon as a limit is crossed (`headers-too-large`, `response-too-large`, `timeout`); nothing over the limit is buffered.

A `Response` carries `status`, the final `url`, `redirected`, `headers` and the body bytes. `text()` decodes UTF-8 and `json()` parses JSON within `serde_json`'s nesting limit, both on the host, so the guest receives a value rather than parsing bytes against its own heap. A 4xx or 5xx status is a response, not an error.

## Transport

Dupin already fetches currency rates and updates through the system curl (`/usr/bin/curl`), so the capability uses it too and adds no TLS stack, HTTP client or certificate store to the binary. Each hop is one curl process with `--disable` (no `.curlrc`), an empty environment (no proxy variables), `--proto =https`, no redirect following, `--noproxy '*'`, `--globoff` and `--resolve` pinning the checked addresses. Every option except `--disable` reaches curl as a config file on a pipe, so request headers and the URL never appear in its argument list, where other processes of the same user could read them. curl writes the response heads to a pipe of their own and only the body to stdout, so chunked trailers can't run into the body. Dupin parses the status line and headers itself so it can cap them, and drops trailers. The trust store is the system's.

## Testing

The tests run against plain-HTTP servers on loopback. Loopback and `http` origins are reachable only through a `#[cfg(test)]` constructor that also supplies a fixed resolver; the shipped service has neither.
