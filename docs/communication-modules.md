# Communication modules

## Current product scope

The operator retired built-in Sony BVS and GVG100 switcher support on 2026-10-21.
Their editors, encoders, bridge types and native senders have been removed.
Previously saved actions naming these protocols fail explicitly; they are not
reinterpreted as raw text or another protocol. Historical parity evidence records
the earlier implementation, not current availability. These retired switchers
are no longer completion requirements.

The built-in focus is TCP/UDP communication: raw UTF-8 and byte payloads,
RossTalk output/input, acknowledged GlobalCache relays and Sony BVW/VDCP/AMP media
receivers. Ordinary serial transport is retained at the operator's request;
it is independent of the retired switcher profiles.
Native MIDI Note On/Off output is also available through explicit OS destination
selection, a shared note action editor, ordered sending and held-note release.
This does not implement its own network MIDI session or virtual MIDI driver.
Grass Valley AMP is received over TCP using Sony BVW transport and AMP clip commands:
newline-terminated `CRAT`/`CMDS`/`STOP` messages with a four-digit decimal length and
9-pin commands as hex without a checksum. Replies are the 9-pin reply in hex, `1001`
for an acknowledgement and `1111` for a rejection. The framing follows two public
AMP clients and the SimplyLive AMP manual; the Grass Valley specification itself and
real AMP controllers were not available. A named channel binds to the endpoint's
selected media bin; channel-less sessions only expose catalog operations.
AMP now serves an explicitly configured media bin, with eight-character names or
full printable ASCII names up to 32 bytes. It implements fixed/extended first/next
ID listings, single-clip In Preset and Cue Up With Data, optional cue timecode,
and reading back the selected AMP preset's initial position. Clip loading uses
the native video-deck runtime; it pauses at the requested position until Play.
Invalid catalogs fail before connection, and unavailable clips are rejected.
The configured bin is exposed as one virtual folder, with enumeration, exact
selection and current-folder queries. Loaded-ID/source-start queries and
transport status ranges are connected to native state. STOP acknowledges and
closes its TCP session, discarding queued commands after it; reconnecting starts
fresh protocol state. Commands require a created session.
No legacy switcher work is planned.

## Implemented module boundary

The communication payload encoders live in `crates/orbit-app/src/settings/` and the
host's communication modules. Each protocol owns its stable ID, label key,
validation-error key and payload encoder. The editor builds its profile list from
those entries, and the host boundary uses the same entries for validation and
encoding. Protocol-specific authoring fields remain separate views.

The native host owns transport access, endpoint validation and acknowledgement
handling. An unsupported structured payload is rejected before opening a socket.
The module registry is compiled into the application. It does not execute
downloaded scripts, load third-party libraries or provide a plugin installer.

## Future plugin host design

A future plugin host can expose the same protocol module contract through a
versioned manifest with stable IDs, direction, supported transports and declared
capabilities. The host should retain endpoint validation, connection lifecycle,
workspace/session guards and transport ownership. Plugin editors and protocol
code should not receive unrestricted process or filesystem access.

Installation, compatibility negotiation, permission grants, isolation, updates
and plugin signing are separate future work. They are not implemented by this
registry refactor. The existing application module registry remains responsible
for app composition; protocol modules do not replace that boundary.

## Verification (2026-10-21)

- `npm run check` passed, including renderer/test typechecks, production build
  and native Cargo check.
- `npm run maintenance:check` passed, including import, architecture,
  dependency, reachability and unused-export checks.
- All 1827 renderer tests in 224 files passed with existing coverage gates.
  The final targeted communication suite passed 55 tests.
- Eleven native tests passed using the production communication, input-config,
  serial and GlobalCache modules. These include actual TCP/UDP exchanges and
  rejection of retired structured payloads before connection.
- Renderer build budget passed at 1324.2 KiB. No external plugin loading or
  physical-device acceptance is claimed.
