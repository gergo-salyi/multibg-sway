use std::{num::NonZeroU32, path::PathBuf};

use log::{debug, error, warn};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, 
    delegate_shm,
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            KeyboardInteractivity, Layer, LayerShell, 
            LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool}, 
    },
};
use smithay_client_toolkit::reexports::client::{
    Connection, QueueHandle,
    protocol::{wl_output, wl_shm, wl_surface},
};

use crate::{
    image::workspace_bgs_from_output_image_dir,
    sway::SwayConnectionTask,
};

pub struct State {
    pub compositor_state: CompositorState,
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub shm: Shm,
    pub layer_shell: LayerShell,
    pub wallpaper_dir: PathBuf,
    pub pixel_format: Option<wl_shm::Format>,
    pub background_layers: Vec<BackgroundLayer>,
    pub sway_connection_task: SwayConnectionTask,
    pub brightness: i32,
    pub contrast: f32,
}

impl State {
    fn pixel_format(&mut self) -> wl_shm::Format {
        *self.pixel_format.get_or_insert_with(|| {
            // Consume less gpu memory by using Bgr888 if available,
            // fall back to the always supported Xrgb8888 otherwise
            for format in self.shm.formats() {
                if let wl_shm::Format::Bgr888 = format {
                    debug!("Using pixel format: {:?}", format);
                    return *format
                }
                // XXX: One may add Rgb888 and HDR support here
            }
            debug!("Using default pixel format: Xrgb8888");

            wl_shm::Format::Xrgb8888
        })
    }
}

impl CompositorHandler for State
{
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }
}

impl LayerShellHandler for State
{
    fn closed(
        &mut self, 
        _conn: &Connection, 
        _qh: &QueueHandle<Self>, 
        _layer: &LayerSurface
    ) {
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        _configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // The new layer is ready: request all the visible workspace from sway, 
        // it will get picked up by the main event loop and be drawn from there
        let bg_layer = self.background_layers.iter_mut()
            .find(|bg_layer| &bg_layer.layer == layer).unwrap();

        if !bg_layer.configured {
            bg_layer.configured = true;
            self.sway_connection_task
                .request_visible_workspace(&bg_layer.output_name);

            debug!(
                "Background layer has been configured for output: {}",
                bg_layer.output_name
            );
        }
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output)
        else {
            error!("New output has no output info, skipping");
            return;
        };

        let Some(output_name) = info.name
        else {
            error!("New output has no name, skipping");
            return;
        };

        let Some((width, height)) = info.modes.iter()
            .find(|mode| mode.current)
            .map(|mode| mode.dimensions)
        else {
            error!(
                "New output '{}' has no current mode set, skipping", 
                output_name
            );
            return;
        };

        if !width.is_positive() {
            error!(
                "New output '{}' has a non-positive width: {}, skipping",
                output_name,
                width
            );
            return;
        }
        if !height.is_positive() {
            error!(
                "New output '{}' has a non-positive height: {}, skipping",
                output_name,
                height
            );
            return;
        }

        debug!(
            "New output, name: {}, resolution: {}x{}",
            output_name, width, height
        );

        let surface = self.compositor_state.create_surface(qh);

        let layer = self.layer_shell.create_layer_surface(
            qh, 
            surface, 
            Layer::Background, 
            layer_surface_name(&output_name), 
            Some(&output)
        );

        layer.set_exclusive_zone(-1); // Don't let the status bar push it around
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(width as u32, height as u32);
        
        layer.commit();

        let pixel_format = self.pixel_format();

        let output_wallpaper_dir = self.wallpaper_dir.join(&output_name);

        // Initialize slot pool with a minimum size (0 is not allowed)
        // it will be automatically resized later
        let mut shm_slot_pool = SlotPool::new(1, &self.shm).unwrap();

        let workspace_backgrounds = match workspace_bgs_from_output_image_dir(
            &output_wallpaper_dir,
            &mut shm_slot_pool,
            pixel_format,
            self.brightness,
            self.contrast,
            NonZeroU32::new(width as u32).unwrap(),
            NonZeroU32::new(height as u32).unwrap()
        ) {
            Ok(workspace_bgs) => {
                debug!(
                    "Loaded {} wallpapers on new output for workspaces: {}",
                    workspace_bgs.len(),
                    workspace_bgs.iter()
                        .map(|workspace_bg| workspace_bg.workspace_name.as_str())
                        .collect::<Vec<_>>().join(", ")
                );
                workspace_bgs
            },
            Err(e) => {
                error!(
            "Failed to get wallpapers for new output '{}' form '{:?}': {}",
                    output_name, output_wallpaper_dir, e
                );
                return;
            }
        };
        
        debug!(
            "Shm slot pool size for output '{}' after loading wallpapers: {} KiB",
            output_name,
            shm_slot_pool.len() / 1024
        );

        self.background_layers.push(BackgroundLayer { 
            output_name, 
            width, 
            height, 
            layer, 
            configured: false,
            workspace_backgrounds,
            shm_slot_pool,
        });
        
        debug!(
            "New sum of shm slot pool sizes for all outputs: {} KiB",
            self.background_layers.iter()
                .map(|bg_layer| bg_layer.shm_slot_pool.len())
                .sum::<usize>() / 1024
        );
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // This will only be needed if we implement scaling the wallpapers
        // to the output resolution
        
