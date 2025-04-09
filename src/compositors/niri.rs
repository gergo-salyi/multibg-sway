use std::io;
use super::{CompositorInterface, WorkspaceVisible, EventSender};
use niri_ipc::{socket::Socket, Event, Request, Response, Workspace};

pub struct NiriConnectionTask {}

impl NiriConnectionTask {
    pub fn new() -> Self {
        NiriConnectionTask {}
    }
}

impl CompositorInterface for NiriConnectionTask {
    fn request_visible_workspaces(&mut self) -> Vec<WorkspaceVisible> {
        request_workspaces().into_iter()
            .filter(|w| w.is_active)
            .map(|workspace| WorkspaceVisible {
                output: workspace.output.unwrap_or_default(),
                workspace_name: workspace.name
                    .unwrap_or_else(|| format!("{}", workspace.idx)),
            })
            .collect()
    }

    fn subscribe_event_loop(self, event_sender: EventSender) {
        let mut workspaces_state = request_workspaces();
        let mut callback = request_event_stream();
        while let Ok(event) = callback() {
            match event {
                Event::WorkspaceActivated { id, focused: _ } => {
                    let visible_workspace =
                        find_workspace(&workspaces_state, id);
                    event_sender.send(visible_workspace);
                },
                Event::WorkspacesChanged { workspaces } => {
                    workspaces_state = workspaces
                },
                _ => {},
            }
        }
    }
}

fn find_workspace(workspaces: &[Workspace], id: u64) -> WorkspaceVisible {
    let workspace = workspaces.iter()
        .find(|workspace| workspace.id == id)
        .expect("Unknown niri workspace id {id}");
    let workspace_name = workspace.name.clone()
        .unwrap_or_else(|| format!("{}", workspace.idx));
    let output = workspace.output.clone().unwrap_or_default();
    WorkspaceVisible { output, workspace_name }
}

fn request_event_stream() -> impl FnMut() -> Result<Event, io::Error> {
    let Ok((Ok(Response::Handled), callback)) = Socket::connect()
        .expect("failed to connect to niri socket")
        .send(Request::EventStream)
    else {
        panic!("failed to subscribe to event stream");
    };
    callback
}

fn request_workspaces() -> Vec<Workspace> {
    let response = Socket::connect()
        .expect("failed to connect to niri socket")
        .send(Request::Workspaces)
        .expect("failed to send niri ipc request")
        .0
        .expect("niri workspace query failed");
    let Response::Workspaces(workspaces) = response else {
        panic!("unexpected response from niri");
    };
    workspaces
}
