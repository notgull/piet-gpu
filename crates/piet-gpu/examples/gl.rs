// SPDX-License-Identifier: LGPL-3.0-or-later OR MPL-2.0
// This file is a part of `piet-gpu`.
//
// `piet-gpu` is free software: you can redistribute it and/or modify it under the terms of
// either:
//
// * GNU Lesser General Public License as published by the Free Software Foundation, either
// version 3 of the License, or (at your option) any later version.
// * Mozilla Public License as published by the Mozilla Foundation, version 2.
//
// `piet-gpu` is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
// without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See the GNU Lesser General Public License or the Mozilla Public License for more details.
//
// You should have received a copy of the GNU Lesser General Public License and the Mozilla
// Public License along with `piet-gpu`. If not, see <https://www.gnu.org/licenses/> or
// <https://www.mozilla.org/en-US/MPL/2.0/>.

//! An example that uses the `gl` crate to render to a `winit` window.
//!
//! This uses `glutin` crate to set up a GL context, `winit` to create a window, and the `gl`
//! crate to make GL calls.
//!
//! This example exists mostly to give an example of how a `GpuContext` can be implemented.
//! In order to

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;

use glutin_winit::{DisplayBuilder, GlWindow};

use piet::kurbo::{Affine, BezPath, Point, Rect, Vec2};
use piet::{GradientStop, RenderContext as _};
use piet_gpu::BufferType;

use raw_window_handle::HasRawWindowHandle;

use winit::dpi::PhysicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowBuilder;

use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::fmt;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create the winit event loop.
    let event_loop = EventLoop::new();

    let mut size = PhysicalSize::new(600, 400);
    let make_window_builder = move || {
        WindowBuilder::new()
            .with_title("piet-gpu example")
            .with_transparent(true)
            .with_inner_size(size)
    };

    // If we're on Windows, start with the window.
    let window = if cfg!(windows) {
        Some(make_window_builder())
    } else {
        None
    };

    // Start building an OpenGL display.
    let display = DisplayBuilder::new().with_window_builder(window);

    // Look for a config that supports transparency and has a good sample count.
    let (mut window, gl_config) = display.build(
        &event_loop,
        ConfigTemplateBuilder::new().with_alpha_size(8),
        |configs| {
            configs
                .reduce(|accum, config| {
                    let transparency_check = config.supports_transparency().unwrap_or(false)
                        & !accum.supports_transparency().unwrap_or(false);

                    if transparency_check || config.num_samples() > accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        },
    )?;

    // Try to build a several different contexts.
    let window_handle = window.as_ref().map(|w| w.raw_window_handle());
    let contexts = [
        ContextAttributesBuilder::new().build(window_handle),
        ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(None))
            .build(window_handle),
        ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(window_handle),
    ];

    let display = gl_config.display();
    let gl_handler = (|| {
        // Try to build a context for each config.
        for context in &contexts {
            if let Ok(gl_context) = unsafe { display.create_context(&gl_config, context) } {
                return Ok(gl_context);
            }
        }

        // If we couldn't build a context, return an error.
        Err(Box::<dyn std::error::Error>::from(
            "Could not create a context",
        ))
    })()?;

    // Set up data for the window.
    let framerate = Duration::from_millis({
        let framerate = 1.0 / 60.0;
        (framerate * 1000.0) as u64
    });
    let mut next_frame = Instant::now() + framerate;
    let mut state = None;
    let mut renderer = None;
    let mut not_current_gl_context = Some(gl_handler);

    // Drawing data.
    let star = {
        let mut path = BezPath::new();
        path.move_to((300.0, 300.0));
        path.line_to((400.0, 400.0));
        path.line_to((300.0, 400.0));
        path.close_path();
        path
    };
    let mut solid_red = None;
    let mut outline = None;

    // Draw the window.
    let mut draw = move |ctx: &mut piet_gpu::RenderContext<'_, GlContext>| {
        ctx.clear(None, piet::Color::AQUA);

        let solid_red = solid_red.get_or_insert_with(|| ctx.solid_brush(piet::Color::RED));
        let outline = outline.get_or_insert_with(|| ctx.solid_brush(piet::Color::BLACK));

        ctx.fill(Rect::new(0.0, 0.0, 400.0, 400.0), solid_red);
        //ctx.stroke(&star, outline, 5.0);

        ctx.finish().unwrap();
        ctx.status()
    };

    event_loop.run(move |event, target, control_flow| {
        control_flow.set_wait_until(next_frame);

        match event {
            Event::Resumed => {
                // We can now create windows.
                let window = window.take().unwrap_or_else(|| {
                    let window_builder = make_window_builder();
                    glutin_winit::finalize_window(target, window_builder, &gl_config).unwrap()
                });

                let attrs = window.build_surface_attributes(Default::default());
                let surface = unsafe {
                    gl_config
                        .display()
                        .create_window_surface(&gl_config, &attrs)
                        .unwrap()
                };

                // Make the context current.
                let gl_context = not_current_gl_context
                    .take()
                    .unwrap()
                    .make_current(&surface)
                    .unwrap();

                unsafe {
                    renderer
                        .get_or_insert_with(|| {
                            // Register the GL pointers if we can.
                            {
                                gl::load_with(|symbol| {
                                    let symbol_cstr = CString::new(symbol).unwrap();
                                    gl_config.display().get_proc_address(symbol_cstr.as_c_str())
                                });

                                piet_gpu::Source::new(GlContext::new()).unwrap()
                            }
                        })
                        .context()
                        .set_context();
                }

                state = Some((surface, window, gl_context));
            }

            Event::Suspended => {
                // Destroy the window.
                if let Some((.., context)) = state.take() {
                    not_current_gl_context = Some(context.make_not_current().unwrap());
                }

                if let Some(renderer) = &renderer {
                    renderer.context().unset_context();
                }
            }

            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => control_flow.set_exit(),
                WindowEvent::Resized(new_size) => {
                    size = new_size;

                    if let Some((surface, _, context)) = &state {
                        surface.resize(
                            context,
                            NonZeroU32::new(size.width).unwrap(),
                            NonZeroU32::new(size.height).unwrap(),
                        );
                    }
                }
                _ => {}
            },

            Event::RedrawEventsCleared => {
                if let (Some((surface, _, context)), Some(renderer)) = (&state, &mut renderer) {
                    // Create the render context.
                    let mut render_context = renderer.render_context(size.width, size.height);

                    // Perform drawing.
                    draw(&mut render_context).unwrap();

                    // Swap buffers.
                    surface.swap_buffers(context).unwrap();
                }

                // Schedule the next frame.
                next_frame += framerate;
            }

            _ => {}
        }
    })
}

