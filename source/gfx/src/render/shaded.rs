//! The renderer's second pipeline: H3D meshes drawn with the game's vertex
//! program through zakuro-gpu's shade.vert, turned upright like the 2D
//! layer, and its raster.frag fragment stages (scene.rs builds the draws).

use std::collections::HashMap;
use std::sync::Arc;

use ash::vk;
use zakuro_gpu::native3d;

use super::{glsl_to_spirv, vkerr, Buffer, Decoded, Image, Renderer};
use crate::prim::{Sampling, TexRef};
use crate::scene::{Key, Mip, Shaded};

pub(super) struct ShadedGpu {
    set_layout: vk::DescriptorSetLayout,
    pub(super) layout: vk::PipelineLayout,
    vs: vk::ShaderModule,
    fs: vk::ShaderModule,
    /// gfx-final: the depth-writing build (Key::writes_depth)
    fs_depth: vk::ShaderModule,
    pipelines: HashMap<Key, vk::Pipeline>,
    /// drawdist: shade.vert for batches of tufts (drawdist.rs), its own
    /// pipelines; every other draw keeps the plain shade.vert
    vs_batch: vk::ShaderModule,
    pipelines_batch: HashMap<Key, vk::Pipeline>,
    /// skybg: raster.frag for draws with a cube map (and its depth-writing
    /// build), their own pipelines; None if it did not build (they then
    /// draw with the plain raster.frag, the first face flat, as before)
    fs_cube: Option<(vk::ShaderModule, vk::ShaderModule)>,
    pipelines_cube: HashMap<Key, vk::Pipeline>,
    pub(super) buffer: Option<Buffer>,
    used: u64,
    /// gfx-terrain: samplers with a level of detail
    mip_samplers: HashMap<(Sampling, Mip), vk::Sampler>,
}

const ALIGN: u64 = 256;

/// shade.vert with its position turned upright, as the 2D layer's vertex
/// shader turns the PICA's portrait framebuffer.
fn upright_shade_vert() -> String {
    upright_shade_vert_from(native3d::SHADE_VERT)
}

/// lfgrass: one edit of a shader's source, said once in the log when the
/// text it looks for isn't there (an edit that matches nothing used to drop
/// its feature without a word: the far grass was lost that way)
fn patch(src: &str, what: &str, from: &str, to: &str) -> String {
    if !src.contains(from) {
        crate::perf::warn_once(format!("gfx: shade.vert's {what} edit found nothing to change; that feature is off"));
    }
    src.replace(from, to)
}

/// lfgrass: the edits work on LF text. include_str! keeps the file's bytes,
/// and a Windows checkout (Git's core.autocrlf) gives the shader CRLF, which
/// the multi-line edits (drawdist's) never matched: no far grass. So the
/// source is made LF first, whatever the checkout.
fn upright_shade_vert_from(source: &str) -> String {
    let src = source.replace('\r', "");
    // wide2: a full-width layer's sides (wide2.rs)
    let src = patch(&src, "wide2", "gl_Position = vec4(position.xy, -position.z, position.w);", crate::wide2::SHADE_VERT_SIDE);
    // wideart: a translucent 5:3 picture's sides start on its edge and sample the side panel (wide_ui.rs)
    let src = patch(&src, "wideart start", "gl_Position = vec4(position.xy, -position.z, position.w);", crate::wide_ui::SHADE_VERT_PANEL_START);
    let src = patch(&src, "wideart panel", "out_texcoord2 = vec2(semantic(12u, 0.0), semantic(13u, 0.0));", crate::wide_ui::SHADE_VERT_PANEL);
    let src = patch(
        &src,
        "upright",
        "gl_Position = vec4(position.xy, -position.z, position.w);",
        "gl_Position = vec4(-position.y, position.x, -position.z, position.w);",
    );
    // gfxcastle: the battle free camera's sky (freecam.rs)
    patch(&src, "sky haze", "out_depth = position.z / position.w * depth_map.x + depth_map.y;", SKY_HAZE)
}

/// drawdist: the shade.vert of batches of tufts, their instances' places
/// and wind (drawdist.rs)
fn batch_shade_vert() -> String {
    batch_shade_vert_from(native3d::SHADE_VERT)
}

fn batch_shade_vert_from(source: &str) -> String {
    let src = upright_shade_vert_from(source);
    let src = patch(&src, "drawdist globals", crate::drawdist::SHADE_VERT_GLOBALS.0, crate::drawdist::SHADE_VERT_GLOBALS.1);
    let src = patch(&src, "drawdist uniform", crate::drawdist::SHADE_VERT_UNIFORM.0, crate::drawdist::SHADE_VERT_UNIFORM.1);
    patch(&src, "drawdist main", crate::drawdist::SHADE_VERT_MAIN.0, crate::drawdist::SHADE_VERT_MAIN.1)
}

