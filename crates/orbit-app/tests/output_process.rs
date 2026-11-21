//! The output process as a real child of a real stand-in controller: the controller is killed the way a crash or
//! Task Manager kills it, and the output process must stay on air, adoptable, and `Busy` to a second client.
//!
//! The binary is a bin-only crate, so the link, protocol, and framing sources are compiled into this test.
#![allow(dead_code)]

#[path = "../src/native_outputs/framing.rs"]
pub mod framing;
#[path = "../src/locked.rs"]
mod locked;
mod native_outputs {
    pub use super::framing;
}
#[path = "../src/output_process/client.rs"]
mod client;
#[path = "../src/output_process/protocol.rs"]
mod protocol;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use client::{Config, EventSink, OutputLink, ProcessSpawner, Spawned, Spawner};
use protocol::{read_endpoint, Endpoint};
use serde_json::{json, Value};

const BINARY: &str = env!("CARGO_BIN_EXE_orbit-app");
const ROLE: &str = "ORBIT_OUTPUT_TEST_ROLE";
const GRACE: &str = "60";

fn scene(session: &str, text: &str) -> Value {
    json!({
        "version": 2, "kind": "full", "sessionId": session, "clientSequence": 1,
        "snapshot": {
            "liveItem": null,
            "liveSlide": { "id": "slide", "content": text, "layers": [] },
            "liveSlideId": null, "liveBuildStep": 0, "livePropIds": [], "livePlaylistId": null,
            "library": [], "libraries": [], "playlist": [],
            "screenConfig": { "screen": { "type": "audience", "label": "Screen" } },
            "themes": [], "props": [], "stageLayouts": [], "stageScreenAssignments": [], "masks": [],
            "backgroundId": null, "backgrounds": [], "media": [], "logoImagePath": "", "logoImageUrl": "",
            "backgroundScale": "fit", "backgroundPlayback": "stop", "backgroundEndBehavior": "hold",
            "backgroundPlaybackState": null, "backgroundSelectionId": null, "backgroundTransition": "none",
            "audioId": null, "selectedAudio": null, "activeMessageText": "",
            "stageMessage": { "text": "", "state": "cleared" },
            "countdownTimers": [], "countdownTimerRuns": {}, "audioSettings": {},
            "transportPlaybackPaused": {}
        }
    })
}

fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let listing = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&listing.stdout).contains(&format!("\"{pid}\""))
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

