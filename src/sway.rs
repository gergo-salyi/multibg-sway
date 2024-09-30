use std::{
    sync::{Arc, mpsc::Sender},
    thread::spawn,
};

use mio::Waker;
use swayipc::{Connection, Event, EventType, WorkspaceChange};

#[derive(Debug)]
pub struct WorkspaceVisible {
    pub output: String,
    pub workspace_name: String
}

pub struct SwayConnectionTask {
    sway_conn: Connection,
    tx: Sender<WorkspaceVisible>,
    waker: Arc<Waker>,
}
impl SwayConnectionTask
{
    pub fn new(tx: Sender<WorkspaceVisible>, waker: Arc<Waker>) -> Self {
        SwayConnectionTask {
            sway_conn: Connection::new()
                .expect("Failed to connect to sway socket"),
            tx,
            waker
        }
    }

    pub fn request_visible_workspace(&mut self, output: &str) {
        if let Some(workspace) = self.sway_conn.get_workspaces().unwrap()
            .into_iter()
            .filter(|w| w.visible)
            .find(|w| w.output == output)
        {
            self.tx.send(WorkspaceVisible {
                output: workspace.output,
                workspace_name: workspace.name,
            }).unwrap();

            self.waker.wake().unwrap();
        }
    }

    pub fn request_visible_workspaces(&mut self) {
        for workspace in self.sway_conn.get_workspaces().unwrap()
            .into_iter().filter(|w| w.visible)
        {
            self.tx.send(WorkspaceVisible {
                output: workspace.output,
                workspace_name: workspace.name,
            }).unwrap();
        }
        self.waker.wake().unwrap();
    }

    pub fn spawn_subscribe_event_loop(self) {
        spawn(|| self.subscribe_event_loop());
    }

    fn subscribe_event_loop(self) {
        let event_stream = self.sway_conn.subscribe([EventType::Workspace])
            .unwrap();
        for event_result in event_stream {
            let event = event_result.unwrap();
            let Event::Workspace(workspace_event) = event else {continue};
            if let WorkspaceChange::Focus = workspace_event.change {
                let current_workspace = workspace_event.current.unwrap();

                self.tx.send(WorkspaceVisible {
                    output: current_workspace.output.unwrap(),
                    workspace_name: current_workspace.name.unwrap(),
                }).unwrap();

                self.waker.wake().unwrap();
            }
        }
    }
}
