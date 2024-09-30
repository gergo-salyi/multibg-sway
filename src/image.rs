use std::{
    fs::read_dir,
    path::Path,
};

use fast_image_resize::{
    FilterType, PixelType, Resizer, ResizeAlg, ResizeOptions,
    images::Image,
};
use image::{ImageBuffer, ImageError, ImageReader, Rgb};
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
    surface_width: u32,
    surface_height: u32,
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

        let raw_image = match ImageReader::open(&path)
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
        let image_width = image.width();
        let image_height = image.height();
        
        if image_width == 0 {
            error!(
                "Image '{}' has zero width, skipping", workspace_name
            );
            continue;
        };
        if image_height == 0 {
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

            let src_image = Image::from_vec_u8(
                image_width,
                image_height,
                image.into_raw(),
                PixelType::U8x3,
            ).unwrap();

            let mut dst_image = Image::new(
                surface_width,
                surface_height,
                PixelType::U8x3,
            );

            let mut resizer = Resizer::new();
            resizer.resize(
                &src_image,
                &mut dst_image,
                &ResizeOptions::new()
                    .fit_into_destination(None)
                    .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
            ).unwrap();

            image = ImageBuffer::from_raw(
                surface_width, 
                surface_height, 
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
    // Align buffer stride to both 4 and pixel format block size
    // Not being aligned to 4 caused
    // https://github.com/gergo-salyi/multibg-sway/issues/6
    const BUFFER_STRIDE_ALIGNEMENT: u32 = 4 * 3;

    let width = image.width();
    let height = image.height();
    let image_stride = width * 3;

    let unaligned_bytes = image_stride % BUFFER_STRIDE_ALIGNEMENT;

    let buffer_stride =
    if unaligned_bytes == 0 {
        image_stride
    } else {
        let padding = BUFFER_STRIDE_ALIGNEMENT - unaligned_bytes;
        image_stride + padding
    };

    let (buffer, canvas) = slot_pool
        .create_buffer(
            width.try_into().unwrap(),
            height.try_into().unwrap(),
            buffer_stride.try_into().unwrap(),
            wl_shm::Format::Bgr888
        )
        .unwrap();

    if unaligned_bytes == 0 {
        canvas[..image.len()].copy_from_slice(&image);
    }
    else {
        let height: usize = height.try_into().unwrap();
        let buffer_stride: usize = buffer_stride.try_into().unwrap();
        let image_stride: usize = image_stride.try_into().unwrap();

        for row in 0..height {
            let canvas_start = row * buffer_stride;
            let image_start = row * image_stride;
            let len = image_stride;

            canvas[canvas_start..(canvas_start + len)].copy_from_slice(
                &image.as_raw()[image_start..(image_start + len)]
            );
        }
    }

    buffer
}
