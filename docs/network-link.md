# Network Link

Network Link connects Presenter Orbit computers into a group. Any member can trigger content or show controls. Receivers use their own content at matching stored positions; files, media assets and themes are not copied. This is an Orbit-to-Orbit protocol. It does not communicate with ProPresenter's private Network Link protocol.

## Connect computers

1. On every computer, open **Settings → Remote**, enable Remote Control and set a nonempty password.
2. Use **Connect computers → Find computers** to list computers on the local IPv4 network. Select a computer, or enter its host and port manually.
3. Enter that computer's remote password and its group name. If neither computer has a group, choose a new name. Select **Connect to group**. Both computers save an outgoing connection to the other, using their own remote passwords for the return connection. Discovery does not expose passwords or grant access.
4. Connect additional computers to a member of the same group. Accepted commands propagate through the connected group while retaining the original sender and sequence. Replay protection prevents forwarding loops and repeated execution. Every member can take control; there is no fixed master.
5. Refresh delivery status to inspect confirmation or an error. The individual connection list supports deliberate one-way/manual connections as well as removing outgoing connections. Disable Network Link to stop receiving and sending. To switch groups, disable the link and remove the old connections first.

Discovery uses a bounded request/reply exchange on IPv4 multicast `239.255.79.66:49322`, limited to one network hop. Only computers with a running password-protected remote server advertise. A discovery reply is an untrusted address suggestion; connecting still requires authentication. A firewall, routed network or occupied discovery port can prevent automatic discovery; the manual host/port connection remains available. One advertising Orbit process per computer can use this discovery port.

## Corresponding content

Positions are zero-based stored workspace slots internally, not local IDs or browser search positions. Keep corresponding order aligned on every computer:

- **Libraries:** the raw `libraries` array, each library's `itemIds`, then the selected arrangement's slide order. Title sorting and filtering do not renumber these slots. The explicitly selected library determines membership when a presentation belongs to several libraries.
- **Playlists:** raw playlist and entry positions, including folder/header slots, followed by the receiving occurrence's arrangement slides. Folder/header targets and disabled slides fail explicitly. Repeated presentations and arrangement sections retain their precise occurrences.
- **Media and audio:** remote-catalog bin order (unfiled first, then stored media libraries), then that bin's ordered media. Audio playlists use `itemFiles`; graphics use `graphicsOrder` where defined. Trigger, play, pause and stop use the corresponding receiver asset.
- **Timers:** raw timer position. Manual run controls and timer type, duration, overrun, target/elapsed values and format edits are relayed. Existing timers must be present on each receiver; local IDs/names are preserved.
- **Messages:** raw message position. Manual show/clear, text edits and text-token values are relayed. Token declarations must correspond in stored position and type. Text placeholders and values resolve to the receiving token names; local token IDs, timer references and message themes are preserved.
- **Props and macros:** raw collection position. The receiver runs its own corresponding prop/macro. Macro child actions execute locally without publishing a second set of network commands.
- **Stage layouts:** raw layout position and the corresponding stage-output slot. Stage-output slots use sorted stage output keys on each computer; the receiver resolves its own key.

Missing targets and mismatching token structures are errors, rather than a search for a similarly named item. The sender and receiver may have different workspace paths, local IDs and presentation content. Manual clicks, controls and edits are linked. Automatic Go To Next advancement and timeline chase are not broadcast.

## Delivery

The native host sends JSON bounded to 48 KiB to the authenticated `POST /api/v1/link/cue` endpoint. Group negotiation uses authenticated `POST /api/v1/link/join`; the callback address comes from the TCP connection rather than a supplied host. The group string is a namespace, not an authentication secret. Settings live in separate app-data `network-link.json`; no workspace schema changes are involved.

All members must use protocol `orbit-link/3`. Packets contain a group, original sender-session UUID, positive monotonic sequence and an explicitly tagged library, playlist, media or show-control command. Older versions and malformed/mixed fields are rejected. Native controls commit through the existing output coordinator. Renderer-owned controls wait for the existing command-result acknowledgement; accepting an HTTP request alone does not count as execution.

Each process supports 32 outgoing peers, 64 pending dispatcher commands, 64 pending commands per peer and 64 sender sessions. Each peer has an independent ordered worker, so a slow peer does not block healthy peers. Cue/media requests have a two-second HTTP deadline; renderer-owned show controls allow execution acknowledgement up to 35 seconds. A macro is confirmed once the renderer has actually started its run (including scheduled delayed actions); eventual macro completion/failures remain available through the existing remote command results. Timer configuration acknowledgement also waits for the native output snapshot to adopt the saved definition before subsequent timer controls. A five-second queue-age limit drops stale pending commands. Failed delivery discards that peer's already-pending backlog; a later explicit action can resume delivery. There is no automatic catch-up replay. Configuration/workspace changes retire pending generations. Late results cannot overwrite the new configuration's status or discard its commands. The remote server bounds concurrent request workers so one delayed macro cannot block all other remote requests.

The implementation follows the public [Renewed Vision Network Link workflow](https://support.renewedvision.com/hc/en-us/articles/4412193892627-Network-Link) for peer groups and indexed content/show controls. Physical network and platform certification remain deployment checks.