#[cfg(test)]
mod lfgrass_tests {
    /// lfgrass: every edit lands, from an LF and from a CRLF checkout alike
    #[test]
    fn shade_vert_edits_land_whatever_the_line_endings() {
        let lf = zakuro_gpu::native3d::SHADE_VERT.replace('\r', "");
        let crlf = lf.replace('\n', "\r\n");
        for source in [&lf, &crlf] {
            let src = super::batch_shade_vert_from(source);
            assert!(src.contains(crate::drawdist::SHADE_VERT_MAIN.1), "drawdist main");
            assert!(src.contains(crate::drawdist::SHADE_VERT_GLOBALS.1), "drawdist globals");
            assert!(src.contains(crate::drawdist::SHADE_VERT_UNIFORM.1), "drawdist uniform");
            assert!(src.contains("gl_Position = vec4(-position.y, position.x, -position.z, position.w);"), "upright");
            assert!(!src.contains('\r'));
        }
    }
}

/// gfxcastle: the battle free camera's haze overlays (freecam.rs): the mode
/// in the output semantics' spare slot 22 (low byte; the strength in the next,
/// the distance haze's full reach in the high half), the dome's up in 23, ten
/// bits a component (wide2 has the Shading block's pad). The alpha the
/// overlay's combiners take: on the dome by the elevation, on the field by
/// the distance from the eye (w).
const SKY_HAZE: &str = "out_depth = position.z / position.w * depth_map.x + depth_map.y;
    uint haze = semantics[22];
    if ((haze & 0xFFu) == 1u || (haze & 0xFFu) == 2u) {
        float alpha = 0.0;
        if ((haze & 0xFFu) == 1u) {
            uint packed = semantics[23];
            vec3 up = vec3(float(packed & 0x3FFu), float((packed >> 10) & 0x3FFu), float((packed >> 20) & 0x3FFu)) / 511.5 - 1.0;
            float elevation = dot(normalize(inputs[0].xyz), normalize(up));
            alpha = elevation >= 0.0 ? smoothstep(0.57, 0.88, elevation) : smoothstep(0.03, 0.42, -elevation);
        } else {
            float reach = float(haze >> 16);
            alpha = smoothstep(0.5 * reach, reach, position.w);
        }
        out_color = vec4(1.0, 1.0, 1.0, alpha * float((haze >> 8) & 0xFFu) / 255.0);
    }";

/// what one shaded draw needs bound.
pub(super) struct Prepared {
    pub pipeline: vk::Pipeline,
    pub set: vk::DescriptorSet,
    pub index_offset: u64,
    pub count: u32,
}

