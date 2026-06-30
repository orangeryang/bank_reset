# banked-reset

Tiny Rust CLI for checking Codex banked reset credit expiry.

Installed as `banked-reset`, with `br` as a short alias for the same binary.

## Usage

```sh
banked-reset          # or: br
banked-reset --verbose
banked-reset --json
banked-reset --show-ids
banked-reset --auth ~/.codex/auth.json
```

Default output is compact and sorted by earliest expiry:

```text
banked reset credits: 1 available
next expiry: 2026-07-12 08:00:00 +08:00 (in 21d 0h)

available credits:
#    expires_in  expires_at                 granted_at                 credit_id
1    in 21d 0h   2026-07-12 08:00:00 +08:00 2026-06-12 08:00:00 +08:00 RateLimi...abcd
```

`--verbose` prints the same summary plus the raw reset-credit payload returned by the service.

`--show-ids` reveals credit ids in the compact table. It does not affect `--verbose`, which prints the raw reset-credit payload.

## Auth

The tool reads existing Codex auth from:

```text
$CODEX_HOME/auth.json
~/.codex/auth.json
```

It does not print auth tokens and does not mutate the auth file.

## Endpoint

This uses the private ChatGPT backend endpoint:

```text
GET https://chatgpt.com/backend-api/wham/rate-limit-reset-credits
```

The endpoint is undocumented and may change. The tool does not estimate expiry dates; it only displays `credits[].expires_at` returned by the service.
