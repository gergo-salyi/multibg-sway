#![allow(clippy::too_many_arguments)]

use std::{
    fs::{DirEntry, read_dir},
    io,
    path::Path,
};

use anyhow::{bail, Context};
use fast_image_resize::{
    FilterType, PixelType, Resizer, ResizeAlg, ResizeOptions,
    images::Image,
};
use image::{ColorType, DynamicImage, ImageBuffer, ImageDecoder, ImageReader};
use log::{debug, error, warn};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::reexports::client::protocol::wl_shm;

use crate::wayland::WorkspaceBackground;

pub fn workspace_bgs_from_output_image_dir(
    dir_path: impl AsRef<Path>,
    slot_pool: &mut SlotPool,
    format: wl_shm::Format,
    brightness: i32,
    contrast: f32,
    width: u32,
    height: u32,
) -> anyhow::Result<Vec<WorkspaceBackground>> {
    let mut buffers = Vec::new();
    let mut resizer = Resizer::new();
    let stride = match format {
        wl_shm::Format::Xrgb8888 => width as usize * 4,
        wl_shm::Format::Bgr888 => {
            // Align buffer stride to both 4 and pixel format block size
            // Not being aligned to 4 caused
            // https://github.com/gergo-salyi/multibg-sway/issues/6
            (width as usize * 3).next_multiple_of(4)
        },
        _ => unreachable!()
    };
    let dir = read_dir(&dir_path).context("Failed to open directory")?;
    for entry_result in dir {
        match workspace_bg_from_file(
            entry_result,
            slot_pool,
            format,
            brightness,
            contrast,
            width,
            height,
            stride,
            &mut resizer
        ) {
            Ok(Some(workspace_bg)) => buffers.push(workspace_bg),
            Ok(None) => continue,
            Err(e) => {
                error!("Skipping a directory entry in {:?} \
                    due to an error: {:#}", dir_path.as_ref(), e);
                continue;
            }
        }
    }
    if buffers.is_empty() {
        bail!("Found no suitable images in the directory")
    }
    Ok(buffers)
}

fn workspace_bg_from_file(
    dir_entry_result: io::Result<DirEntry>,
    slot_pool: &mut SlotPool,
    format: wl_shm::Format,
    brightness: i32,
    contrast: f32,
    width: u32,
    height: u32,
    stride: usize,
    resizer: &mut Resizer,
) -> anyhow::Result<Option<WorkspaceBackground>> {
    let entry = dir_entry_result.context("Failed to read direectory")?;
    let path = entry.path();
    // Skip dirs
    if path.is_dir() { return Ok(None) }
    // Use the file stem as the name of the workspace for this wallpaper
    let workspace_name = path.file_stem().unwrap()
        .to_string_lossy().into_owned();
    let (buffer, canvas) = slot_pool.create_buffer(
        width.try_into().unwrap(),
        height.try_into().unwrap(),
        stride.try_into().unwrap(),
        format,
    ).context("Failed to create Wayland shared memory buffer")?;
    let color_transform = if brightness == 0 && contrast == 0.0 {
        ColorTransform::None
    } else {
        ColorTransform::Legacy { brightness, contrast }
    };
    load_wallpaper(
        &path,
        &mut canvas[..stride * height as usize],
        width,
        height,
        stride,
        format,
        color_transform,
        resizer
    ).context("Failed to load wallpaper")?;
    Ok(Some(WorkspaceBackground { workspace_name, buffer }))
}

#[derive(Clone, Copy, PartialEq)]
pub enum ColorTransform {
    // Levels { input_max: u8, input_min: u8, output_max: u8, output_min: u8 },
    Legacy { brightness: i32, contrast: f32 },
    None,
}

