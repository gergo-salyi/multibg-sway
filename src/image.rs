use std::{
    fs::read_dir,
    num::NonZeroU32,
    path::Path
};

use fast_image_resize as fr;
use image::{ImageBuffer, Rgb, ImageError};
use log::{debug, error};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::reexports::client::protocol::wl_shm;

use crate::wayland::WorkspaceBackground;

pub fn workspace_bgs_from_output_image_dir(
    dir_path: impl AsRef<Path>, 
    slot_pool: &mut SlotPool,
    format: wl_shm::Format,
    brightness: i32,
    contrast: f32,
    surface_width: NonZeroU32,
    surface_height: NonZeroU32,
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

        let path = entry.path();

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

        // It is possible to adjust the contrast and brightness here
        let mut image = raw_image;
        if contrast != 0.0 {
            image = image.adjust_contrast(contrast)
        }
        if brightness != 0 {
            image = image.brighten(brightness)
        }

        let mut image = image.into_rgb8();
        
        let Some(image_width) = NonZeroU32::new(image.width()) else {
            error!(
                "Image '{}' has zero width, skipping", workspace_name
            );
            continue;
        };
        let Some(image_height) = NonZeroU32::new(image.height()) else {
            error!(
                "Image '{}' has zero height, skipping", workspace_name
            );
            continue;
        };

        if image_width != surface_width || image_height != surface_height
        {
            debug!("Resizing image '{}' from {}x{} to {}x{}",
                workspace_name, 
                image_width, image_height,
                surface_width, surface_height
            );

            let src_image = fr::Image::from_vec_u8(
                image_width,
                image_height,
                image.into_raw(),
                fr::PixelType::U8x3,
            ).unwrap();

            let dst_image = resize_image_with_cropping(
                src_image.view(),
                surface_width,
                surface_height,
            );

            image = ImageBuffer::from_raw(
                surface_width.get(), 
                surface_height.get(), 
                dst_image.into_vec()
            ).unwrap();
        }

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

// Copied example from fast_image_resize
fn resize_image_with_cropping(
    mut src_view: fr::DynamicImageView,
    dst_width: NonZeroU32,
    dst_height: NonZeroU32
) -> fr::Image {
    // Set cropping parameters
    src_view.set_crop_box_to_fit_dst_size(dst_width, dst_height, None);

    // Create container for data of destination image
    let mut dst_image = fr::Image::new(
        dst_width,
        dst_height,
        src_view.pixel_type(),
    );
    // Get mutable view of destination image data
    let mut dst_view = dst_image.view_mut();

    // Create Resizer instance and resize source image
    // into buffer of destination image
    let mut resizer = fr::Resizer::new(
        fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3)
    );
    resizer.resize(&src_view, &mut dst_view).unwrap();

    dst_image
}
