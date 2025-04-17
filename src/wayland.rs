use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
};

use log::{debug, error, warn};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    delegate_shm,
    compositor::{CompositorHandler, Region},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer,
            LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        raw::RawPool,
    },
};
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, Proxy, QueueHandle,
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::{self, Transform, WlOutput},
        wl_shm,
        wl_surface::WlSurface,
    },
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{
    wp_viewport::WpViewport,
    wp_viewporter::WpViewporter
};

use crate::{
    State,
    image::{load_wallpaper, output_wallpaper_files, WallpaperFile},
};

impl CompositorHandler for State
{
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
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
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // The new layer is ready: request all the visible workspace from sway,
        // it will get picked up by the main event loop and be drawn from there
        let bg_layer = self.background_layers.iter_mut()
            .find(|bg_layer| &bg_layer.layer == layer).unwrap();

        if !bg_layer.configured {
            bg_layer.configured = true;
            self.compositor_connection_task
                .request_visible_workspace(&bg_layer.output_name);

            debug!(
                "Configured layer on output: {}, new surface size {}x{}",
                bg_layer.output_name,
                configure.new_size.0, configure.new_size.1
            );
        }
        else {
            debug!(
"Ignoring configure for already configured layer on output: {}, \
new surface size {}x{}",
                bg_layer.output_name,
                configure.new_size.0, configure.new_size.1
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
        output: WlOutput,
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

        if !width.is_positive() || !height.is_positive() {
            error!(
            "New output '{}' has non-positive resolution: {} x {}, skipping",
                output_name, width, height
            );
            return;
        }

        let (width, height) = {
            match info.transform {
                Transform::Normal
                | Transform::_180
                | Transform::Flipped
                | Transform::Flipped180 => (width, height),
                Transform::_90
                | Transform::_270
                | Transform::Flipped90
                | Transform::Flipped270 => (height, width),
                _ => {
                    warn!(
                        "New output '{}' has unsupported transform",
                        output_name
                    );
                    (width, height)
                }
            }
        };

        let integer_scale_factor = info.scale_factor;

        let Some((logical_width, logical_height)) = info.logical_size
        else {
            error!(
                "New output '{}' has no logical_size, skipping",
                output_name
            );
            return;
        };

        if !logical_width.is_positive() || !logical_height.is_positive() {
            error!(
            "New output '{}' has non-positive logical size: {} x {}, skipping",
                output_name, logical_width, logical_height
            );
            return;
        }

        debug!(
"New output, name: {}, resolution: {}x{}, integer scale factor: {}, \
logical size: {}x{}, transform: {:?}",
            output_name, width, height, integer_scale_factor,
            logical_width, logical_height, info.transform
        );

        let layer = self.layer_shell.create_layer_surface(
            qh,
            self.compositor_state.create_surface(qh),
            Layer::Background,
            layer_surface_name(&output_name),
            Some(&output)
        );

        layer.set_anchor(
            Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT
        );
        layer.set_exclusive_zone(-1); // Don't let the status bar push it around
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);

        let surface = layer.wl_surface();

        // Disable receiving pointer, touch, and tablet events
        // by setting an empty input region.
        // This prevents disappearing or hidden cursor when a normal window
        // closes below the pointer leaving it above our surface
        match Region::new(&self.compositor_state) {
            Ok(region) => surface.set_input_region(Some(region.wl_region())),
            Err(error) => error!(
                "Failed to create empty input region, on new output '{}': {}",
                output_name, error
            )
        };

        let mut viewport = None;

        if width == logical_width || height == logical_height {
            debug!("Output '{}' needs no scaling", output_name);
        }
        else if width == logical_width * integer_scale_factor
            && height == logical_height * integer_scale_factor
        {
            debug!("Output '{}' needs integer scaling", output_name);
            surface.set_buffer_scale(integer_scale_factor);
        }
        else {
            debug!("Output '{}' needs fractional scaling", output_name);
            let new_viewport = self.viewporter.get_viewport(surface, qh, ());
            new_viewport.set_destination(logical_width, logical_height);
            viewport = Some(new_viewport);
        }

        layer.commit();

        let pixel_format = self.pixel_format();
        let output_dir = self.wallpaper_dir.join(&output_name);
        debug!("Looking for wallpapers for new output {} in {:?}",
            output_name, output_dir);
        let wallpaper_files = match output_wallpaper_files(&output_dir) {
            Ok(wallpaper_files) => wallpaper_files,
            Err(e) => {
                error!("Failed to get wallpapers for new output {output_name} \
                    form {output_dir:?}: {e:#}");
                return
            }
        };
        let mut workspace_backgrounds = Vec::new();
        let mut resizer = fast_image_resize::Resizer::new();
        let mut reused_count = 0usize;
        let mut loaded_count = 0usize;
        let mut error_count = 0usize;
        for wallpaper_file in wallpaper_files {
            if log::log_enabled!(log::Level::Debug) {
                if wallpaper_file.path == wallpaper_file.canon_path {
                    debug!("Wallpaper file {:?} for workspace {}",
                        wallpaper_file.path, wallpaper_file.workspace);
                } else {
                    debug!("Wallpaper file {:?} -> {:?} for workspace {}",
                        wallpaper_file.path, wallpaper_file.canon_path,
                        wallpaper_file.workspace);
                }
            }
            if let Some(wallpaper) = find_equal_output_wallpaper(
                &workspace_backgrounds,
                &wallpaper_file
            ) {
                workspace_backgrounds.push(WorkspaceBackground {
                    workspace_name: wallpaper_file.workspace,
                    wallpaper
                });
                reused_count += 1;
                continue
            }
            if let Some(wallpaper) = find_equal_wallpaper(
                &self.background_layers,
                width,
                height,
                info.transform,
                &wallpaper_file
            ) {
                workspace_backgrounds.push(WorkspaceBackground {
                    workspace_name: wallpaper_file.workspace,
                    wallpaper
                });
                reused_count += 1;
                continue
            }
            let stride = match pixel_format {
                wl_shm::Format::Xrgb8888 => width as usize * 4,
                wl_shm::Format::Bgr888 => {
                    // Align buffer stride to both 4 and pixel format
                    // block size. Not being aligned to 4 caused
                    // https://github.com/gergo-salyi/multibg-sway/issues/6
                    (width as usize * 3).next_multiple_of(4)
                },
                _ => unreachable!()
            };
            let shm_size = stride * height as usize;
            let mut shm_pool = match RawPool::new(shm_size, &self.shm) {
                Ok(shm_pool) => shm_pool,
                Err(e) => {
                    error!("Failed to create shm pool: {e}");
                    error_count += 1;
                    continue
                }
            };
            if let Err(e) = load_wallpaper(
                &wallpaper_file.path,
                &mut shm_pool.mmap()[..shm_size],
                width as u32,
                height as u32,
                stride,
                pixel_format,
                self.color_transform,
                &mut resizer
            ) {
                error!("Failed to load wallpaper: {e:#}");
                error_count += 1;
                continue
            }
            let wl_buffer = shm_pool.create_buffer(
                0,
                width,
                height,
                stride.try_into().unwrap(),
                pixel_format,
                (),
                qh
            );
            workspace_backgrounds.push(WorkspaceBackground {
                workspace_name: wallpaper_file.workspace,
                wallpaper: Rc::new(Wallpaper {
                    wl_buffer,
                    active_count: Cell::new(0),
                    shm_pool,
                    canon_path: wallpaper_file.canon_path,
                    canon_modified: wallpaper_file.canon_modified,
                })
            });
            loaded_count += 1;
        }
        debug!("Wallpapers for new output: {} reused, {} loaded, {} errors",
            reused_count, loaded_count, error_count);
        debug!("Wallpapers are available for workspaces: {}",
            workspace_backgrounds.iter()
                .map(|bg| bg.workspace_name.as_str())
                .collect::<Vec<_>>().join(", "));
        self.background_layers.push(BackgroundLayer {
            output_name,
            width,
            height,
            layer,
            configured: false,
            workspace_backgrounds,
            current_workspace: None,
            transform: info.transform,
            viewport,
        });
        print_memory_stats(&self.background_layers);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output)
        else {
            error!("Updated output has no output info, skipping");
            return;
        };

