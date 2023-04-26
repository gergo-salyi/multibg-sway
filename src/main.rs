mod cli;
mod image;
mod sway;
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
use log::error;
use mio::{
    Events, Interest, Poll, Token, Waker, 
    unix::SourceFd,
};
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    registry::RegistryState,
    shell::wlr_layer::LayerShell,
    shm::{Shm, slot::SlotPool},
};
use smithay_client_toolkit::reexports::client::{
    Connection, EventQueue,
    backend::{ReadEventsGuard, WaylandError},
    globals::registry_queue_init,
};

use crate::{
    cli::Cli,
    sway::{SwayConnectionTask, WorkspaceVisible},
    wayland::State,
};

fn main()
{
    #[cfg(debug_assertions)]
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(
            "warn,multibg_sway=trace"
        )
    ).init();
    
    #[cfg(not(debug_assertions))]
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn")
    ).init();

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

    // Initialize slot pool with a minimum size (0 is not allowed)
    // it will be automatically resized later
    let shm_slot_pool = SlotPool::new(1, &shm).unwrap();
    
    // Sync tools for sway ipc tasks
    let mut poll = Poll::new().unwrap();
    let waker = Arc::new(Waker::new(poll.registry(), SWAY).unwrap());
    let (tx, rx) = channel();

    let mut state = State {
        compositor_state,
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        shm_slot_pool,
        layer_shell,
        wallpaper_dir,
        pixel_format: None,
        background_layers: Vec::new(),
        sway_connection_task: SwayConnectionTask::new(
            tx.clone(), Arc::clone(&waker)
        ),
        brightness: cli.brightness.unwrap_or(0),
        contrast: cli.contrast.unwrap_or(0.0)
    };

    event_queue.roundtrip(&mut state).unwrap();
    
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
    SwayConnectionTask::new(tx, waker).spawn_subscribe_event_loop();

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