        let Some(info) = self.output_state.info(&output)
        else {
            error!("Updated output has no output info, skipping");
            return;
        };
        
        let Some(name) = info.name
        else {
            error!("Updated output has no name, skipping");
            return;
        };
        
        debug!(
            "Update output: {}",
            name
        );

        warn!("Handling of output updates are not yet implemented");
        
        // let Some((width, height)) = info.modes.iter()
        //     .find(|mode| mode.current)
        //     .map(|mode| mode.dimensions)
        // else {
        //     error!(
        //         "Updated output '{}' has no current mode set, skipping", 
        //         name
        //     );
        //     return;
        // };
        //
        // if let Some(bg_layer) = self.background_layers.iter()
        //     .find(|bg_layers| bg_layers.output_name == name)
        // {
        //     if bg_layer.width == width && bg_layer.height == height {
        //         // if a known output has its resolution unchanged
        //         // then ignore this event
        //         return;
        //     }
        // }
        //
        // // renew the output otherwise
        // self.output_destroyed(conn, qh, output.clone());
        // self.new_output(conn, qh, output)
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output)
        else {
            error!("Destroyed output has no output info, skipping");
            return;
        };

        let Some(name) = info.name
        else {
            error!("Destroyed output has no name, skipping");
            return;
        };
        
        debug!(
            "Output destroyed: {}",
            name,
        );

        if let Some(bg_layer_index) = self.background_layers.iter()
            .position(|bg_layers| bg_layers.output_name == name)
        {
            let removed_bg_layer = self.background_layers
                .swap_remove(bg_layer_index);

            // Workspaces on the destroyed output may have been moved anywhere
            // so reset the wallpaper on all the visible workspaces
            self.sway_connection_task.request_visible_workspaces();

            debug!(
                "Dropping {} wallpapers on destroyed output for workspaces: {}",
                removed_bg_layer.workspace_backgrounds.len(),
                removed_bg_layer.workspace_backgrounds.iter()
                    .map(|workspace_bg| workspace_bg.workspace_name.as_str())
                    .collect::<Vec<_>>().join(", ")
            );

            for workspace_bg in removed_bg_layer.workspace_backgrounds.iter() {
                if workspace_bg.buffer.slot().has_active_buffers() {
                    warn!(
"On destroyed output '{}' workspace background '{}' will be dropped while its shm slot still has active buffers", 
                        name,
                        workspace_bg.workspace_name,
                    );
                }
            }

            drop(removed_bg_layer);
        }
        else {
            error!(
    "Ignoring destroyed output with unknown name '{}', known outputs were: {}",
                name,
                self.background_layers.iter()
                    .map(|bg_layer| bg_layer.output_name.as_str())
                    .collect::<Vec<_>>().join(", ")
            );
        }
        
        debug!(
            "New sum of shm slot pool sizes for all outputs: {} KiB",
            self.background_layers.iter()
                .map(|bg_layer| bg_layer.shm_slot_pool.len())
                .sum::<usize>() / 1024
        );
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(State);
delegate_layer!(State);
delegate_output!(State);
delegate_registry!(State);
delegate_shm!(State);

pub struct BackgroundLayer {
    pub output_name: String,
    pub width: i32,
    pub height: i32,
    pub layer: LayerSurface,
    pub configured: bool,
    pub workspace_backgrounds: Vec<WorkspaceBackground>,
    pub shm_slot_pool: SlotPool
}
impl BackgroundLayer 
{
    pub fn draw_workspace_bg(&mut self, workspace_name: &str)
    {
        if !self.configured {
            error!(
"Cannot draw wallpaper image on the not yet configured layer for output: {}",
                self.output_name
            );
            return;
        }

        let Some(workspace_bg) = self.workspace_backgrounds.iter()
            .find(|workspace_bg| workspace_bg.workspace_name == workspace_name)
            .or_else(|| self.workspace_backgrounds.iter()
                .find(|workspace_bg| workspace_bg.workspace_name == "_default")
            )
        else {
            error!(
"There is no wallpaper image on output '{}' for workspace '{}', only for: {}",
                self.output_name,
                workspace_name,
                self.workspace_backgrounds.iter()
                    .map(|workspace_bg| workspace_bg.workspace_name.as_str())
                    .collect::<Vec<_>>().join(", ")
            );
            return;
        };

        if workspace_bg.buffer.slot().has_active_buffers() {
            debug!(
"Skipping draw on output '{}' for workspace '{}' because its buffer already active",
                self.output_name,
                workspace_name,
            );
            return;
        }
        
        // Attach and commit to new workspace background
        if let Err(e) = workspace_bg.buffer.attach_to(self.layer.wl_surface()) {
            error!(
            "Error attaching buffer of workspace '{}' on output '{}': {:#?}",
                workspace_name,
                self.output_name,
                e
            );
            return;
        }
        
        // Damage the entire surface
        self.layer.wl_surface().damage_buffer(0, 0, self.width, self.height);
        
        self.layer.commit();

        debug!(
            "Setting wallpaper on output '{}' for workspace: {}",
            self.output_name, workspace_name
        );
    }
}

pub struct WorkspaceBackground {
    pub workspace_name: String,
    pub buffer: Buffer,
}

fn layer_surface_name(output_name: &str) -> Option<String> {
    Some([env!("CARGO_PKG_NAME"), "_wallpaper_", output_name].concat())
}