        let Some(output_name) = info.name
        else {
            error!("Updated output has no name, skipping");
            return;
        };

        let Some((width, height)) = info.modes.iter()
            .find(|mode| mode.current)
            .map(|mode| mode.dimensions)
        else {
            error!(
                "Updated output '{}' has no current mode set, skipping",
                output_name
            );
            return;
        };

        if !width.is_positive() || !height.is_positive() {
            error!(
        "Updated output '{}' has non-positive resolution: {} x {}, skipping",
                output_name, width, height
            );
            return;
        }

        let (width, height) = {
            match info.transform {
                Transform::Normal
                | Transform::_180
                | Transform::Flipped
                | Transform::Flipped180 => (width, height),
                Transform::_90
                | Transform::_270
                | Transform::Flipped90
                | Transform::Flipped270 => (height, width),
                _ => {
                    warn!(
                        "Updated output '{}' has unsupported transform",
                        output_name
                    );
                    (width, height)
                }
            }
        };

        let integer_scale_factor = info.scale_factor;

        let Some((logical_width, logical_height)) = info.logical_size
        else {
            error!(
                "Updated output '{}' has no logical_size, skipping",
                output_name
            );
            return;
        };

        if !logical_width.is_positive() || !logical_height.is_positive() {
            error!(
        "Updated output '{}' has non-positive logical size: {} x {}, skipping",
                output_name, logical_width, logical_height
            );
            return;
        }