fn kill_process(pid: u32, tree: bool) {
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill");
        command.args(["/F", "/PID", &pid.to_string()]);
        if tree {
            command.arg("/T");
        }
        let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(windows))]
    {
        let _ = tree;
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

fn wait_until(what: &str, seconds: u64, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn config(directory: &Path, spawner: Arc<dyn Spawner>, events: EventSink) -> Config {
    Config {
        directory: directory.to_path_buf(),
        rendering_bit_depth: "normal".into(),
        spawner,
        events,
    }
}

fn silent() -> EventSink {
    Arc::new(|_, _| {})
}

/// The controller that is killed. It starts the output process itself, through the real link and spawner, puts a
/// scene on air, and then waits to be killed.
#[test]
#[ignore = "Child controller entry point, launched by the crash-recovery scenarios"]
fn stand_in_controller() {
    let role = std::env::var(ROLE).unwrap_or_default();
    if role != "controller" && role != "foreign" {
        return;
    }
    let directory = PathBuf::from(std::env::var("ORBIT_OUTPUT_TEST_DIR").unwrap());
    let ready = PathBuf::from(std::env::var("ORBIT_OUTPUT_TEST_READY").unwrap());
    if role == "foreign" {
        let events: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let recorded = events.clone();
        let link = OutputLink::start(config(
            &directory,
            Arc::new(NeverSpawn),
            Arc::new(move |event, payload| {
                recorded.lock().unwrap().push((event.to_string(), payload));
            }),
        ));
        wait_until("a different controller to be refused", 20, || {
            events.lock().unwrap().iter().any(|(event, payload)| {
                event == "output:link-changed" && payload["foreign"] == true
            })
        });
        assert!(!link.attached());
        std::fs::write(ready, serde_json::to_vec(&*events.lock().unwrap()).unwrap()).unwrap();
        link.shutdown();
        return;
    }
    let spawner = Arc::new(ProcessSpawner {
        executable: BINARY.into(),
        extra_arguments: vec!["--inert".into(), "--grace-seconds".into(), GRACE.into()],
        // A spawn that had to fall back from job breakaway is reported through this sink.
        events: {
            let note = directory.parent().unwrap().join("spawn-events.txt");
            Arc::new(move |_, payload| {
                let _ = std::fs::write(&note, payload.to_string());
            })
        },
    });
    let link = OutputLink::start(config(&directory, spawner, silent()));
    wait_until("the controller to attach", 30, || link.attached());
    let accepted = link
        .invoke(
            "updateOutputScene",
            vec![scene("controller", "on air before the crash")],
        )
        .unwrap();
    assert_eq!(accepted["ok"], true);
    let endpoint = read_endpoint(&directory).unwrap().unwrap();
    std::fs::write(&ready, format!("{} {}", std::process::id(), endpoint.pid)).unwrap();
    // A stand-in that nobody kills ends itself, so it cannot linger after a failed test.
    std::thread::sleep(Duration::from_secs(180));
}

/// Kills the output process on every exit path of a test, including a failed assertion.
struct Cleanup {
    controller: std::process::Child,
    pids: Vec<u32>,
    directory: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.controller.kill();
        let _ = self.controller.wait();
        for pid in &self.pids {
            kill_process(*pid, false);
        }
        if let Ok(Some(endpoint)) = read_endpoint(&self.directory) {
            kill_process(endpoint.pid, false);
        }
        let _ = std::fs::remove_dir_all(self.directory.parent().unwrap());
    }
}

struct Started {
    controller: u32,
    output: u32,
    directory: PathBuf,
    cleanup: Cleanup,
}

fn start_controller(name: &str) -> Started {
    let root = std::env::temp_dir().join(format!("orbit-output-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let directory = root.join("output-process");
    let ready = root.join("ready.txt");
    let controller = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "stand_in_controller",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, "controller")
        .env("ORBIT_OUTPUT_TEST_DIR", &directory)
        .env("ORBIT_OUTPUT_TEST_READY", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the stand-in controller starts");
    let controller_id = controller.id();
    let mut cleanup = Cleanup {
        controller,
        pids: vec![controller_id],
        directory: directory.clone(),
    };
    wait_until("the stand-in controller to put a scene on air", 60, || {
        ready.exists()
    });
    let pids = std::fs::read_to_string(&ready).unwrap();
    let mut pids = pids.split(' ').map(|pid| pid.parse::<u32>().unwrap());
    let (controller_pid, output) = (pids.next().unwrap(), pids.next().unwrap());
    cleanup.pids.push(output);
    assert_eq!(controller_pid, controller_id);
    assert!(
        process_alive(output),
        "the output process runs while its controller lives"
    );
    println!(
        "SPAWN FALLBACK NOTE: {:?}",
        std::fs::read_to_string(directory.parent().unwrap().join("spawn-events.txt")).ok()
    );
    Started {
        controller: controller_pid,
        output,
        directory,
        cleanup,
    }
}

struct NeverSpawn;

impl Spawner for NeverSpawn {
    fn spawn(&self, _: &Path) -> Result<Box<dyn Spawned>, String> {
        Err("A second output process must not be started.".into())
    }
}

fn adopt_and_release(started: &Started) {
    let events: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let sink = events.clone();
    let link = OutputLink::start(config(
        &started.directory,
        Arc::new(NeverSpawn),
        Arc::new(move |event, payload| sink.lock().unwrap().push((event.to_string(), payload))),
    ));
    wait_until(
        "a new controller to attach to the surviving output process",
        30,
        || link.attached(),
    );
    let attached = events
        .lock()
        .unwrap()
        .iter()
        .find(|(event, _)| event == "output:link-changed")
        .map(|(_, payload)| payload.clone())
        .unwrap();
    assert_eq!(
        attached["hasScene"], true,
        "the output process still holds the scene"
    );
    let answer = link
        .invoke(
            "updateOutputScene",
            vec![scene("restarted", "an empty new controller")],
        )
        .unwrap();
    assert_eq!(answer["cacheMiss"], true);
    assert_eq!(
        answer["snapshot"]["liveSlide"]["content"], "on air before the crash",
        "the new controller is handed what is on air"
    );

    let ready = started
        .directory
        .parent()
        .unwrap()
        .join("foreign-controller.json");
    let errors = started
        .directory
        .parent()
        .unwrap()
        .join("foreign-controller.log");
    let mut second = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "stand_in_controller",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, "foreign")
        .env("ORBIT_OUTPUT_TEST_DIR", &started.directory)
        .env("ORBIT_OUTPUT_TEST_READY", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::from(std::fs::File::create(&errors).unwrap()))
        .spawn()
        .unwrap();
    assert!(
        second.wait().unwrap().success(),
        "{:?}",
        std::fs::read_to_string(errors)
    );
    let second_events: Vec<(String, Value)> =
        serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
    assert!(second_events
        .iter()
        .any(|(event, payload)| event == "output:link-changed" && payload["foreign"] == true));
    assert_eq!(
        read_endpoint(&started.directory).unwrap().unwrap().pid,
        started.output
    );

    link.shutdown();
    wait_until("the output process to end on Shutdown", 15, || {
        !process_alive(started.output)
    });
    assert_eq!(read_endpoint(&started.directory).unwrap(), None);
}

#[test]
fn a_hard_killed_controller_leaves_an_adoptable_output_process() {
    let started = start_controller("hard");
    kill_process(started.controller, false);
    wait_until("the controller to die", 15, || {
        !process_alive(started.controller)
    });
    assert!(
        process_alive(started.output),
        "the output process outlives a hard-killed controller"
    );
    let endpoint: Endpoint = read_endpoint(&started.directory).unwrap().unwrap();
    assert_eq!(endpoint.pid, started.output);
    adopt_and_release(&started);
    drop(started.cleanup);
}

#[test]
fn a_reconnecting_controller_waits_for_its_old_socket_and_delivers_rapid_takes() {
    struct ChildOutput {
        child: Box<dyn Spawned>,
        directory: PathBuf,
    }
    impl Drop for ChildOutput {
        fn drop(&mut self) {
            self.child.kill();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    struct Link(Arc<OutputLink>);
    impl Drop for Link {
        fn drop(&mut self) {
            self.0.shutdown();
        }
    }
    let directory =
        std::env::temp_dir().join(format!("orbit-output-reconnect-{}", uuid::Uuid::new_v4()));
    let spawner = ProcessSpawner {
        executable: BINARY.into(),
        extra_arguments: vec!["--inert".into(), "--grace-seconds".into(), GRACE.into()],
        events: silent(),
    };
    let _output = ChildOutput {
        child: spawner.spawn(&directory).unwrap(),
        directory: directory.clone(),
    };
    wait_until("the output endpoint", 30, || {
        read_endpoint(&directory).unwrap().is_some()
    });
    let endpoint = read_endpoint(&directory).unwrap().unwrap();
    let mut previous =
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
    previous
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    protocol::write_message(
        &mut previous,
        &protocol::ToOutput::Hello {
            version: protocol::VERSION,
            token: endpoint.token,
            controller_pid: std::process::id(),
            rendering_bit_depth: "normal".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        protocol::read_message::<protocol::FromOutput>(&mut previous).unwrap(),
        protocol::FromOutput::Welcome { .. }
    ));
    let mut initial = scene("previous-socket", "held during reconnect");
    initial["snapshot"]["screenConfig"]["screen"]["enabled"] = json!(false);
    protocol::write_message(
        &mut previous,
        &protocol::ToOutput::Invoke {
            id: 1,
            method: "updateOutputScene".into(),
            args: vec![initial],
        },
    )
    .unwrap();
    loop {
        match protocol::read_message::<protocol::FromOutput>(&mut previous).unwrap() {
            protocol::FromOutput::Result {
                id: 1,
                ok: Some(result),
                error: None,
            } => {
                assert_eq!(result["ok"], true);
                break;
            }
            protocol::FromOutput::Event { .. } => {}
            other => panic!("Unexpected scene response: {other:?}"),
        }
    }
    let events: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let recorded = events.clone();
    let link = Link(OutputLink::start(config(
        &directory,
        Arc::new(NeverSpawn),
        Arc::new(move |name, payload| {
            recorded.lock().unwrap().push((name.to_string(), payload));
        }),
    )));
    wait_until("the connection handover", 10, || {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "output:link-changed")
    });
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, payload)| payload["foreign"] == true),
        "The old socket belongs to this controller, not another application"
    );
    previous.shutdown(std::net::Shutdown::Both).unwrap();
    drop(previous);
    wait_until("the same controller to reconnect", 20, || link.0.attached());
    assert_eq!(
        read_endpoint(&directory).unwrap().unwrap().pid,
        endpoint.pid
    );
    let mut request = scene("rapid-takes", "first reconnected take");
    request["snapshot"]["screenConfig"]["screen"]["enabled"] = json!(false);
    let held = link
        .0
        .invoke("updateOutputScene", vec![request.clone()])
        .unwrap();
    assert_eq!(held["cacheMiss"], true);
    assert_eq!(
        held["snapshot"]["liveSlide"]["content"],
        "held during reconnect"
    );
    request["baseRevision"] = held["revision"].clone();
    request["clientSequence"] = json!(2);
    let accepted = link.0.invoke("updateOutputScene", vec![request]).unwrap();
    assert_eq!(accepted["ok"], true);
    std::thread::scope(|scope| {
        let link = &link.0;
        for _ in 0..4 {
            scope.spawn(move || {
                for _ in 0..64 {
                    assert!(link
                        .invoke("getOutputContent", vec![json!("screen")])
                        .unwrap()
                        .is_object());
                    assert!(link.invoke("getNdiStatus", vec![]).unwrap().is_object());
                }
            });
        }
        scope.spawn(move || {
            let mut revision=accepted["revision"].clone();
            for index in 0..64 {
                let accepted=link.invoke("updateOutputScene",vec![json!({"version":2,"kind":"patch","sessionId":"rapid-takes",
                    "clientSequence":index+3,"baseRevision":revision,"patch":{"activeMessageText":format!("Take {index}")}})]).unwrap();
                assert_eq!(accepted["ok"],true);
                revision=accepted["revision"].clone();
            }
        });
    });
    assert!(link.0.attached());
    assert_eq!(
        link.0
            .invoke("getOutputContent", vec![json!("screen")])
            .unwrap()["messageText"],
        "Take 63"
    );
    assert!(!events
        .lock()
        .unwrap()
        .iter()
        .any(|(name, _)| matches!(name.as_str(), "output:display-error" | "output:link-error")));
}
