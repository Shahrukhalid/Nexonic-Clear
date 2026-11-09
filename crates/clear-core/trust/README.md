# Bundled trust lists

Retrieved 9 November 2026. Files are unmodified copies; see
`THIRD_PARTY_NOTICES.md` for sources and licences.

| File | Certificates | SHA-256 |
| --- | --- | --- |
| `C2PA-TRUST-LIST.pem` | 33 | `148c1c167e98dc909a0d5793d2feee1022aa60fef5c83a0d6b08f67f4446bc37` |
| `C2PA-TSA-TRUST-LIST.pem` | 24 | `eca8347ee71e31d226e00c85b15aca356116fefe6c26ed8c0ee1f954115b1990` |
| `interim-anchors.pem` | 27 | `db57e50b9bd4e4c786cbb91f38fd85e2af6a9d5412438a8cddcc42688df07ae5` |
| `interim-allowed.pem` | 115 | `dfe328032e56f98b83f27f48a3642314124568b9f71ff471dffabf9b307c65d5` |
| `store.cfg` | EKU list | `1044c30a906b0c9cbe28b4a4c0002ca56a718f0ae22b9af83238543a07d0e718` |

To update, download the same paths from the source repositories, replace the
files, update this table and run `cargo test -p clear-core`.