        debug!(
"Updated output, name: {}, resolution: {}x{}, integer scale factor: {}, \
logical size: {}x{}, transform: {:?}",
            output_name, width, height, integer_scale_factor,
            logical_width, logical_height, info.transform
        );

        let Some(bg_layer) = self.background_layers.iter_mut()
            .find(|bg_layers| bg_layers.output_name == output_name)
        else {
            error!(
                "Updated output '{}' has no background layer, skipping",
                output_name
            );
            return;
        };

        if bg_layer.width != width || bg_layer.height != height {
            warn!(
"Handling of output mode or transform changes are not yet implemented. \
Restart multibg-sway or expect broken wallpapers or low quality due to scaling"
            );
        }

        let surface = bg_layer.layer.wl_surface();

        if width == logical_width || height == logical_height {
            debug!("Output '{}' needs no scaling", output_name);
            surface.set_buffer_scale(1);
            if let Some(old_viewport) = bg_layer.viewport.take() {
                old_viewport.destroy();
            };
        }
        else if width == logical_width * integer_scale_factor
            && height == logical_height * integer_scale_factor
        {
            debug!("Output '{}' needs integer scaling", output_name);
            surface.set_buffer_scale(integer_scale_factor);
            if let Some(old_viewport) = bg_layer.viewport.take() {
                old_viewport.destroy();
            };
        }
        else {
            debug!("Output '{}' needs fractional scaling", output_name);
            surface.set_buffer_scale(1);
            bg_layer.viewport
                .get_or_insert_with(||
                    self.viewporter.get_viewport(surface, qh, ())
                )
                .set_destination(logical_width, logical_height);
        }

        surface.commit();
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output)
        else {
            error!("Destroyed output has no output info, skipping");
            return;
        };

        let Some(output_name) = info.name
        else {
            error!("Destroyed output has no name, skipping");
            return;
        };

        debug!(
            "Output destroyed: {}",
            output_name,
        );

        if let Some(bg_layer_index) = self.background_layers.iter()
            .position(|bg_layers| bg_layers.output_name == output_name)
        {
            let removed_bg_layer = self.background_layers
                .swap_remove(bg_layer_index);

            // Workspaces on the destroyed output may have been moved anywhere
            // so reset the wallpaper on all the visible workspaces
            self.compositor_connection_task.request_visible_workspaces();

            debug!(
                "Dropping {} wallpapers on destroyed output for workspaces: {}",
                removed_bg_layer.workspace_backgrounds.len(),
                removed_bg_layer.workspace_backgrounds.iter()
                    .map(|workspace_bg| workspace_bg.workspace_name.as_str())
                    .collect::<Vec<_>>().join(", ")
            );

            drop(removed_bg_layer);
        }
        else {
            error!(
    "Ignoring destroyed output with unknown name '{}', known outputs were: {}",
                output_name,
                self.background_layers.iter()
                    .map(|bg_layer| bg_layer.output_name.as_str())
                    .collect::<Vec<_>>().join(", ")
            );
        }

        print_memory_stats(&self.background_layers);
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

impl Dispatch<WpViewporter, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WpViewporter,
        _event: <WpViewporter as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        unreachable!("wp_viewporter has no events");
    }
}

impl Dispatch<WpViewport, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WpViewport,
        _event: <WpViewport as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        unreachable!("wp_viewport has no events");
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &WlBuffer,
        _event: <WlBuffer as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        for bg_layer in state.background_layers.iter_mut() {
            for bg in bg_layer.workspace_backgrounds.iter_mut() {
                if bg.wallpaper.wl_buffer == *proxy {
                    let active_count = bg.wallpaper.active_count.get();
                    if let Some(new_count) = active_count.checked_sub(1) {
                        debug!("Compositor released the wl_shm wl_buffer \
                            of {:?}", bg.wallpaper.canon_path);
                        bg.wallpaper.active_count.set(new_count);
                    } else {
                        error!("Unexpected release event for the wl_shm \
                            wl_buffer of {:?}", bg.wallpaper.canon_path);
                    }
                    return
                }
            }
        }
        warn!("Release event for already destroyed wl_shm wl_buffer");
    }
}

