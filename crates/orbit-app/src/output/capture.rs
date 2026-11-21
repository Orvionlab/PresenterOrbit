//! Recording of program output. The picture and the program audio exist in the output process, so the recording
//! runs there: this process resolves the preset (from the workspace) and its stream key (from the app data),
//! which only it can read, and sends them over the link. The output process writes only into the preset's
//! recording directory or stream ingest.

use orbit_host::Host;
use serde_json::{json, Value};

use crate::state::runtime::output_call;

/// `startCapture`: resolves the preset here, then starts the recording where the program output is.
pub fn start(host: &Host, workspace: &str, preset_id: &str) -> Result<Value, String> {
    let resolved = host.invoke(
        "resolveCaptureStart",
        vec![json!(workspace), json!(preset_id)],
    )?;
    output_call(host, "startCaptureResolved", vec![resolved])
}

/// `stopCapture`. A recording ends with its workspace, which the output process does not know about, so a
/// workspace switch calls this too.
pub fn stop(host: &Host) -> Result<Value, String> {
    output_call(host, "stopCapture", Vec::new())
}

/// `stopCapture` for the recording the operator was asked about: it does nothing when another one runs.
pub fn stop_if_current(host: &Host, file_path: &str) -> Result<Value, String> {
    output_call(host, "stopCapture", vec![json!(file_path)])
}

/// `getCaptureStatus` of the recording that runs in the output process.
pub fn status(host: &Host) -> Result<Value, String> {
    output_call(host, "getCaptureStatus", Vec::new())
}