fn generate_five_pointed_star(center: Point, inner_radius: f64, outer_radius: f64) -> BezPath {
    let point_from_polar = |radius: f64, angle: f64| {
        let x = center.x + radius * angle.cos();
        let y = center.y + radius * angle.sin();
        Point::new(x, y)
    };

    let one_fifth_circle = std::f64::consts::PI * 2.0 / 5.0;

    let outer_points = (0..5).map(|i| point_from_polar(outer_radius, one_fifth_circle * i as f64));
    let inner_points = (0..5).map(|i| {
        point_from_polar(
            inner_radius,
            one_fifth_circle * i as f64 + one_fifth_circle / 2.0,
        )
    });
    let mut points = outer_points.zip(inner_points).flat_map(|(a, b)| [a, b]);

    // Set up the path.
    let mut path = BezPath::new();
    path.move_to(points.next().unwrap());

    // Add the points to the path.
    for point in points {
        path.line_to(point);
    }

    // Close the path.
    path.close_path();
    path
}

/// The global OpenGL context.
struct GlContext {
    /// Whether we have a context installed.
    has_context: Cell<bool>,

    /// A program for rendering.
    render_program: gl::types::GLuint,

    // Uniform locations.
    u_transform: gl::types::GLint,
    viewport_size: gl::types::GLint,
    tex: gl::types::GLint,
    mask: gl::types::GLint,
}

impl GlContext {
    fn assert_context(&self) {
        if !self.has_context.get() {
            panic!("No GL context installed");
        }
    }

