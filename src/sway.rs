use std::{
    sync::{Arc, mpsc::Sender},
    thread::spawn,
};

use log::debug;
use mio::Waker;
use swayipc::{BindingEvent, Connection, Event, EventType, WorkspaceChange};

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
        let event_stream = self.sway_conn
            .subscribe([EventType::Binding, EventType::Workspace])
            .unwrap();

        let mut set_workspace_conn = Connection::new().unwrap();
        let output = std::env::var("MULTIBGSWAY_OUTPUT").unwrap()
            .trim().to_string();
        let delay = std::env::var("MULTIBGSWAY_DELAY").unwrap()
            .trim()
            .parse::<u64>().unwrap();

        for event_result in event_stream {
            let event = event_result.unwrap();
            match event {
                /*
                Event::Workspace(workspace_event) => {
                    if let WorkspaceChange::Focus = workspace_event.change {
                        let current_workspace = workspace_event.current
                            .unwrap();

                        self.tx.send(WorkspaceVisible {
                            output: current_workspace.output.unwrap(),
                            workspace_name: current_workspace.name.unwrap(),
                        }).unwrap();

                        self.waker.wake().unwrap();
                    }
                }
                */
                Event::Binding(BindingEvent { binding, .. }) => {
                    if binding.command == "nop"
                        && binding.event_state_mask.iter()
                            .any(|m| m.as_str() == "Mod4")
                    {
                        let Some(symbol) = binding.symbol
                        else { continue };

                        let Ok(workspace_number) = symbol.parse::<u8>()
                        else { continue };

                        if workspace_number > 10 { continue }

                        debug!(
                            "Switching to workspace {}",
                            symbol
                        );

                        self.tx.send(WorkspaceVisible {
                            output: output.clone(),
                            workspace_name: symbol,
                        }).unwrap();

                        self.waker.wake().unwrap();

                        if delay > 0 {
                            std::thread::sleep(
                                std::time::Duration::from_micros(delay)
                            );
                        }

                        set_workspace_conn
                            .run_command(
                                format!(
                                    "workspace number {}",
                                    workspace_number
                                )
                            )
                            .unwrap()
                            .into_iter().for_each(|r| r.unwrap());
                    }
                }
                _ => {}
            }
        }
    }
}
