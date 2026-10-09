# Capture status

The `登录握手 / Login handshake` row reports `has_key_exchange` from the
transport, not the presence of account data or a decoded-command count.

- Genshin: the pinned auto-artifactarium decoder recognized a token response
  and successfully decoded its session-seed candidates. Session-key discovery
  and verification against subsequent encrypted packets can still fail.
- Star Rail: a token response supplied the session seed for the current KCP
  conversation. Before inventory selects a conversation, the decoder reports
  any observed exchange; afterwards it reports only the selected conversation.

Start capture before launching the game to observe this exchange. Before it
arrives, the headline asks the user to launch and log in; inventory rows remain
pending. Afterwards the headline becomes `正在接收游戏数据 / Receiving game data`.
Only actual inventory/achievement observations complete those category rows.
Opening Achievements is prompted only when the requested inventory has arrived.

A completed handshake does not guarantee a complete export. Failures remain
prominent and do not turn an unobserved handshake green. Each new capture resets
the transport and UI state; a new Genshin handshake clears old seed candidates.
The Genshin dependency patch is documented in
`vendor/auto-artifactarium/PATCHES.md`; it exposes a boolean, never key material.