    // SAFETY: Context must be current.
    unsafe fn new() -> Self {
        // Create the program.
        let program = unsafe {
            let vertex_shader = Self::compile_shader(gl::VERTEX_SHADER, VERTEX_SHADER).unwrap();

            let fragment_shader =
                Self::compile_shader(gl::FRAGMENT_SHADER, FRAGMENT_SHADER).unwrap();

            let program = gl::CreateProgram();
            gl::AttachShader(program, vertex_shader);
            gl::AttachShader(program, fragment_shader);
            gl::LinkProgram(program);

            let mut success = gl::FALSE as gl::types::GLint;
            gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);

            if success == gl::FALSE as gl::types::GLint {
                let mut len = 0;
                gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut len);

                let mut buf = Vec::with_capacity(len as usize);
                gl::GetProgramInfoLog(program, len, std::ptr::null_mut(), buf.as_mut_ptr() as _);
                buf.set_len((len as usize) - 1);
                panic!(
                    "Could not link program: {}",
                    std::str::from_utf8(&buf).unwrap()
                );
            }

            gl::DetachShader(program, vertex_shader);
            gl::DetachShader(program, fragment_shader);
            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);

            program
        };

        unsafe {
            extern "system" fn debug_callback(
                source: u32,
                ty: u32,
                id: u32,
                severity: u32,
                msg_len: i32,
                msg: *const i8,
                _user_param: *mut std::ffi::c_void,
            ) {
                let source = match source {
                    gl::DEBUG_SOURCE_API => "API",
                    gl::DEBUG_SOURCE_WINDOW_SYSTEM => "Window System",
                    gl::DEBUG_SOURCE_SHADER_COMPILER => "Shader Compiler",
                    gl::DEBUG_SOURCE_THIRD_PARTY => "Third Party",
                    gl::DEBUG_SOURCE_APPLICATION => "Application",
                    gl::DEBUG_SOURCE_OTHER => "Other",
                    _ => "Unknown",
                };

                let ty = match ty {
                    gl::DEBUG_TYPE_ERROR => "Error",
                    gl::DEBUG_TYPE_DEPRECATED_BEHAVIOR => "Deprecated Behavior",
                    gl::DEBUG_TYPE_UNDEFINED_BEHAVIOR => "Undefined Behavior",
                    gl::DEBUG_TYPE_PORTABILITY => "Portability",
                    gl::DEBUG_TYPE_PERFORMANCE => "Performance",
                    gl::DEBUG_TYPE_MARKER => "Marker",
                    gl::DEBUG_TYPE_OTHER => "Other",
                    _ => "Unknown",
                };

                let message = {
                    let slice =
                        unsafe { std::slice::from_raw_parts(msg as *const u8, msg_len as usize) };
                    std::str::from_utf8(slice).unwrap()
                };

                match severity {
                    gl::DEBUG_SEVERITY_HIGH => {
                        log::error!("{ty}-{id} ({source}): {message}");
                    }
                    gl::DEBUG_SEVERITY_MEDIUM => {
                        log::warn!("{ty}-{id} ({source}): {message}");
                    }
                    gl::DEBUG_SEVERITY_LOW => {
                        log::info!("{ty}-{id} ({source}): {message}");
                    }
                    gl::DEBUG_SEVERITY_NOTIFICATION => {
                        log::debug!("{ty}-{id} ({source}): {message}");
                    }
                    _ => (),
                };
            }

            // Set up a debug callback.
            gl::Enable(gl::DEBUG_OUTPUT);

            gl::DebugMessageCallback(Some(debug_callback), std::ptr::null());
        }

        // Get the uniform locations.
        let u_transform = unsafe {
            let name = CString::new("u_transform").unwrap();
            gl::GetUniformLocation(program, name.as_ptr())
        };

        let viewport_size = unsafe {
            let name = CString::new("viewport_size").unwrap();
            gl::GetUniformLocation(program, name.as_ptr())
        };

        let tex = unsafe {
            let name = CString::new("tex").unwrap();
            gl::GetUniformLocation(program, name.as_ptr())
        };

        let mask = unsafe {
            let name = CString::new("mask").unwrap();
            gl::GetUniformLocation(program, name.as_ptr())
        };

        Self {
            has_context: Cell::new(true),
            render_program: program,
            u_transform,
            viewport_size,
            tex,
            mask,
        }
    }

    fn unset_context(&self) {
        self.has_context.set(false);
    }

    unsafe fn set_context(&self) {
        self.has_context.set(true);
    }

    unsafe fn compile_shader(
        shader_type: gl::types::GLenum,
        source: &str,
    ) -> Result<gl::types::GLuint, GlError> {
        let shader = gl::CreateShader(shader_type);
        let source = CString::new(source).unwrap();
        gl::ShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);

        let mut success = gl::FALSE as gl::types::GLint;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);

        if success == gl::FALSE as gl::types::GLint {
            let mut len = 0;
            gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);

            let mut buf = Vec::with_capacity(len as usize);
            gl::GetShaderInfoLog(
                shader,
                len,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut gl::types::GLchar,
            );
            buf.set_len((len as usize) - 1);

            return Err(GlError(format!(
                "Shader compilation failed: {}",
                std::str::from_utf8(&buf).unwrap()
            )));
        }

        Ok(shader)
    }
}