impl Renderer {
    pub(super) fn shaded_init(&mut self) -> Result<(), String> {
        if self.shaded.is_some() {
            return Ok(());
        }
        let sampler_binding = |b: u32| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(b)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        };
        let buffer_binding = |b: u32, ty: vk::DescriptorType, stage: vk::ShaderStageFlags| {
            vk::DescriptorSetLayoutBinding::default().binding(b).descriptor_type(ty).descriptor_count(1).stage_flags(stage)
        };
        let bindings = [
            sampler_binding(0),
            sampler_binding(1),
            sampler_binding(2),
            buffer_binding(3, vk::DescriptorType::UNIFORM_BUFFER, vk::ShaderStageFlags::FRAGMENT),
            buffer_binding(4, vk::DescriptorType::STORAGE_BUFFER, vk::ShaderStageFlags::FRAGMENT),
            buffer_binding(5, vk::DescriptorType::STORAGE_BUFFER, vk::ShaderStageFlags::VERTEX),
            buffer_binding(6, vk::DescriptorType::STORAGE_BUFFER, vk::ShaderStageFlags::VERTEX),
            buffer_binding(7, vk::DescriptorType::STORAGE_BUFFER, vk::ShaderStageFlags::VERTEX),
        ];
        unsafe {
            let set_layout = self
                .device
                .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)
                .map_err(vkerr("shaded set layout"))?;
            let set_layouts = [set_layout];
            let layout = self
                .device
                .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts), None)
                .map_err(vkerr("shaded layout"))?;
            let vs_code = glsl_to_spirv(&upright_shade_vert(), naga::ShaderStage::Vertex).inspect_err(|e| eprintln!("gfx: shade.vert does not compile, {e}"))?;
            let vs = self
                .device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vs_code), None)
                .map_err(vkerr("shade.vert module"))?;
            let fs_words: Vec<u32> = native3d::RASTER_FRAG.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
            let fs = self
                .device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&fs_words), None)
                .map_err(vkerr("raster.frag module"))?;
            // gfx-final: the depth-writing build
            let fs_depth_words: Vec<u32> = native3d::RASTER_FRAG_DEPTH.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
            let fs_depth = self
                .device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&fs_depth_words), None)
                .map_err(vkerr("raster.frag depth module"))?;
            // drawdist
            let vs_batch_code = glsl_to_spirv(&batch_shade_vert(), naga::ShaderStage::Vertex).inspect_err(|e| eprintln!("gfx: drawdist's shade.vert does not compile, {e}"))?;
            let vs_batch = self
                .device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vs_batch_code), None)
                .map_err(vkerr("drawdist shade.vert module"))?;
            // skybg: the cube-map build of raster.frag
            let mut fs_cube = None;
            if crate::skybg::enabled() {
                match (crate::skybg::cube_frag(false), crate::skybg::cube_frag(true)) {
                    (Ok(plain), Ok(depth)) => {
                        let plain = self.device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&plain), None).map_err(vkerr("cube raster.frag module"))?;
                        let depth = self.device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&depth), None).map_err(vkerr("cube raster.frag depth module"))?;
                        fs_cube = Some((plain, depth));
                    }
                    (Err(e), _) | (_, Err(e)) => eprintln!("gfx: the cube-map raster.frag does not build, {e}"),
                }
            }
            self.shaded = Some(ShadedGpu {
                set_layout,
                layout,
                vs,
                fs,
                fs_depth,
                pipelines: HashMap::new(),
                vs_batch,
                pipelines_batch: HashMap::new(),
                fs_cube,
                pipelines_cube: HashMap::new(),
                buffer: None,
                used: 0,
                mip_samplers: HashMap::new(),
            });
            // smooth: the listed pipelines made ahead (cache.rs); the
            // renderer that goes first stops the thread (Drop). They are
            // made without the shared pipeline cache: the driver holds a
            // cache's lock while it compiles, and a frame that needed a
            // pipeline waited 336 ms behind the warm thread (cold run);
            // the driver's own shader cache keeps what they compiled
            let (device, render_pass) = (self.device.clone(), self.render_pass);
            crate::cache::warm_start(move |bytes| {
                let Some(key) = key_from(bytes) else { return true };
                if let Ok(pipeline) = make_shaded(&device, vk::PipelineCache::null(), key, layout, vs, fs, fs_depth, render_pass) {
                    // made here and used by no one (inside shaded_init's unsafe)
                    device.destroy_pipeline(pipeline, None);
                }
                true
            });
        }
        Ok(())
    }

    fn shaded_pipeline(&mut self, key: Key, batch: bool, cube: bool) -> Result<vk::Pipeline, String> {
        let render_pass = self.render_pass;
        let g = self.shaded.as_mut().unwrap();
        // skybg: a cube map's draw, the cube build of raster.frag (not on
        // the made-ahead list)
        if let Some((fs, fs_depth)) = g.fs_cube.filter(|_| cube && !batch) {
            if let Some(&p) = g.pipelines_cube.get(&key) {
                return Ok(p);
            }
            let _t = crate::perf::scope(crate::perf::C::Pipeline);
            let (layout, vs) = (g.layout, g.vs);
            let p = make_shaded(&self.device, self.pipeline_disk.cache, key, layout, vs, fs, fs_depth, render_pass)
                .map_err(|e| format!("shaded cube pipeline: {e:?}"))?;
            self.pipeline_disk.created(&self.device);
            self.shaded.as_mut().unwrap().pipelines_cube.insert(key, p);
            return Ok(p);
        }
        // drawdist: a batch of tufts, its own shade.vert (not on the made-ahead list)
        if batch {
            if let Some(&p) = g.pipelines_batch.get(&key) {
                return Ok(p);
            }
            let _t = crate::perf::scope(crate::perf::C::Pipeline);
            let (layout, vs, fs, fs_depth) = (g.layout, g.vs_batch, g.fs, g.fs_depth);
            let p = make_shaded(&self.device, self.pipeline_disk.cache, key, layout, vs, fs, fs_depth, render_pass)
                .map_err(|e| format!("shaded pipeline: {e:?}"))?;
            self.pipeline_disk.created(&self.device);
            self.shaded.as_mut().unwrap().pipelines_batch.insert(key, p);
            return Ok(p);
        }
        if let Some(&p) = g.pipelines.get(&key) {
            return Ok(p);
        }
        let _t = crate::perf::scope(crate::perf::C::Pipeline); // gfx-perf
        // smooth: made by make_shaded, which the warm thread shares; the key
        // goes on the list the next run makes ahead
        let (layout, vs, fs, fs_depth) = (g.layout, g.vs, g.fs, g.fs_depth);
        let p = make_shaded(&self.device, self.pipeline_disk.cache, key, layout, vs, fs, fs_depth, render_pass)
            .map_err(|e| format!("shaded pipeline: {e:?}"))?;
        crate::cache::note_shaded_key(key_bytes(&key));
        self.pipeline_disk.created(&self.device);
        self.shaded.as_mut().unwrap().pipelines.insert(key, p);
        Ok(p)
    }

    /// msaa: a shaded pipeline for `render_pass` at `samples`, each sample
    /// shaded apart over `shading` of them (0: once a pixel); msaa.rs keeps it.
    pub(super) fn shaded_make_at(&mut self, key: Key, render_pass: vk::RenderPass, samples: u32, shading: f32, batch: bool, cube: bool) -> Result<vk::Pipeline, String> {
        let g = self.shaded.as_ref().unwrap();
        let (layout, vs, fs, fs_depth) = (g.layout, if batch { g.vs_batch } else { g.vs }, g.fs, g.fs_depth); // drawdist: batch
        // skybg: a cube map's draw
        let (fs, fs_depth) = g.fs_cube.filter(|_| cube && !batch).unwrap_or((fs, fs_depth));
        let p = make_shaded_at(&self.device, self.pipeline_disk.cache, key, layout, vs, fs, fs_depth, render_pass, samples, shading)
            .map_err(|e| format!("shaded msaa pipeline: {e:?}"))?;
        self.pipeline_disk.created(&self.device);
        Ok(p)
    }

    /// room in the shaded buffer for this frame's draws.
    pub(super) fn shaded_reserve(&mut self, draws: &[&Shaded]) -> Result<(), String> {
        self.shaded_init()?;
        let al = |n: u64| n.div_ceil(ALIGN) * ALIGN;
        let mut size = 0u64;
        // perf3: sets, not lists searched through for each draw (a
        // thousand draws made that a million comparisons a render); and
        // index lists once each, as shaded_prepare puts them
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::with_capacity(draws.len() * 2);
        for s in draws {
            // gfx-perf: tables shared by draws go up once a frame
            if seen.insert(Arc::as_ptr(&s.tables) as usize) {
                size += al(s.tables.len() as u64 * 4);
            }
            if seen.insert(Arc::as_ptr(&s.program) as usize) {
                size += al(s.program.len() as u64 * 4);
            }
            if seen.insert(Arc::as_ptr(&s.vertices) as usize) {
                size += al(s.vertices.len() as u64 + 12);
            }
            if seen.insert(Arc::as_ptr(&s.indices) as usize) {
                size += al(s.indices.len() as u64 * 4);
            }
            size += al(s.shading.len() as u64 * 4) + al(s.draw.len() as u64 * 4);
        }
        let have = self.shaded.as_ref().unwrap().buffer.as_ref().map_or(0, |b| b.size);
        if have < size.max(ALIGN) {
            if let Some(old) = self.shaded.as_mut().unwrap().buffer.take() {
                zakuro_gpu::wait_idle(&self.device).ok(); // smooth: keeps off the present thread
                self.destroy_buffer(old);
            }
            let usage = vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::UNIFORM_BUFFER | vk::BufferUsageFlags::INDEX_BUFFER;
            let b = self.buffer((size.max(ALIGN) * 3 / 2).max(1 << 20), usage)?;
            self.shaded.as_mut().unwrap().buffer = Some(b);
        }
        self.shaded.as_mut().unwrap().used = 0;
        Ok(())
    }

    /// perf3: `len` zero bytes at `at` (just put, so inside the buffer)
    fn shaded_zero(&mut self, at: u64, len: usize) {
        let g = self.shaded.as_mut().unwrap();
        let b = g.buffer.as_ref().unwrap();
        unsafe { std::ptr::write_bytes(b.ptr.add(at as usize), 0, len) };
        g.used = g.used.max((at + len as u64).div_ceil(ALIGN) * ALIGN);
    }

    fn shaded_put(&mut self, bytes: &[u8]) -> u64 {
        crate::perf::count(crate::perf::C::ShadedBytes, bytes.len() as u64); // gfx-perf
        let g = self.shaded.as_mut().unwrap();
        let b = g.buffer.as_ref().unwrap();
        let at = g.used;
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), b.ptr.add(at as usize), bytes.len()) };
        g.used = (at + bytes.len() as u64).div_ceil(ALIGN) * ALIGN;
        at
    }

    /// a shaded draw's pipeline, descriptors and data, with `placed` the
    /// offsets of programs and vertex arrays already written this frame.
    pub(super) fn shaded_prepare(
        &mut self,
        s: &Shaded,
        keys: &[Option<u64>; 3],
        decoded: &HashMap<(TexRef, u64), Arc<Decoded>>,
        placed: &mut HashMap<usize, (u64, u64)>,
        samples: u32, // msaa
    ) -> Result<Prepared, String> {
        // msaa: into the samples with a pipeline made for them
        let pipeline = if samples > 1 { self.msaa_shaded_pipeline(s, samples)? } else { self.shaded_pipeline(s.key, crate::drawdist::is_batch(s), crate::skybg::is_cube(s))? };
        // perf3: the words as their bytes in place (little-endian, as the
        // GPU reads them), not copied into a new list a byte at a time: a
        // render puts every draw's Shading, Draw and indices, and at 175
        // pictures a second with the free camera's ~1000 draws that copying
        // was most of the presenter's time
        fn bytes(w: &[u32]) -> &[u8] {
            // SAFETY: u32 has no padding and u8 no alignment
            unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, std::mem::size_of_val(w)) }
        }
        let program = match placed.get(&(Arc::as_ptr(&s.program) as usize)) {
            Some(&p) => p,
            None => {
                let b = bytes(&s.program);
                let p = (self.shaded_put(b), b.len() as u64);
                placed.insert(Arc::as_ptr(&s.program) as usize, p);
                p
            }
        };
        let vertices = match placed.get(&(Arc::as_ptr(&s.vertices) as usize)) {
            Some(&p) => p,
            None => {
                // a word past the end, a value can run into the word after
                let padded = s.vertices.len().div_ceil(4) * 4 + 8;
                let at = self.shaded_put(&s.vertices);
                self.shaded_zero(at + s.vertices.len() as u64, padded - s.vertices.len());
                let p = (at, padded as u64);
                placed.insert(Arc::as_ptr(&s.vertices) as usize, p);
                p
            }
        };
        let shading = {
            let b = bytes(&s.shading);
            (self.shaded_put(b), b.len() as u64)
        };
        let draw = {
            let b = bytes(&s.draw);
            (self.shaded_put(b), b.len() as u64)
        };
        // gfx-perf: once a frame per table set, as programs are
        let tables = match placed.get(&(Arc::as_ptr(&s.tables) as usize)) {
            Some(&p) => p,
            None => {
                let bytes: &[u8] = unsafe { std::slice::from_raw_parts(s.tables.as_ptr() as *const u8, s.tables.len() * 4) };
                let p = (self.shaded_put(bytes), bytes.len() as u64);
                placed.insert(Arc::as_ptr(&s.tables) as usize, p);
                p
            }
        };
        // perf3: once a render per index list, as vertices are (a draw's
        // pictures in between and its haze overlay share it)
        let index_offset = match placed.get(&(Arc::as_ptr(&s.indices) as usize)) {
            Some(&p) => p.0,
            None => {
                let b = bytes(&s.indices);
                let p = (self.shaded_put(b), b.len() as u64);
                placed.insert(Arc::as_ptr(&s.indices) as usize, p);
                p.0
            }
        };
        let mut views = [vk::ImageView::null(); 3];
        let mut samplers = [vk::Sampler::null(); 3];
        for unit in 0..3 {
            let (view, sampler) = match (&s.textures[unit], keys[unit]) {
                (Some((tex, sampling)), Some(hash)) => {
                    let k = (*tex, hash);
                    let view = self.texture_view(k, decoded.get(&k))?;
                    // gfx-terrain: the unit's sizes and level of detail
                    // selena: of the image drawn with, which stays on the
                    // GPU when its texels have gone from `decoded` (it was
                    // drawn from its first size alone then)
                    let mip = match self.textures.get(&k) {
                        Some(t) if t.levels > 1 => s.mips[unit],
                        _ => Mip { levels: 1, ..Mip::default() },
                    };
                    // hdpacks: a pack's picture, larger than the texture,
                    // sampled over its own sizes
                    let mip = match self.textures.get(&k).and_then(|t| Some((t.levels, crate::hd::shift(t.width, tex.width)?))) {
                        Some((levels, shift)) if levels > 1 => crate::hd::mip(s.mips[unit], *sampling, levels, shift),
                        _ => mip,
                    };
                    (view, self.mip_sampler(*sampling, mip)?)
                }
                _ => (self.white.view, self.sampler(Sampling { wrap: 0, wrap_t: 0, sharp: false, linear_mag: false, linear_min: false })?),
            };
            views[unit] = view;
            samplers[unit] = sampler;
        }
        let g = self.shaded.as_ref().unwrap();
        let buffer = g.buffer.as_ref().unwrap().buffer;
        let layouts = [g.set_layout];
        let set = unsafe {
            self.device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.descriptor_pool).set_layouts(&layouts))
        }
        .map_err(vkerr("shaded descriptor set"))?[0];
        let image_infos: Vec<[vk::DescriptorImageInfo; 1]> = (0..3)
            .map(|u| [vk::DescriptorImageInfo::default().image_view(views[u]).sampler(samplers[u]).image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)])
            .collect();
        let info = |(offset, range): (u64, u64)| [vk::DescriptorBufferInfo::default().buffer(buffer).offset(offset).range(range)];
        let (i3, i4, i5, i6, i7) = (info(draw), info(tables), info(program), info(shading), info(vertices));
        let mut writes = Vec::new();
        for unit in 0..3 {
            writes.push(
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(unit as u32)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&image_infos[unit]),
            );
        }
        writes.push(vk::WriteDescriptorSet::default().dst_set(set).dst_binding(3).descriptor_type(vk::DescriptorType::UNIFORM_BUFFER).buffer_info(&i3));
        for (b, i) in [(4u32, &i4), (5, &i5), (6, &i6), (7, &i7)] {
            writes.push(vk::WriteDescriptorSet::default().dst_set(set).dst_binding(b).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).buffer_info(i));
        }
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        Ok(Prepared { pipeline, set, index_offset, count: s.indices.len() as u32 })
    }
}

