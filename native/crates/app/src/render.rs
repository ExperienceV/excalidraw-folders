//! Turns the core's display list into vello draw calls, on screen or into
//! a PNG.

use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use pizarra_core::kurbo::{Affine, Rect, Shape};
use pizarra_core::{Color, Frame, Style};
use serde_json::Value;
use vello::kurbo::Stroke;
use vello::peniko::{self, Blob, Fill, ImageAlphaType, ImageBrushRef, ImageData, ImageFormat, Mix};
use vello::util::RenderContext;
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

pub fn to_peniko(c: Color) -> peniko::Color {
    peniko::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Decoded bitmaps of a document's image files, by file id. Files that
/// cannot be decoded are remembered too, so they are not retried each frame.
#[derive(Default)]
pub struct Images {
    decoded: HashMap<String, Option<ImageData>>,
}

impl Images {
    /// The bitmap for `file_id`, decoded on first use from the document's
    /// `files` map.
    fn get(&mut self, file_id: &str, files: &Value) -> Option<&ImageData> {
        self.decoded
            .entry(file_id.to_owned())
            .or_insert_with(|| decode_data_url(files[file_id]["dataURL"].as_str()?))
            .as_ref()
    }
}

/// Decodes a `data:<mime>;base64,...` URL holding a PNG, JPEG, GIF, WebP
/// or BMP file.
fn decode_data_url(data_url: &str) -> Option<ImageData> {
    let (_, encoded) = data_url.split_once(";base64,")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    let pixels = image::load_from_memory(&bytes).ok()?.into_rgba8();
    let (width, height) = pixels.dimensions();
    Some(ImageData {
        data: Blob::new(Arc::new(pixels.into_raw())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    })
}

/// Encodes the core's display list into a vello scene. `scale` is the
/// window's HiDPI factor; the core works in logical pixels. `files` is the
/// document's file map, where image ops find their bitmaps.
pub fn encode(scene: &mut Scene, frame: &Frame, scale: f64, viewport: Rect, images: &mut Images, files: &Value) {
    scene.reset();
    let base = Affine::scale(scale);
    for item in &frame.items {
        let transform = base * item.transform;
        let translucent = item.opacity < 1.0;
        if translucent {
            // Group opacity: overlapping strokes of one element must not add up.
            scene.push_layer(Fill::NonZero, Mix::Normal, item.opacity, Affine::IDENTITY, &viewport);
        }
        for op in item.ops.iter() {
            let color = to_peniko(op.color);
            match &op.style {
                Style::Fill => scene.fill(Fill::NonZero, transform, color, None, &op.path),
                Style::Stroke { width, dash } => {
                    let mut stroke = Stroke::new(*width);
                    if !dash.is_empty() {
                        stroke = stroke.with_dashes(0.0, dash);
                    }
                    scene.stroke(&stroke, transform, color, None, &op.path);
                }
                Style::Image { file_id, crop, flip_x, flip_y } => {
                    let Some(image) = images.get(file_id, files) else {
                        scene.fill(Fill::NonZero, transform, color, None, &op.path);
                        continue;
                    };
                    let target = op.path.bounding_box();
                    let source = match crop {
                        Some(c) => Rect::new(c.x, c.y, c.x + c.width, c.y + c.height),
                        None => Rect::new(0.0, 0.0, f64::from(image.width), f64::from(image.height)),
                    };
                    // Source pixels → the element's box, mirrored if flipped.
                    let mirror = |flipped: bool, extent: f64| if flipped { (-1.0, extent) } else { (1.0, 0.0) };
                    let ((mx, tx), (my, ty)) = (mirror(*flip_x, target.width()), mirror(*flip_y, target.height()));
                    let fit = Affine::translate(target.origin().to_vec2())
                        * Affine::new([mx, 0.0, 0.0, my, tx, ty])
                        * Affine::scale_non_uniform(target.width() / source.width(), target.height() / source.height())
                        * Affine::translate(-source.origin().to_vec2());
                    let brush: ImageBrushRef<'_> = image.into();
                    scene.fill(Fill::NonZero, transform * fit, brush, None, &source);
                }
            }
        }
        if translucent {
            scene.pop_layer();
        }
    }
}

pub fn renderer_options() -> RendererOptions {
    RendererOptions { antialiasing_support: AaSupport::area_only(), ..Default::default() }
}

/// Renders `frame` off-screen with the same GPU pipeline as the window and
/// writes it as a PNG of `width`×`height` pixels. `files` is the document's
/// file map.
pub fn write_png(frame: &Frame, width: u32, height: u32, files: &Value, out: &Path) -> Result<(), Box<dyn Error>> {
    let mut context = RenderContext::new();
    let dev_id = pollster::block_on(context.device(None)).ok_or("no compatible GPU found")?;
    let (device, queue) = (&context.devices[dev_id].device, &context.devices[dev_id].queue);
    let limit = device.limits().max_texture_dimension_2d;
    if width > limit || height > limit {
        return Err(format!("image of {width}×{height} px exceeds the GPU limit of {limit} px").into());
    }

    let mut scene = Scene::new();
    let viewport = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    encode(&mut scene, frame, 1.0, viewport, &mut Images::default(), files);
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let params = RenderParams {
        base_color: to_peniko(frame.background),
        width,
        height,
        antialiasing_method: AaConfig::Area,
    };
    Renderer::new(device, renderer_options())?.render_to_texture(
        device,
        queue,
        &scene,
        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
        &params,
    )?;

    // Buffer rows must be padded to the GPU's copy alignment.
    let row = width * 4;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
        },
        size,
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("failed to read back the image"));
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let data = buffer.get_mapped_range(..)?;

    let mut png = png::Encoder::new(BufWriter::new(File::create(out)?), width, height);
    png.set_color(png::ColorType::Rgba);
    let mut writer = png.write_header()?.into_stream_writer()?;
    for line in data.chunks(padded as usize) {
        std::io::Write::write_all(&mut writer, &line[..row as usize])?;
    }
    writer.finish()?;
    Ok(())
}