#[derive(Debug)]
struct GlError(String);

impl fmt::Display for GlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GL error: {}", self.0)
    }
}

impl std::error::Error for GlError {}

impl piet_gpu::GpuContext for GlContext {
    type Buffer = gl::types::GLuint;
    type Error = GlError;
    type Texture = gl::types::GLuint;
    type VertexArray = gl::types::GLuint;

    fn clear(&self, color: piet::Color) {
        self.assert_context();
        let (r, g, b, a) = color.as_rgba();

        unsafe {
            gl::ClearColor(r as f32, g as f32, b as f32, a as f32);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }
    }

    fn flush(&self) -> Result<(), Self::Error> {
        self.assert_context();

        unsafe {
            gl::Flush();
            Ok(())
        }
    }

    fn create_texture(
        &self,
        interpolation: piet::InterpolationMode,
        repeat: piet_gpu::RepeatStrategy,
    ) -> Result<Self::Texture, Self::Error> {
        unsafe {
            let mut texture = 0;
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);

            let (min_filter, mag_filter) = match interpolation {
                piet::InterpolationMode::NearestNeighbor => (gl::NEAREST, gl::NEAREST),
                piet::InterpolationMode::Bilinear => (gl::LINEAR, gl::LINEAR),
            };

            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, min_filter as _);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, mag_filter as _);

            let (wrap_s, wrap_t) = match repeat {
                piet_gpu::RepeatStrategy::Color(clr) => {
                    let (r, g, b, a) = clr.as_rgba();
                    gl::TexParameterfv(
                        gl::TEXTURE_2D,
                        gl::TEXTURE_BORDER_COLOR,
                        [r as f32, g as f32, b as f32, a as f32].as_ptr(),
                    );

                    (gl::CLAMP_TO_EDGE, gl::CLAMP_TO_EDGE)
                }
                piet_gpu::RepeatStrategy::Repeat => (gl::REPEAT, gl::REPEAT),
                _ => panic!("unsupported repeat strategy"),
            };

            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, wrap_s as _);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, wrap_t as _);

            Ok(texture as _)
        }
    }

    fn delete_texture(&self, texture: Self::Texture) {
        self.assert_context();

        unsafe {
            gl::DeleteTextures(1, &texture);
        }
    }

    fn write_texture<T: bytemuck::Pod>(
        &self,
        texture: &Self::Texture,
        size: (u32, u32),
        format: piet_gpu::ImageFormat,
        data: Option<&[T]>,
    ) {
        self.assert_context();

        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, *texture);

            let (internal_format, format, ty) = match format {
                piet_gpu::ImageFormat::Rgba => (gl::RGBA8, gl::RGBA, gl::UNSIGNED_BYTE),
                _ => panic!("unsupported image format"),
            };

            let (width, height) = size;
            let data_ptr = data
                .map(|data| data.as_ptr() as *const _)
                .unwrap_or(std::ptr::null());

            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                internal_format as _,
                width as _,
                height as _,
                0,
                format,
                ty,
                data_ptr,
            );
        }
    }

    fn write_subtexture<T: bytemuck::Pod>(
        &self,
        texture: &Self::Texture,
        offset: (u32, u32),
        size: (u32, u32),
        format: piet_gpu::ImageFormat,
        data: &[T],
    ) {
        self.assert_context();

        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, *texture);

            let (format, ty) = match format {
                piet_gpu::ImageFormat::Rgba => (gl::RGBA, gl::UNSIGNED_BYTE),
                _ => panic!("unsupported image format"),
            };

            let (width, height) = size;
            let (x, y) = offset;

            gl::TexSubImage2D(
                gl::TEXTURE_2D,
                0,
                x as _,
                y as _,
                width as _,
                height as _,
                format,
                ty,
                data.as_ptr() as *const _,
            );
        }
    }

    fn set_texture_interpolation(
        &self,
        texture: &Self::Texture,
        interpolation: piet::InterpolationMode,
    ) {
        self.assert_context();

        let mode = match interpolation {
            piet::InterpolationMode::Bilinear => gl::LINEAR,
            piet::InterpolationMode::NearestNeighbor => gl::NEAREST,
        };

        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, *texture);
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_MAG_FILTER,
                mode as gl::types::GLint,
            );
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_MIN_FILTER,
                mode as gl::types::GLint,
            );
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
    }

    fn max_texture_size(&self) -> (u32, u32) {
        self.assert_context();

        unsafe {
            let mut side = 0;
            gl::GetIntegerv(gl::MAX_TEXTURE_SIZE, &mut side);
            (side as u32, side as u32)
        }
    }

    fn create_buffer(&self) -> Result<Self::Buffer, Self::Error> {
        self.assert_context();

        unsafe {
            let mut buffer = 0;
            gl::GenBuffers(1, &mut buffer);
            Ok(buffer as _)
        }
    }

    fn write_buffer<T: bytemuck::Pod>(
        &self,
        buffer: &Self::Buffer,
        data: &[T],
        ty: BufferType,
    ) -> Result<(), Self::Error> {
        self.assert_context();

        unsafe {
            let bind_location = match ty {
                BufferType::Vertex => gl::ARRAY_BUFFER,
                BufferType::Index => gl::ELEMENT_ARRAY_BUFFER,
            };

            let len = (data.len() * std::mem::size_of::<T>()) as isize;
            gl::BindBuffer(bind_location, *buffer);
            gl::BufferData(
                bind_location,
                len,
                data.as_ptr() as *const _,
                gl::DYNAMIC_DRAW,
            );
            //gl::BindBuffer(bind_location, 0);

            Ok(())
        }
    }

    fn delete_buffer(&self, buffer: Self::Buffer) {
        self.assert_context();

        unsafe {
            gl::DeleteBuffers(1, &buffer);
        }
    }

    fn create_vertex_array(
        &self,
        buffer: &Self::Buffer,
        formats: &[piet_gpu::VertexFormat],
    ) -> Result<Self::VertexArray, Self::Error> {
        unsafe {
            let mut vertex_array = 0;
            gl::GenVertexArrays(1, &mut vertex_array);
            gl::BindBuffer(gl::ARRAY_BUFFER, *buffer);

            gl::BindVertexArray(vertex_array);

            // Get the location of the attributes.
            let position_location =
                gl::GetAttribLocation(self.render_program, b"aPos\0".as_ptr() as *const _);
            let tex_coord_location =
                gl::GetAttribLocation(self.render_program, b"aTexCoord\0".as_ptr() as *const _);
            let color_location =
                gl::GetAttribLocation(self.render_program, b"color\0".as_ptr() as *const _);

            for format in formats {
                let location = match format.data_type {
                    piet_gpu::DataType::Position => position_location,
                    piet_gpu::DataType::Texture => tex_coord_location,
                    piet_gpu::DataType::Color => color_location,
                    _ => panic!("unsupported data type"),
                };

                let data_ty = match format.format {
                    piet_gpu::DataFormat::Float => gl::FLOAT,
                    piet_gpu::DataFormat::UnsignedByte => gl::UNSIGNED_BYTE,
                    _ => panic!("unsupported data format"),
                };

                gl::VertexAttribPointer(
                    location as _,
                    format.num_components as _,
                    data_ty,
                    gl::FALSE,
                    format.stride as _,
                    format.offset as *const _,
                );

                gl::EnableVertexAttribArray(location as _);

                println!(
                    "Bound attribute {:?} to location {}",
                    &format, location as usize
                );
            }

            //gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);

            Ok(vertex_array as _)
        }
    }

    fn delete_vertex_array(&self, vertex_array: Self::VertexArray) {
        self.assert_context();

        unsafe {
            gl::DeleteVertexArrays(1, &vertex_array);
        }
    }

    fn push_buffers(
        &self,
        draw_buffers: piet_gpu::DrawBuffers<'_, Self>,
        current_texture: &Self::Texture,
        mask_texture: &Self::Texture,
        transform: &piet::kurbo::Affine,
        size: (u32, u32),
    ) -> Result<(), Self::Error> {
        unsafe {
            // Use our program.
            gl::UseProgram(self.render_program);

            // Set the viewport size.
            let (width, height) = size;
            gl::Viewport(0, 0, width as i32, height as i32);
            gl::Uniform2f(self.viewport_size, width as f32, height as f32);

            // Set the transform.
            let [a, b, c, d, e, f] = transform.as_coeffs();
            let transform = [
                a as f32, b as f32, 0.0, c as f32, d as f32, 0.0, e as f32, f as f32, 1.0,
            ];
            gl::UniformMatrix3fv(self.u_transform, 1, gl::FALSE, transform.as_ptr());

            // Set the mask texture.
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, *mask_texture);
            gl::Uniform1i(self.mask, 0);

            // Set the texture.
            gl::ActiveTexture(gl::TEXTURE1);
            gl::BindTexture(gl::TEXTURE_2D, *current_texture);
            gl::Uniform1i(self.tex, 1);

            // Set the blend mode.
            //gl::Enable(gl::BLEND);
            //gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);

            // Set vertex attributes.
            gl::BindVertexArray(*draw_buffers.vertex_array);

            // Set buffers.
            gl::BindBuffer(gl::ARRAY_BUFFER, *draw_buffers.vertex_buffer);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, *draw_buffers.index_buffer);

            // Draw.
            gl::DrawElements(
                gl::TRIANGLES,
                draw_buffers.num_indices as i32,
                gl::UNSIGNED_INT,
                std::ptr::null(),
            );

            // Unbind everything.
            gl::BindVertexArray(0);
            //gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, 0);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::UseProgram(0);
        }

        Ok(())
    }
}

