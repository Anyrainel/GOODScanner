# Local capture-state patch

Source: https://github.com/konkers/auto-artifactarium at
`4ba25fac64b88970143af6bc2a2ef51338e620d0`. Upstream LICENSE and credits are retained.

The pinned source is kept locally to expose `key_exchange_captured()`, a boolean
derived from the token response's successfully decoded session-seed candidates.
No seeds or keys are exposed to the application. A new game handshake clears
these candidates and the associated timestamp so a previous login cannot mark
the next capture as ready. Decryption and token recognition remain upstream's
implementation.

Tests exercise token recognition before inventory, ordinary dispatch packets,
and a new handshake clearing the previous login state.
