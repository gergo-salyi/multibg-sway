use std::{
    sync::{mpsc::Sender, Arc},
    thread::spawn,
};

use mio::Waker;
use niri_ipc::{socket::Socket, Event, Request, Response};

#[derive(Debug)]
pub struct WorkspaceVisible {
    pub output: String,
    pub workspace_name: String,
}

pub struct SwayConnectionTask {
    tx: Sender<WorkspaceVisible>,
    waker: Arc<Waker>,
}
impl SwayConnectionTask {
    pub fn new(tx: Sender<WorkspaceVisible>, waker: Arc<Waker>) -> Self {
        SwayConnectionTask { tx, waker }
    }

    pub fn request_visible_workspace(&mut self, output: &str) {
        if let Ok((Ok(Response::Workspaces(workspaces)), _)) = Socket::connect()
            .expect("failed to connect to niri socket")
            .send(Request::Workspaces)
        {
            if let Some(workspace) = workspaces
                .into_iter()
                .filter(|w| w.is_focused)
                .find(|w| w.output.as_ref().map_or("", |v| v) == output)
            {
                self.tx
                    .send(WorkspaceVisible {
                        output: workspace.output.unwrap_or_else(|| String::new()),
                        workspace_name: workspace.name.unwrap_or_else(|| String::new()),
                    })
                    .unwrap();

                self.waker.wake().unwrap();
            }
        }
    }

    pub fn request_visible_workspaces(&mut self) {
        if let Ok((Ok(Response::Workspaces(workspaces)), _)) = Socket::connect()
            .expect("failed to connect to niri socket")
            .send(Request::Workspaces)
        {
            for workspace in workspaces.into_iter().filter(|w| w.is_active) {
                self.tx
                    .send(WorkspaceVisible {
                        output: workspace.output.unwrap_or_else(|| String::new()),
                        workspace_name: workspace.name.unwrap_or_else(|| String::new()),
                    })
                    .unwrap();

                self.waker.wake().unwrap();
            }
        }
    }

    pub fn spawn_subscribe_event_loop(self) {
        spawn(|| self.subscribe_event_loop());
    }

    fn subscribe_event_loop(self) {
        if let Ok((Ok(Response::Handled), mut callback)) = Socket::connect()
            .expect("failed to connect to niri socket")
            .send(Request::EventStream)
        {
            while let Ok(event) = callback() {
                if let Event::WorkspaceActivated { id, focused: _ } = event {
                    let (name, output) = self.query_workspace(id);

                    self.tx
                        .send(WorkspaceVisible {
                            output: output,
                            workspace_name: name,
                        })
                        .unwrap();

                    self.waker.wake().unwrap();
                }
            }
        } else {
            panic!("failed to subscribe to event stream");
        }
    }

    fn query_workspace(&self, id: u64) -> (String, String) {
        if let Ok((Ok(Response::Workspaces(workspaces)), _)) = Socket::connect()
            .expect("failed to connect to niri socket")
            .send(Request::Workspaces)
        {
            for workspace in workspaces.into_iter().filter(|w| w.id == id) {
                return (
                    workspace.name.unwrap_or_else(|| String::new()),
                    workspace.output.unwrap_or_else(|| String::new()),
                );
            }
            panic!("unknown workspace id");
        } else {
            panic!("niri workspace query failed");
        }
    }
}
