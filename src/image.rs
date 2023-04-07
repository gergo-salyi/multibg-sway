use std::{
    fs::read_dir,
    path::Path
};

use log::error;
use image::{ImageBuffer, Rgb, ImageError};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::reexports::client::protocol::wl_shm;

use crate::wayland::WorkspaceBackground;

pub fn workspace_bgs_from_output_image_dir(
    dir_path: impl AsRef<Path>, 
    slot_pool: &mut SlotPool,
    format: wl_shm::Format,
)
    -> Result<Vec<WorkspaceBackground>, String>
{
    let mut buffers = Vec::new();

    let dir = read_dir(&dir_path)
        .map_err(|e| format!("Failed to open directory: {}", e))?;

    for entry_result in dir {

        let entry = match entry_result {
            Ok(entry) => entry,
            Err(e) => {
                error!(
                    "Skipping a directory entry in '{:?}' due to an error: {}",
                    dir_path.as_ref(), e
                );
                continue;
            }
        };
        
        // Resolve symlinks
        let path = match entry.path().canonicalize() {
            Ok(file_type) => file_type,
            Err(e) => {
                error!(
                    "Skipping '{:?}' in '{:?}' due to an error: {}",
                    entry.path(), dir_path.as_ref(), e
                );
                continue;
            }
        };

        // Skip dirs
        if path.is_dir() { continue }

        // Use the file stem as the name of the workspace for this wallpaper
        let workspace_name = path.file_stem().unwrap()
            .to_string_lossy().into_owned();

        let raw_image = match image::io::Reader::open(&path)
            .map_err(ImageError::IoError)
            .and_then(|r| r.with_guessed_format()
                .map_err(ImageError::IoError)
            )
            .and_then(|r| r.decode())
        {
            Ok(raw_image) => raw_image,
            Err(e) => {
                error!(
                    "Failed to open image '{:?}': {}",
                    path, e
                );
                continue;
            }
        };

        let image = raw_image
            // It is possible to adjust the contrast and brightness here
            // .adjust_contrast(-25.0)
            // .brighten(-60)
            .into_rgb8();

        let buffer = match format {
            wl_shm::Format::Xrgb8888 => 
                buffer_xrgb8888_from_image(image, slot_pool),
            wl_shm::Format::Bgr888 => 
                buffer_bgr888_from_image(image, slot_pool),
            _ => unreachable!()
        };

        buffers.push(WorkspaceBackground { workspace_name, buffer });
    }

    if buffers.is_empty() {
        Err("Found 0 suitable images in the directory".to_string())
    }
    else {
        Ok(buffers)
    }
}

fn buffer_xrgb8888_from_image(
    image: ImageBuffer<Rgb<u8>, Vec<u8>>,
    slot_pool: &mut SlotPool,
) 
    -> Buffer 
{
    let (buffer, canvas) = slot_pool
        .create_buffer(
            image.width() as i32, 
            image.height() as i32, 
            image.width() as i32 * 4, 
            wl_shm::Format::Xrgb8888
        )
        .unwrap();

    let canvas_len = image.len() / 3 * 4;

    let image_pixels = image.pixels();
    let canvas_pixels = canvas[..canvas_len].chunks_exact_mut(4);

    for (image_pixel, canvas_pixel) in image_pixels.zip(canvas_pixels) {
        canvas_pixel[0] = image_pixel.0[2];
        canvas_pixel[1] = image_pixel.0[1];
        canvas_pixel[2] = image_pixel.0[0];
    }

    buffer
}

fn buffer_bgr888_from_image(
    image: ImageBuffer<Rgb<u8>, Vec<u8>>,
    slot_pool: &mut SlotPool,
) 
    -> Buffer 
{
    let (buffer, canvas) = slot_pool
        .create_buffer(
            image.width() as i32, 
            image.height() as i32, 
            image.width() as i32 * 3,
            wl_shm::Format::Bgr888
        )
        .unwrap();

    canvas[..image.len()].copy_from_slice(&image);

    buffer
}

