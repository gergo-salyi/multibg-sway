
use super::{CompositorInterface, WorkspaceVisible, EventSender};
use niri_ipc::{socket::Socket, Event, Request, Response};

pub struct NiriConnectionTask {}

impl NiriConnectionTask {
    pub fn new() -> Self {
        NiriConnectionTask {}
    }

    fn query_workspace(&self, id: u64) -> (String, String) {
        if let Ok((Ok(Response::Workspaces(workspaces)), _)) = Socket::connect()
            .expect("failed to connect to niri socket")
            .send(Request::Workspaces)
        {
            if let Some(workspace) = workspaces.into_iter().find(|w| w.id == id) {
                return (
                    workspace.name.unwrap_or_else(|| format!("{}", workspace.idx)),
                    workspace.output.unwrap_or_else(String::new),
                );
            }
            panic!("unknown workspace id");
        } else {
            panic!("niri workspace query failed");
        }
    }
}
impl CompositorInterface for NiriConnectionTask {
    fn request_visible_workspaces(&mut self) -> Vec<WorkspaceVisible> {
        if let Ok((Ok(Response::Workspaces(workspaces)), _)) = Socket::connect()
            .expect("failed to connect to niri socket")
            .send(Request::Workspaces)
        {
            workspaces
                .into_iter()
                .filter(|w| w.is_active)
                .map(|workspace| WorkspaceVisible {
                    output: workspace.output.unwrap_or_default(),
                    workspace_name: workspace.name
                        .unwrap_or_else(|| format!("{}", workspace.idx)),
                })
                .collect()
        } else {
            panic!("unable to retrieve niri workspaces")
        }
    }

    fn subscribe_event_loop(self, event_sender: EventSender) {
        if let Ok((Ok(Response::Handled), mut callback)) = Socket::connect()
            .expect("failed to connect to niri socket")
            .send(Request::EventStream)
        {
            while let Ok(event) = callback() {
                if let Event::WorkspaceActivated { id, focused: _ } = event {
                    let (workspace_name, output) = self.query_workspace(id);
                    event_sender.send(WorkspaceVisible {
                        output,
                        workspace_name,
                    });
                }
            }
        } else {
            panic!("failed to subscribe to event stream");
        }
    }
}
