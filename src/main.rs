mod cli;
mod image;
mod compositors;
mod wayland;

use std::{
    io,
    os::fd::AsRawFd,
    path::Path,
    sync::{
        Arc,
        mpsc::{channel, Receiver},
    }
};

use clap::Parser;
use log::{debug, error, info};
use mio::{
    Events, Interest, Poll, Token, Waker,
    unix::SourceFd,
};
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    registry::RegistryState,
    shell::wlr_layer::LayerShell,
    shm::Shm,
};
use smithay_client_toolkit::reexports::client::{
    Connection, EventQueue,
    backend::{ReadEventsGuard, WaylandError},
    globals::registry_queue_init,
};
use smithay_client_toolkit::reexports::protocols
    ::wp::viewporter::client::wp_viewporter::WpViewporter;

use crate::{
    cli::{Cli, PixelFormat},
    compositors::{Compositor, ConnectionTask, WorkspaceVisible},
    wayland::State,
};

fn main() -> Result<(), ()> {
    run().map_err(|e| { error!("{e:#}"); })
}

fn run() -> anyhow::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(
            #[cfg(debug_assertions)]
            "info,multibg_sway=trace",
            #[cfg(not(debug_assertions))]
            "info",
        )
    ).init();

    info!(concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")));

    let cli = Cli::parse();
    let wallpaper_dir = Path::new(&cli.wallpaper_dir).canonicalize().unwrap();

    // ********************************
    //     Initialize wayland client
    // ********************************

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let compositor_state = CompositorState::bind(&globals, &qh).unwrap();
    let layer_shell = LayerShell::bind(&globals, &qh).unwrap();
    let shm = Shm::bind(&globals, &qh).unwrap();

    let registry_state = RegistryState::new(&globals);

    let viewporter: WpViewporter = registry_state
        .bind_one(&qh, 1..=1, ()).expect("wp_viewporter not available");

    // Sync tools for sway ipc tasks
    let mut poll = Poll::new().unwrap();
    let waker = Arc::new(Waker::new(poll.registry(), SWAY).unwrap());
    let (tx, rx) = channel();

    let compositor = cli.compositor
        .or_else(Compositor::from_env)
        .unwrap_or(Compositor::Sway);

    let mut state = State {
        compositor_state,
        registry_state,
        output_state: OutputState::new(&globals, &qh),
        shm,
        layer_shell,
        viewporter,
        wallpaper_dir,
        force_xrgb8888: cli.pixelformat
            .is_some_and(|p| p == PixelFormat::Baseline),
        pixel_format: None,
        background_layers: Vec::new(),
        compositor_connection_task: ConnectionTask::new(
            compositor,
            tx.clone(), Arc::clone(&waker)
        ),
        brightness: cli.brightness.unwrap_or(0),
        contrast: cli.contrast.unwrap_or(0.0),
    };

    event_queue.roundtrip(&mut state).unwrap();

    debug!("Initial wayland roundtrip done. Starting main event loop.");

    // ********************************
    //     Main event loop
    // ********************************

    let mut events = Events::with_capacity(16);

    const WAYLAND: Token = Token(0);
    let read_guard = event_queue.prepare_read().unwrap();
    let wayland_socket_fd = read_guard.connection_fd().as_raw_fd();
    poll.registry().register(
        &mut SourceFd(&wayland_socket_fd),
        WAYLAND,
        Interest::READABLE
    ).unwrap();
    drop(read_guard);

    const SWAY: Token = Token(1);
    ConnectionTask::spawn_subscribe_event_loop(compositor, tx, waker);

    loop {
        event_queue.flush().unwrap();
        event_queue.dispatch_pending(&mut state).unwrap();
        let mut read_guard_option = Some(event_queue.prepare_read().unwrap());

        if let Err(poll_error) = poll.poll(&mut events, None) {
            if poll_error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            else {
                panic!("Main event loop poll failed: {:?}", poll_error);
            }
        }

        for event in events.iter() {
            match event.token() {
                WAYLAND => handle_wayland_event(
                    &mut state,
                    &mut read_guard_option,
                    &mut event_queue
                ),
                SWAY => handle_sway_event(&mut state, &rx),
                _ => unreachable!()
            }
        }
    }
}

fn handle_wayland_event(
    state: &mut State,
    read_guard_option: &mut Option<ReadEventsGuard>,
    event_queue: &mut EventQueue<State>,
) {
    if let Some(read_guard) = read_guard_option.take() {
        if let Err(e) = read_guard.read() {
            // WouldBlock is normal here because of epoll false wakeups
            if let WaylandError::Io(ref io_err) = e {
                if io_err.kind() == io::ErrorKind::WouldBlock {
                    return;
                }
            }
            panic!("Failed to read Wayland events: {}", e)
        }

        if let Err(e) = event_queue.dispatch_pending(state) {
            panic!("Failed to dispatch pending Wayland events: {}", e);
        }
    }
}

fn handle_sway_event(
    state: &mut State,
    rx: &Receiver<WorkspaceVisible>,
) {
    while let Ok(workspace) = rx.try_recv()
    {
        // Find the background layer that of the output where the workspace is
        if let Some(affected_bg_layer) = state.background_layers.iter_mut()
            .find(|bg_layer| bg_layer.output_name == workspace.output)
        {
            affected_bg_layer.draw_workspace_bg(&workspace.workspace_name);
        }
        else {
            error!(
        "Workspace '{}' is on an unknown output '{}', known outputs were: {}",
                workspace.workspace_name,
                workspace.output,
                state.background_layers.iter()
                    .map(|bg_layer| bg_layer.output_name.as_str())
                    .collect::<Vec<_>>().join(", ")
            );
            continue;
        };
    }
}