pub struct BackgroundLayer {
    pub output_name: String,
    pub width: i32,
    pub height: i32,
    pub layer: LayerSurface,
    pub configured: bool,
    pub workspace_backgrounds: Vec<WorkspaceBackground>,
    pub current_workspace: Option<String>,
    pub transform: Transform,
    pub viewport: Option<WpViewport>,
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

        if self.current_workspace.as_deref() == Some(workspace_name) {
            debug!("Skipping draw on output {} for workspace {} because its \
                wallpaper is already set", self.output_name, workspace_name);
            return
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

        // Attach and commit to new workspace background
        self.layer.attach(Some(&workspace_bg.wallpaper.wl_buffer), 0, 0);
        workspace_bg.wallpaper.active_count.set(
            workspace_bg.wallpaper.active_count.get() + 1
        );

        // Damage the entire surface
        self.layer.wl_surface().damage_buffer(0, 0, self.width, self.height);

        self.layer.commit();

        self.current_workspace = Some(workspace_name.to_string());

        debug!(
            "Setting wallpaper on output '{}' for workspace: {}",
            self.output_name, workspace_name
        );
    }
}

pub struct WorkspaceBackground {
    pub workspace_name: String,
    pub wallpaper: Rc<Wallpaper>,
}

pub struct Wallpaper {
    pub wl_buffer: WlBuffer,
    pub active_count: Cell<usize>,
    pub shm_pool: RawPool,
    pub canon_path: PathBuf,
    pub canon_modified: u128,
}

impl Drop for Wallpaper {
    fn drop(&mut self) {
        if self.active_count.get() != 0 {
            warn!("Destroying a {} times active wl_buffer of wallpaper {:?}",
                self.active_count.get(), self.canon_path);
        }
        self.wl_buffer.destroy();
    }
}

fn layer_surface_name(output_name: &str) -> Option<String> {
    Some([env!("CARGO_PKG_NAME"), "_wallpaper_", output_name].concat())
}

fn find_equal_wallpaper(
    background_layers: &[BackgroundLayer],
    width: i32,
    height: i32,
    transform: Transform,
    wallpaper_file: &WallpaperFile
) -> Option<Rc<Wallpaper>> {
    for bg_layer in background_layers {
        if bg_layer.width == width
            && bg_layer.height == height
            && bg_layer.transform == transform
        {
            for bg in &bg_layer.workspace_backgrounds {
                if bg.wallpaper.canon_modified == wallpaper_file.canon_modified
                    && bg.wallpaper.canon_path == wallpaper_file.canon_path
                {
                    debug!("Reusing the wallpaper of output {} workspace {}",
                        bg_layer.output_name, bg.workspace_name);
                    return Some(Rc::clone(&bg.wallpaper));
                }
            }
        }
    }
    None
}

fn find_equal_output_wallpaper(
    workspace_backgrounds: &[WorkspaceBackground],
    wallpaper_file: &WallpaperFile
) -> Option<Rc<Wallpaper>> {
    for bg in workspace_backgrounds {
        if bg.wallpaper.canon_modified == wallpaper_file.canon_modified
            && bg.wallpaper.canon_path == wallpaper_file.canon_path
        {
            debug!("Reusing the wallpaper of workspace {}",
                bg.workspace_name);
            return Some(Rc::clone(&bg.wallpaper));
        }
    }
    None
}

fn print_memory_stats(background_layers: &[BackgroundLayer]) {
    if log::log_enabled!(log::Level::Debug) {
        let mut wl_shm_count = 0.0f32;
        let mut wl_shm_size = 0.0f32;
        for bg_layer in background_layers {
            for bg in &bg_layer.workspace_backgrounds {
                let factor = 1.0 / Rc::strong_count(&bg.wallpaper) as f32;
                wl_shm_count += factor;
                wl_shm_size += factor * bg.wallpaper.shm_pool.len() as f32;
            }
        }
        let count = (wl_shm_count + 0.5) as usize;
        let size_kb = (wl_shm_size + 0.5) as usize / 1024;
        debug!("Memory use: {size_kb} KiB from {count} wl_shm pools");
    }
}