impl Renderer {
    /// gfx-terrain: a sampler for a shaded draw's unit, as the emulated GPU
    /// makes it (zakuro-gpu raster/hardware.rs `Filtering::of`): one size
    /// minifies with the magnification filter; several go between their
    /// levels of detail as the unit's LOD word says.
    fn mip_sampler(&mut self, s: Sampling, mip: Mip) -> Result<vk::Sampler, String> {
        if let Some(&x) = self.shaded.as_ref().and_then(|g| g.mip_samplers.get(&(s, mip))) {
            return Ok(x);
        }
        // gfx-2d: s and t wrap apart, as the PICA does
        let address = |wrap: u8| match wrap {
            0 => vk::SamplerAddressMode::CLAMP_TO_EDGE,
            1 => vk::SamplerAddressMode::CLAMP_TO_BORDER,
            2 => vk::SamplerAddressMode::REPEAT,
            _ => vk::SamplerAddressMode::MIRRORED_REPEAT,
        };
        let (mode, mode_t) = (address(s.wrap), address(s.wrap_t));
        let f = |linear: bool| if linear { vk::Filter::LINEAR } else { vk::Filter::NEAREST };
        let (minify, between, levels, bias) = if mip.levels > 1 {
            (mip.linear, mip.between, (mip.min as f32, mip.max as f32), mip.bias as f32 / 256.0)
        } else {
            (s.linear_mag, false, (0.0, 0.0), 0.0)
        };
        let limits = unsafe { self.instance.get_physical_device_properties(self.physical) }.limits;
        let max_bias = limits.max_sampler_lod_bias;
        // gfx2d3: textures with smaller sizes, minified linearly, are
        // filtered anisotropically: ground seen at a slant (the battle's
        // terrain from the free camera, the HD Map Floors) keeps the detail
        // along the view where one isotropic size blurred it across. The
        // game's level range and bias stay as they are, so nothing is
        // sharper than the game allows straight on.
        let aniso = if mip.levels > 1 && mip.linear { self.anisotropy(limits.max_sampler_anisotropy) } else { 1.0 };
        let sampler = unsafe {
            self.device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(f(s.linear_mag))
                    .min_filter(f(minify))
                    .mipmap_mode(if between { vk::SamplerMipmapMode::LINEAR } else { vk::SamplerMipmapMode::NEAREST })
                    .mip_lod_bias(bias.clamp(-max_bias, max_bias))
                    .address_mode_u(mode)
                    .address_mode_v(mode_t)
                    .address_mode_w(mode)
                    .border_color(vk::BorderColor::FLOAT_TRANSPARENT_BLACK)
                    .min_lod(levels.0)
                    .max_lod(levels.1)
                    .anisotropy_enable(aniso > 1.0) // gfx2d3
                    .max_anisotropy(aniso),
                None,
            )
        }
        .map_err(vkerr("sampler"))?;
        if let Some(g) = self.shaded.as_mut() {
            g.mip_samplers.insert((s, mip), sampler);
        }
        Ok(sampler)
    }

    /// gfx2d3: the anisotropy 3D textures with smaller sizes are filtered
    /// with: 16 (the device's limit if lower), FATES_GFX_ANISO=n to set it,
    /// 1 to turn it off. Both devices it draws on (the presenter's and its
    /// own) enable the feature when the GPU has it.
    fn anisotropy(&self, limit: f32) -> f32 {
        static WANT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        let want = *WANT.get_or_init(|| crate::perf::var("FATES_GFX_ANISO").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(16.0));
        let supported = unsafe { self.instance.get_physical_device_features(self.physical) }.sampler_anisotropy == vk::TRUE;
        if supported { want.clamp(1.0, limit.max(1.0)) } else { 1.0 }
    }

    /// gfx-terrain: a texture with its smaller sizes (`rgba` holds them one
    /// after the other, the largest first), as an image with that many
    /// levels.
    pub(super) fn upload_levels(&mut self, width: u32, height: u32, levels: u32, rgba: &[u8]) -> Result<Image, String> {
        // gfx-perf: recorded with the frame (FATES_GFX_UPLOAD_WAIT=1: as here)
        if !super::upload_wait() {
            return self.upload_deferred_levels(width, height, levels, rgba);
        }
        self.settle();
        let format = vk::Format::R8G8B8A8_UNORM;
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: levels,
            base_array_layer: 0,
            layer_count: 1,
        };
        let image = unsafe {
            let image = self
                .device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
                        .extent(vk::Extent3D { width, height, depth: 1 })
                        .mip_levels(levels)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )
                .map_err(vkerr("image"))?;
            let req = self.device.get_image_memory_requirements(image);
            let ty = self.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)?;
            let memory = self
                .device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None)
                .map_err(vkerr("image memory"))?;
            self.device.bind_image_memory(image, memory, 0).map_err(vkerr("bind image"))?;
            let view = self
                .device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default().image(image).view_type(vk::ImageViewType::TYPE_2D).format(format).subresource_range(range),
                    None,
                )
                .map_err(vkerr("image view"))?;
            Image { image, memory, view }
        };
        self.ensure(3, rgba.len() as u64)?;
        let staging = self.staging.as_ref().unwrap();
        unsafe {
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), staging.ptr, rgba.len());
        }
        let sb = staging.buffer;
        let mut regions = Vec::new();
        let mut offset = 0u64;
        for level in 0..levels {
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            regions.push(
                vk::BufferImageCopy::default()
                    .buffer_offset(offset)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: level,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D { width: w, height: h, depth: 1 }),
            );
            offset += (w * h * 4) as u64;
        }
        let barrier = |from: vk::ImageLayout, to: vk::ImageLayout| {
            vk::ImageMemoryBarrier::default()
                .image(image.image)
                .old_layout(from)
                .new_layout(to)
                .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(range)
        };
        self.begin()?;
        unsafe {
            let all = vk::PipelineStageFlags::ALL_COMMANDS;
            let to_dst = [barrier(vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL)];
            self.device.cmd_pipeline_barrier(self.commands, all, all, vk::DependencyFlags::empty(), &[], &[], &to_dst);
            self.device.cmd_copy_buffer_to_image(self.commands, sb, image.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &regions);
            let to_read = [barrier(vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            self.device.cmd_pipeline_barrier(self.commands, all, all, vk::DependencyFlags::empty(), &[], &[], &to_read);
        }
        self.submit_and_wait()?;
        Ok(image)
    }
}

