# Operations: backup machine and failover runbook

This runbook describes what an operator can do with Presenter Orbit as it exists today to cover the loss of a presentation computer. Orbit has no automatic failover, no live state synchronization between computers and no process supervisor. A backup machine is a second, independently running Orbit that the operator keeps in step by hand and cuts to by hand.

Nothing below was rehearsed on two machines. Each step says whether it rests on source inspection, a measurement on one machine, or an assumption to confirm in rehearsal.

## What exists

| Need | What Orbit provides | Source |
| --- | --- | --- |
| Do not lose edits | Workspace mutations are staged, journaled and committed as one transaction in `.presenter-orbit-write` inside the workspace folder. The next open finalizes a committed transaction or rolls an interrupted one back to the previous generation. Stale writers are rejected by revision. | `workspace.rs` (`recover_pending_workspace_write`), recovery tests |
| Restart reopens the show file | The workspace path is saved in `settings.json` in the application data folder (`%APPDATA%\com.presenterorbit.desktop\` on Windows) and reopened at launch. | `main.rs` `get_settings`, `runtime-loaders.ts` |
| Outputs return after a restart | At launch the renderer re-applies the saved activation of every screen (monitor and NDI) after the workspace has loaded. | `runtime-loaders.ts` `restoreStartupOutputs` |
| Start without an operator | Launch-at-login, start-minimized and minimize-to-tray settings exist. | `desktop_lifecycle.rs` |
| Monitoring | `GET /api/v1/status` (HTTP Basic, default port 49321) reports the accepted output state. | [remote-api.md](remote-api.md) |
| A downstream consumer for a backup picture | Each NDI output is a named NDI sender. A second machine's senders appear as separate sources, for example `STUDIO-PC (Orbit Restart 98076 1080p)`: the NDI Runtime prefixes the host name. | `ndi.rs`; source name observed in the restart measurement |
| Move a show to another computer | Copy the workspace folder. | README, workspace format |

## What does not exist

- The live state is not persisted. The current cue, live props, running timers, the active message and media playback are rebuilt from the renderer, so after a restart the outputs are active but blank until a cue is triggered. No code path that restores the live slide was found; confirm this in rehearsal.
- No crash watchdog or automatic relaunch. Orbit does not restart itself after a crash.
- No synchronization, no network link, no watcher on the workspace folder (parity rows OPS-08, OPS-09 and CTL-15 are not implemented). A running Orbit does not notice files copied underneath it.
- No automatic switching between the two machines' pictures. A downstream switcher or NDI-capable consumer must change source.
- `GET /api/v1/status` has no uptime, no start time, no NDI sender health and no frame counters. It can show that the app answers, which workspace it has open, which outputs are active and which cue is live. It cannot show that an NDI sender is still producing frames.
- `settings.json` (audio device, remote port and password, display targets, shortcuts) is per machine. Copying it between computers was not tested; configure the backup by hand.

## Capacity decides what a backup must be

The CPU-composited NDI sender path was measured on one machine (see the evidence). In short: 1080p text-based slides sustain 50 fps and image backgrounds 30 fps in every run; 1080p video backgrounds held 30 fps only when the machine was in its faster state and did not hold 24 fps otherwise; 2160p was not qualified for any scene. A backup machine of lower capability than the primary cannot be assumed to carry the same show. Qualify the backup machine with the same harness before relying on it:

```powershell
$env:PRESENTER_ORBIT_CAPACITY_SECONDS = "12"
cargo test --release -p orbit-host capacity_benchmark_1080p -- --ignored --nocapture --test-threads=1
```

## Setup: primary and backup

1. Install the same Orbit version on both machines. Use the same workspace path on both, for example `D:\Shows\Sunday`. Remote commands that name a target carry the exact `workspacePath` reported by status, so a different path on the backup makes the same command fail there.
2. Copy the workspace to the backup (see Keeping the backup in step).
3. On the backup, choose the copied workspace folder. Configure screens, audio device, NDI names and sizes, and Remote (same password; port may match) by hand and check them against the primary.
4. Give the backup NDI outputs names that identify the machine, and enable only the outputs the backup should publish. The NDI Runtime already prefixes the host name; a name such as `Audience BACKUP` avoids confusion in the consumer's source list.
5. Start both. Open the same presentation on the backup. Confirm in the downstream consumer that both sources are listed.

## Keeping the backup in step

Orbit has no sync. The supported mechanism is a folder copy:

1. Stop editing on the primary. Check that `.presenter-orbit-write` does not exist in the workspace folder; its presence means a write is in flight or was interrupted. Copying during a write is not corrupting: the next open on the backup rolls the interrupted generation back, but you then copy an older state.
2. Close Orbit on the backup, or at least leave its workspace.
3. Mirror the folder: `robocopy D:\Shows\Sunday \\BACKUP\Shows\Sunday /MIR /XD .presenter-orbit-write`. `/MIR` deletes files on the backup that no longer exist on the primary, which is what keeps media identical.
4. Start Orbit on the backup so it opens the copied workspace. Orbit does not watch the folder, so a copy made under a running Orbit is not seen until the workspace is reopened.
5. Compare the primary's and the backup's status: `workspacePath`, `presentation.id`, the number of slides.

Do this after every editing session and before every service. The mirror is a point-in-time copy, not replication; anything edited on the primary after the last copy is missing from the backup.

## Monitoring with `GET /api/v1/status`

Poll each machine about once every 2 seconds, not faster: the server handles one connection at a time and times out reads and writes after 3 seconds. Use HTTP Basic with user `presenter-orbit`.

| Signal | Meaning | Action |
| --- | --- | --- |
| No answer, refused, or a timeout over 3 s | App is down, hung, or the machine or network is lost. | Start the switching procedure |
| `401` | Password differs from the monitor's | Fix the monitor; not a show fault |
| `workspacePath` differs from the expected path | The wrong workspace is open | Reopen the intended workspace |
| `outputs[].active` is false for an output that should be on | The screen is disabled or failed to restore | Enable it; check the output error in Settings |
| `currentSlide.id` differs between primary and backup | The backup is not following the show | Re-trigger the cue on the backup |
| `controlsError` present | The workspace catalog could not be read | Check the workspace folder |

A monitor that polls only proves the process and its HTTP service are alive. It does not prove that pictures reach the consumer. Watch the NDI source on a multiviewer or the program monitor as well.

A minimal monitor, not run against a live Orbit (the request shape follows `remote-api.md`):

```powershell
$headers = @{ Authorization = "Basic " + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("presenter-orbit:<password>")) }
foreach ($machine in "PRIMARY", "BACKUP") {
  try {
    $status = Invoke-RestMethod "http://${machine}:49321/api/v1/status" -Headers $headers -TimeoutSec 3
    "{0} ok workspace={1} slide={2}" -f $machine, $status.workspacePath, $status.currentSlide.id
  } catch { "{0} DOWN {1}" -f $machine, $_.Exception.Message }
}
```

## Switching to the backup

Prepare: the backup is running, has the current workspace, and its outputs are enabled and visible in the consumer.

1. Confirm the primary is down or unfit (status, program monitor).
2. On the backup, open the presentation and trigger the current cue. Use the browser panel on the backup's Remote port or the local operator window. If the cue was triggered through Remote commands that name a target, the backup accepts them only when its `workspacePath` equals the primary's.
3. Restore props, timers, the message and media playback by hand; none of these were carried over.
4. In the downstream switcher, select the backup's NDI source.
5. Keep operating from the backup until the end of the service. Do not copy the workspace back over the primary during the service.

Failing back: after the service, treat the backup's workspace as the newer copy if anything was edited on it; mirror it to the primary with the same `robocopy` in the other direction, then restart the primary.

## Recovery after a crash on the primary

1. Relaunch Orbit. It reopens the saved workspace, finishes or rolls back an interrupted write, and re-applies the saved output activation.
2. When the outputs are back, trigger the current cue and restore props, timers and playback.
3. Check status and the consumer.

Measured on one machine, headless, with the real NDI Runtime (`capacity_ndi_restart_latency`, one process, cold runtime load): the NDI part of a restart (loading the runtime, creating the sender, sending the first scene, being discovered and delivering the first frame to a receiver) took 247 ms at 1080p and 411 ms at 2160p. That is a floor, not the recovery time. Process start, WebView startup, settings and workspace load and the screen restore were not measured: doing so needs the real application, which stores its settings in the user's application data folder shared with the installed copy, and was not run for that reason. Measure it in rehearsal with a stopwatch from the kill to the first frame in the consumer, and record it:

| Run | Kill to window | Kill to workspace loaded | Kill to first NDI frame in the consumer |
| --- | --- | --- | --- |
| 1 |  |  |  |

Until that is filled in, the recovery time of the real application is unknown.

## Rehearsal checklist

- Kill the primary mid-cue; time the restart; confirm outputs come back blank and which settings had to be redone.
- Kill the primary while saving an edit; confirm the next open shows either the old or the new generation, never a mix.
- Switch the consumer to the backup source and back.
- Run the capacity harness on the backup machine and compare with the primary.
- Confirm that a command carrying the primary's `workspacePath` works on the backup.