fn load_wallpaper(
    path: &Path,
    dst: &mut [u8],
    surface_width: u32,
    surface_height: u32,
    surface_stride: usize,
    surface_format: wl_shm::Format,
    color_transform: ColorTransform,
    resizer: &mut Resizer,
) -> anyhow::Result<()> {
    let reader = ImageReader::open(path)
        .context("Failed to open image file")?
        .with_guessed_format()
        .context("Failed to read image file format")?;
    let file_format = reader.format()
        .context("Failed to determine image file format")?;
    if !file_format.can_read() {
        bail!("Unsupported image file format {file_format:?}")
    } else if !file_format.reading_enabled() {
        bail!("Application was compiled with support \
            for image file format {file_format:?} disabled")
    }
    let mut decoder = reader.into_decoder()
        .context("Failed to initialize image decoder")?;
    let (image_width, image_height) = decoder.dimensions();
    let image_size = decoder.total_bytes();
    let image_color_type = decoder.color_type();
    if image_width == 0 || image_height == 0 || image_size > isize::MAX as u64 {
        bail!("Image has invalid dimensions {image_width}x{image_height}")
    };
    let image_size = image_size as usize;
    debug!("Image {image_width}x{image_height} {image_color_type:?}");
    if image_color_type.has_alpha() {
        warn!("Image has alpha channel which will be ignored");
    }
    if let Ok(Some(_)) = decoder.icc_profile() {
        debug!("Image has an embedded ICC color profile \
            but ICC color profile handling is not yet implemented");
    }
    let needs_resize = image_width != surface_width
        || image_height != surface_height;
    let surface_row_len = surface_width as usize * 3;
    if !needs_resize
        && image_color_type == ColorType::Rgb8
        && surface_format == wl_shm::Format::Bgr888
        && color_transform == ColorTransform::None
        && surface_row_len == surface_stride
    {
        debug!("Decoding image directly to destination buffer");
        decoder.read_image(&mut dst[..image_size])
            .context("Failed to decode image")?;
        return Ok(());
    }
    let mut image = DynamicImage::from_decoder(decoder)
        .context("Failed to decode image")?;
    if let ColorTransform::Legacy { brightness, contrast } = color_transform {
        if contrast != 0.0 {
            image = image.adjust_contrast(contrast)
        }
        if brightness != 0 {
            image = image.brighten(brightness)
        }
    }
    let mut image = image.into_rgb8();
    if needs_resize {
        debug!("Resizing image from {}x{} to {}x{}",
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
        resizer.resize(
            &src_image,
            &mut dst_image,
            &ResizeOptions::new()
                .fit_into_destination(None)
                .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
        ).context("Failed to resize image")?;
        image = ImageBuffer::from_raw(
            surface_width,
            surface_height,
            dst_image.into_vec()
        ).unwrap();
    }
    match surface_format {
        wl_shm::Format::Bgr888 => {
            if surface_row_len == surface_stride {
                dst.copy_from_slice(&image);
            } else {
                copy_pad_stride(
                    &image,
                    dst,
                    surface_row_len,
                    surface_stride,
                    surface_height as usize,
                );
            }
        },
        wl_shm::Format::Xrgb8888 => {
            swizzle_bgra_from_rgb(&image, dst);
        },
        _ => unreachable!(),
    }
    Ok(())
}

fn copy_pad_stride(
    src: &[u8],
    dst: &mut [u8],
    src_stride: usize,
    dst_stride: usize,
    height: usize,
) {
    for row in 0..height {
        dst[row * dst_stride..][..src_stride].copy_from_slice(
            &src[row * src_stride..][..src_stride]
        );
    }
}

fn swizzle_bgra_from_rgb(src: &[u8], dst: &mut [u8]) {
    let pixel_count = dst.len() / 4;
    assert_eq!(src.len(), pixel_count * 3);
    assert_eq!(dst.len(), pixel_count * 4);
    unsafe {
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            bgra_from_rgb_avx2(src, dst, pixel_count);
            return
        }
        bgra_from_rgb(src, dst, pixel_count)
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn bgra_from_rgb_avx2(src: &[u8], dst: &mut [u8], pixel_count: usize) {
    unsafe { bgra_from_rgb(src, dst, pixel_count) }
}

unsafe fn bgra_from_rgb(src: &[u8], dst: &mut [u8], pixel_count: usize) {
    unsafe {
        let mut src = src.as_ptr();
        let mut dst = dst.as_mut_ptr();
        for _ in 0..pixel_count {
            *dst.add(0) = *src.add(2); // B
            *dst.add(1) = *src.add(1); // G
            *dst.add(2) = *src.add(0); // R
            *dst.add(3) = u8::MAX;     // A
            src = src.add(3);
            dst = dst.add(4);
        }
    }
}
