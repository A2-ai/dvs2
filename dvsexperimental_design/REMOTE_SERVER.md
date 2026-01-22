# Remote server (HTTP CAS)

The remote server (`dvs-server`) is a simple content-addressable storage (CAS)
service used by `dvs push`/`dvs pull` and the planned `dvs sync`.

## Protocol overview

All object operations are performed by OID via HTTP endpoints:

- `HEAD /objects/{algo}/{hash}`
  - Checks existence.
  - Returns 200 with Content-Length if present, 404 if missing.
  - Requires Read permission when auth is enabled.

- `GET /objects/{algo}/{hash}`
  - Downloads object bytes.
  - Returns 200 with `application/octet-stream`.
  - Requires Read permission when auth is enabled.

- `PUT /objects/{algo}/{hash}`
  - Uploads object bytes.
  - Returns 201 if created, 200 if already present.
  - Requires Write permission when auth is enabled.
  - Server verifies that the payload hash matches the `{algo}/{hash}` path.

- `DELETE /objects/{algo}/{hash}`
  - Deletes an object.
  - Returns 204 if deleted, 404 if missing.
  - Requires Delete permission.

Additional endpoints:

- `GET /health` returns `{ "status": "ok" }`.
- `GET /status` returns JSON with version, storage usage, object count, uptime.

## Storage layout

The server stores objects in the same layout as dvs-core:

```
{storage_root}/{algo}/{prefix}/{suffix}
```

This matches the local cache and external storage layout, enabling direct
interoperability.

## Authentication and authorization

Authentication is API key based via the Authorization header:

```
Authorization: Bearer <api_key>
```

Server config defines keys and permissions:

- Read: HEAD/GET
- Write: PUT
- Delete: DELETE
- Admin: all permissions

If auth is disabled, the server accepts requests without a token.

## Upload limits

`max_upload_size` enforces a hard upper bound. The server rejects larger
payloads with HTTP 413 (Payload Too Large).

## Known limitations

- No resumable uploads.
- No range requests.
- No server-side compression.
- No garbage collection of unreferenced objects.

These are candidates for future work once the sync-first CLI is stable.
