use crate::daemon_probe::{connect, send_request, socket_path};
use anyhow::{Context, Result};
use lianli_shared::daemon::DaemonInfo;
use lianli_shared::installation::InstallationContext;
use lianli_shared::ipc::IpcRequest;
use lianli_shared::playlist::{PlaylistAction, PlaylistStatus};
use lianli_shared::services::ServiceScope;
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Reply {
    device_id: String,
    status: PlaylistStatus,
}

/// Sends a transport command to the running daemon, trying the user service
/// and then the system service, and returns one line per affected screen.
pub fn run(action: PlaylistAction, device_id: Option<String>) -> Result<Vec<String>> {
    let context = InstallationContext::detect();
    let mut last_error = None;
    for scope in [ServiceScope::User, ServiceScope::System] {
        let attempt = (|| {
            let path = socket_path(&context, scope)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = connect(&path, deadline)?;
            let info: DaemonInfo = serde_json::from_value(send_request(
                &mut stream,
                &IpcRequest::GetDaemonInfo,
                deadline,
                64 * 1024,
            )?)?;
            let guard = info
                .checked_write_guard(env!("CARGO_PKG_VERSION"))
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let request = IpcRequest::Guarded {
                guard,
                request: Box::new(IpcRequest::PlaylistControl {
                    device_id: device_id.clone(),
                    action,
                }),
            };
            let replies: Vec<Reply> =
                serde_json::from_value(send_request(&mut stream, &request, deadline, 64 * 1024)?)?;
            Ok::<_, anyhow::Error>(replies.into_iter().map(describe).collect())
        })();
        match attempt {
            Ok(lines) => return Ok(lines),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("tried at least one service")).context("Controlling the playlist")
}

fn describe(reply: Reply) -> String {
    let state = if reply.status.paused {
        "paused"
    } else {
        "playing"
    };
    let item = reply
        .status
        .current
        .as_deref()
        .and_then(|path| path.file_name())
        .map_or_else(
            || "readouts".into(),
            |name| name.to_string_lossy().into_owned(),
        );
    let set = if reply.status.alternate {
        " (alternate)"
    } else {
        ""
    };
    format!("{}: {state} {item}{set}", reply.device_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_describe_state_item_and_set() {
        let reply: Vec<Reply> = serde_json::from_value(serde_json::json!([
            {"device_id": "hid:lcd", "status": {"paused": true, "current": "/m/clip.mp4", "alternate": false}},
            {"device_id": "hid:two", "status": {"paused": false, "current": null, "alternate": true}}
        ]))
        .unwrap();
        let lines: Vec<String> = reply.into_iter().map(describe).collect();
        assert_eq!(
            lines,
            [
                "hid:lcd: paused clip.mp4",
                "hid:two: playing readouts (alternate)"
            ]
        );
    }
}