const VERTEX_SHADER: &str = "
#version 330 core

in vec2 aPos;
in vec2 aTexCoord;
in vec4 color;

out vec4 rgbaColor;
out vec2 fTexCoord;
out vec2 fMaskCoord;

uniform mat3 transform;
uniform vec2 viewportSize;

void main() {
    // Transform the vertex position.
    //vec3 pos = transform * vec3(aPos, 1.0);
    //pos /= pos.z;
    vec2 pos = aPos;
    fMaskCoord = pos.xy + viewportSize;

    // Clamp to the viewport size.
    gl_Position = vec4(
        //(2.0 * pos.x / viewportSize.x) - 1.0,
        //1.0 - (2.0 * pos.y / viewportSize.y),
        aPos.x,
        aPos.y,
        0.0,
        1.0
    );

    rgbaColor = color / 255.0;
    fTexCoord = aTexCoord;
}
";

const FRAGMENT_SHADER: &str = "
#version 330 core

in vec4 rgbaColor;
in vec2 fTexCoord;
in vec2 fMaskCoord;

uniform sampler2D tex;
uniform sampler2D mask;

void main() {
    vec4 textureColor = texture2D(tex, fTexCoord);
    vec4 mainColor = rgbaColor * textureColor;

    float maskAlpha = texture2D(mask, fMaskCoord).a;
    maskAlpha += 1.0;
    vec4 finalColor = vec4(
        mainColor.rgb,
        mainColor.a + maskAlpha
    );

    gl_FragColor = finalColor;
}
";