/// smooth: a shaded pipeline's key as the warm lists keep it (cache.rs).
fn key_bytes(key: &Key) -> crate::cache::KeyBytes {
    let mut b = [0u8; 16];
    b[0] = key.blend.is_some() as u8;
    b[1..5].copy_from_slice(&key.blend.unwrap_or(0).to_le_bytes());
    b[5..9].copy_from_slice(&key.mask.to_le_bytes());
    b[9] = key.depth_test as u8 | (key.depth_write as u8) << 1 | (key.lighting as u8) << 2 | (key.procedural as u8) << 3 | (key.writes_depth as u8) << 4;
    b[10] = key.depth_func;
    b[11] = key.cull;
    b[12] = key.topology;
    b[15] = 1; // the encoding's version
    b
}

/// smooth: the key back from the list, none for another version's.
fn key_from(b: &crate::cache::KeyBytes) -> Option<Key> {
    if b[15] != 1 || b[13] != 0 || b[14] != 0 || b[9] >> 5 != 0 || b[0] > 1 || b[11] > 2 || b[12] > 2 || b[10] > 7 {
        return None;
    }
    Some(Key {
        blend: (b[0] == 1).then(|| u32::from_le_bytes(b[1..5].try_into().unwrap())),
        mask: u32::from_le_bytes(b[5..9].try_into().unwrap()),
        depth_test: b[9] & 1 != 0,
        depth_func: b[10],
        depth_write: b[9] & 2 != 0,
        cull: b[11],
        lighting: b[9] & 4 != 0,
        procedural: b[9] & 8 != 0,
        topology: b[12],
        writes_depth: b[9] & 16 != 0,
    })
}

