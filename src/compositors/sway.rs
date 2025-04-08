use std::{
    sync::{mpsc::Sender, Arc},
};

use super::{CompositorInterface, WorkspaceVisible};
use mio::Waker;
use swayipc::{Connection, Event, EventType, WorkspaceChange};

pub struct SwayConnectionTask {
    sway_conn: Connection,
}

impl SwayConnectionTask {
    pub fn new() -> Self {
        SwayConnectionTask {
            sway_conn: Connection::new().expect("Failed to connect to sway socket. If you're not using sway, pass the correct --compositor argument. Original cause"),
        }
    }
}

impl CompositorInterface for SwayConnectionTask {
    fn request_visible_workspace(
        &mut self,
        output: &str,
        tx: Sender<WorkspaceVisible>,
        waker: Arc<Waker>,
    ) {
        if let Some(workspace) = self
            .sway_conn
            .get_workspaces()
            .unwrap()
            .into_iter()
            .filter(|w| w.visible)
            .find(|w| w.output == output)
        {
            tx
                .send(WorkspaceVisible {
                    output: workspace.output,
                    workspace_name: workspace.name,
                })
                .unwrap();

            waker.wake().unwrap();
        }
    }

    fn request_visible_workspaces(&mut self, tx: Sender<WorkspaceVisible>, waker: Arc<Waker>) {
        for workspace in self
            .sway_conn
            .get_workspaces()
            .unwrap()
            .into_iter()
            .filter(|w| w.visible)
        {
            tx
                .send(WorkspaceVisible {
                    output: workspace.output,
                    workspace_name: workspace.name,
                })
                .unwrap();
        }
        waker.wake().unwrap();
    }

    fn subscribe_event_loop(self, tx: Sender<WorkspaceVisible>, waker: Arc<Waker>) {
        let event_stream = self.sway_conn.subscribe([EventType::Workspace]).unwrap();
        for event_result in event_stream {
            let event = event_result.unwrap();
            let Event::Workspace(workspace_event) = event else {
                continue;
            };
            if let WorkspaceChange::Focus = workspace_event.change {
                let current_workspace = workspace_event.current.unwrap();

                tx
                    .send(WorkspaceVisible {
                        output: current_workspace.output.unwrap(),
                        workspace_name: current_workspace.name.unwrap(),
                    })
                    .unwrap();

                waker.wake().unwrap();
            }
        }
    }
}
