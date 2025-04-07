mod niri;
mod sway;

use mio::Waker;
use std::{
    sync::{mpsc::Sender, Arc}, thread::spawn
};

#[derive(Clone, Copy, Debug)]
pub enum Compositor {
    Sway,
    Niri,
}

impl From<&str> for Compositor {
    fn from(s: &str) -> Self {
        match s {
            "sway" => Compositor::Sway,
            "niri" => Compositor::Niri,
            _ => panic!("Unknown compositor"),
        }
    }
}

pub trait CompositorInterface: Send + Sync {
    fn request_visible_workspace(
        &mut self,
        output: &str,
        tx: Sender<WorkspaceVisible>,
        waker: Arc<Waker>,
    );
    fn request_visible_workspaces(&mut self, tx: Sender<WorkspaceVisible>, waker: Arc<Waker>);
    fn subscribe_event_loop(self, tx: Sender<WorkspaceVisible>, waker: Arc<Waker>);
}

pub struct ConnectionTask {
    tx: Sender<WorkspaceVisible>,
    waker: Arc<Waker>,
    interface: Box<dyn CompositorInterface>,
}

impl ConnectionTask {
    pub fn new(composer: Compositor, tx: Sender<WorkspaceVisible>, waker: Arc<Waker>) -> Self {
        let interface: Box<dyn CompositorInterface> = match composer {
            Compositor::Sway => Box::new(sway::SwayConnectionTask::new()),
            Compositor::Niri => Box::new(niri::NiriConnectionTask::new()),
        };

        ConnectionTask {
            tx,
            waker,
            interface,
        }
    }

    pub fn spawn_subscribe_event_loop(
        composer: Compositor,
        tx: Sender<WorkspaceVisible>,
        waker: Arc<Waker>,
    ) {
        spawn(move || match composer {
            Compositor::Sway => {
                let composer_interface = sway::SwayConnectionTask::new();
                composer_interface.subscribe_event_loop(tx, waker);
            }
            Compositor::Niri => {
                let composer_interface = niri::NiriConnectionTask::new();
                composer_interface.subscribe_event_loop(tx, waker);
            }
        });
    }

    pub fn request_visible_workspace(&mut self, output: &str) {
        self.interface
            .request_visible_workspace(output, self.tx.clone(), self.waker.clone());
    }

    pub fn request_visible_workspaces(&mut self) {
        self.interface
            .request_visible_workspaces(self.tx.clone(), self.waker.clone());
    }
}

#[derive(Debug)]
pub struct WorkspaceVisible {
    pub output: String,
    pub workspace_name: String,
}