/// smooth: makes the shaded pipeline for a key (the method above and the
/// warm thread, which makes listed ones ahead through the same cache).
#[allow(clippy::too_many_arguments)]
fn make_shaded(
    device: &ash::Device,
    cache: vk::PipelineCache,
    key: Key,
    layout: vk::PipelineLayout,
    vs: vk::ShaderModule,
    fs: vk::ShaderModule,
    fs_depth: vk::ShaderModule,
    render_pass: vk::RenderPass,
) -> Result<vk::Pipeline, vk::Result> {
    make_shaded_at(device, cache, key, layout, vs, fs, fs_depth, render_pass, 1, 0.0)
}

/// msaa: make_shaded at `samples`, shading `shading` of them apart (0:
/// once a pixel); 1 and 0 make exactly make_shaded's pipeline.
#[allow(clippy::too_many_arguments)]
fn make_shaded_at(
    device: &ash::Device,
    cache: vk::PipelineCache,
    key: Key,
    layout: vk::PipelineLayout,
    vs: vk::ShaderModule,
    fs: vk::ShaderModule,
    fs_depth: vk::ShaderModule,
    render_pass: vk::RenderPass,
    samples: u32,
    shading: f32,
) -> Result<vk::Pipeline, vk::Result> {
    // raster.frag's specialization: the generic build, which reads the
    // combiners and the rest from the Draw block, with lighting and the
    // procedural texture as constants
    let mut constants = [0u32; 30];
    constants[25] = if key.procedural { 0x400 } else { 0 };
    constants[26] = key.lighting as u32;
    // gfx-final: with the depth-writing build's bit, as the hardware
    // renderer's generic pipelines carry it
    constants[28] = 2 | if key.writes_depth { 4 } else { 0 };
    constants[29] = 1;
    let entries: Vec<vk::SpecializationMapEntry> =
        (0..30u32).map(|i| vk::SpecializationMapEntry::default().constant_id(i).offset(i * 4).size(4)).collect();
    let data: Vec<u8> = constants.iter().flat_map(|c| c.to_le_bytes()).collect();
    let spec = vk::SpecializationInfo::default().map_entries(&entries).data(&data);
    let stages = [
        vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vs).name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(if key.writes_depth { fs_depth } else { fs }) // gfx-final
            .name(c"main")
            .specialization_info(&spec),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let topology = match key.topology {
        1 => vk::PrimitiveTopology::TRIANGLE_STRIP,
        2 => vk::PrimitiveTopology::TRIANGLE_FAN,
        _ => vk::PrimitiveTopology::TRIANGLE_LIST,
    };
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(topology);
    let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
    // gfx-terrain: culling on, its winding checked against the emulated
    // picture in shadow mode (Chapter 1 cutscene, f29000: capes and
    // hoods showed their inner faces; mean difference 9.4 -> 8.4 with
    // this mapping, 23.7 with the other). FATES_GFX_CULL3D=0 turns it
    // off, =2 tries the other winding.
    let cull = match (std::env::var("FATES_GFX_CULL3D").ok().as_deref(), key.cull) {
        (Some("0"), _) => vk::CullModeFlags::NONE,
        (Some("2"), 1) => vk::CullModeFlags::BACK,
        (Some("2"), 2) => vk::CullModeFlags::FRONT,
        (_, 1) => vk::CullModeFlags::FRONT,
        (_, 2) => vk::CullModeFlags::BACK,
        _ => vk::CullModeFlags::NONE,
    };
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(cull)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::from_raw(samples.max(1))); // msaa
    // msaa: alpha-tested draws shaded per sample, so the test cuts each one
    let multisample = if shading > 0.0 { multisample.sample_shading_enable(true).min_sample_shading(shading) } else { multisample };
    let compare = |f: u8| match f {
        0 => vk::CompareOp::NEVER,
        1 => vk::CompareOp::ALWAYS,
        2 => vk::CompareOp::EQUAL,
        3 => vk::CompareOp::NOT_EQUAL,
        4 => vk::CompareOp::LESS,
        5 => vk::CompareOp::LESS_OR_EQUAL,
        6 => vk::CompareOp::GREATER,
        _ => vk::CompareOp::GREATER_OR_EQUAL,
    };
    // the PICA writes depth with its test off too
    let (test, func) = if key.depth_test { (true, compare(key.depth_func)) } else { (key.depth_write, vk::CompareOp::ALWAYS) };
    let depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(test)
        .depth_write_enable(key.depth_write)
        .depth_compare_op(func);
    // the PICA numbers its blend factors and equations as Vulkan does
    let attachment = match key.blend {
        Some(config) => {
            let factor = |raw: u32| if raw & 0xF == 15 { vk::BlendFactor::ONE } else { vk::BlendFactor::from_raw((raw & 0xF) as i32) };
            let equation = |raw: u32| if raw & 7 > 4 { vk::BlendOp::ADD } else { vk::BlendOp::from_raw((raw & 7) as i32) };
            vk::PipelineColorBlendAttachmentState::default()
                .blend_enable(true)
                .color_blend_op(equation(config))
                .alpha_blend_op(equation(config >> 8))
                .src_color_blend_factor(factor(config >> 16))
                .dst_color_blend_factor(factor(config >> 20))
                .src_alpha_blend_factor(factor(config >> 24))
                .dst_alpha_blend_factor(factor(config >> 28))
        }
        None => vk::PipelineColorBlendAttachmentState::default(),
    }
    .color_write_mask(vk::ColorComponentFlags::from_raw(key.mask));
    let attachments = [attachment];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR, vk::DynamicState::BLEND_CONSTANTS];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let info = [vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .depth_stencil_state(&depth)
        .color_blend_state(&blend)
        .dynamic_state(&dynamic)
        .layout(layout)
        .render_pass(render_pass)];
    // smooth: through the logged maker (kind 1: shaded)
    crate::cache::make_pipeline(device, cache, info[0], 1.0)
}
