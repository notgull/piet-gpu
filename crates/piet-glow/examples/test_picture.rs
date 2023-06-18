// SPDX-License-Identifier: LGPL-3.0-or-later OR MPL-2.0
// This file is a part of `piet-glow`.
//
// `piet-glow` is free software: you can redistribute it and/or modify it under the terms of
// either:
//
// * GNU Lesser General Public License as published by the Free Software Foundation, either
// version 3 of the License, or (at your option) any later version.
// * Mozilla Public License as published by the Mozilla Foundation, version 2.
//
// `piet-glow` is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
// without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See the GNU Lesser General Public License or the Mozilla Public License for more details.
//
// You should have received a copy of the GNU Lesser General Public License and the Mozilla
// Public License along with `piet-glow`. If not, see <https://www.gnu.org/licenses/>.

//! Test harness for the comparison generator.
//!
//! This can only be run on desktop for now.

use std::cell::RefCell;

use glow::{Context, HasContext};
use piet::samples;
use piet_glow::GlContext;
use winit::event_loop::EventLoop;

#[path = "util/setup_context.rs"]
mod util;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    util::init();
    let event_loop = EventLoop::new();
    let mut glutin_setup = util::glutin_impl::GlutinSetup::new(&event_loop)?;

    event_loop.run(move |ev, elwt, flow| {
        flow.set_wait();

        if let winit::event::Event::<()>::Resumed = ev {
            // Create a rendering context.
            let glow_context = glutin_setup.make_current(elwt)();

            // Create the piet-glow context.
            let context = unsafe { piet_glow::GlContext::new(glow_context).unwrap() };

            // piet takes a raw function pointer, making this workaround necessary.
            std::thread_local! {
                static CONTEXT: RefCell<Option<GlContext<Context>>> = RefCell::new(None);
            }

            CONTEXT.with(move |slot| *slot.borrow_mut() = Some(context));

            samples::samples_main(
                |number, _scale, path| {
                    CONTEXT.with(|slot| {
                        let mut guard = slot.borrow_mut();
                        let context = guard.as_mut().unwrap();

                        // Uses unimplemented bits.
                        if [12, 16].contains(&number) {
                            return Ok(());
                        }

                        let picture = samples::get(number)?;
                        let size = picture.size();

                        // Create a texture to render into.
                        let ctx = context.context();
                        let texture = unsafe {
                            let texture = ctx.create_texture().unwrap();
                            ctx.bind_texture(glow::TEXTURE_2D, Some(texture));
                            ctx.tex_image_2d(
                                glow::TEXTURE_2D,
                                0,
                                glow::RGBA as i32,
                                size.width as i32,
                                size.height as i32,
                                0,
                                glow::RGBA,
                                glow::UNSIGNED_BYTE,
                                None,
                            );

                            // Set up the texture parameters.
                            ctx.tex_parameter_i32(
                                glow::TEXTURE_2D,
                                glow::TEXTURE_MIN_FILTER,
                                glow::LINEAR as i32,
                            );
                            ctx.tex_parameter_i32(
                                glow::TEXTURE_2D,
                                glow::TEXTURE_MAG_FILTER,
                                glow::LINEAR as i32,
                            );

                            texture
                        };

                        // Use a framebuffer to render into the texture and make it current.
                        let _framebuffer = unsafe {
                            let framebuffer = ctx.create_framebuffer().unwrap();
                            ctx.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                            ctx.framebuffer_texture_2d(
                                glow::FRAMEBUFFER,
                                glow::COLOR_ATTACHMENT0,
                                glow::TEXTURE_2D,
                                Some(texture),
                                0,
                            );

                            // Check that the framebuffer is complete.
                            assert_eq!(
                                ctx.check_framebuffer_status(glow::FRAMEBUFFER),
                                glow::FRAMEBUFFER_COMPLETE
                            );

                            framebuffer
                        };

                        // Use a renderbuffer to render into the texture and make it current.
                        let _renderbuffer = unsafe {
                            let renderbuffer = ctx.create_renderbuffer().unwrap();
                            ctx.bind_renderbuffer(glow::RENDERBUFFER, Some(renderbuffer));
                            ctx.renderbuffer_storage(
                                glow::RENDERBUFFER,
                                glow::DEPTH_COMPONENT16,
                                size.width as i32,
                                size.height as i32,
                            );
                            ctx.framebuffer_renderbuffer(
                                glow::FRAMEBUFFER,
                                glow::DEPTH_ATTACHMENT,
                                glow::RENDERBUFFER,
                                Some(renderbuffer),
                            );

                            // Check that the framebuffer is complete.
                            assert_eq!(
                                ctx.check_framebuffer_status(glow::FRAMEBUFFER),
                                glow::FRAMEBUFFER_COMPLETE
                            );

                            renderbuffer
                        };

                        // Create a piet-glow render context.
                        let mut rc = unsafe {
                            context.render_context(size.width as u32, size.height as u32)
                        };

                        // Draw with the context.
                        picture.draw(&mut rc)?;

                        // Get the data out of the texture.
                        let ctx = context.context();
                        let mut data = vec![0; size.width as usize * size.height as usize * 4];
                        unsafe {
                            ctx.read_pixels(
                                0,
                                0,
                                size.width as i32,
                                size.height as i32,
                                glow::RGBA,
                                glow::UNSIGNED_BYTE,
                                glow::PixelPackData::Slice(&mut data),
                            );
                        }

                        // Write the data to a file.
                        let mut img =
                            image::RgbaImage::from_vec(size.width as u32, size.height as u32, data)
                                .unwrap();

                        // Flip it around.
                        image::imageops::flip_vertical_in_place(&mut img);

                        img.save(path)?;

                        // Delete the texture and framebuffer.
                        unsafe {
                            ctx.delete_texture(texture);
                            ctx.delete_framebuffer(_framebuffer);
                            ctx.delete_renderbuffer(_renderbuffer);
                        }

                        Ok(())
                    })
                },
                "piet-glow",
                None,
            );
        }
    })
}
