//! moflexhd: a movie's pictures (issue #7, higher-resolution MOFLEX movies).
//!
//! The movie player has each picture converted (YUV to RGB) into the same
//! texture, so every picture is a texture of its own to the 2D renderer (it
//! knows textures by their address and their texels). A texture the
//! converter writes keeps only its last few pictures, decoded (lib.rs
//! `moflexhd_keep_few`) and on the GPU (render.rs `texture_view`), instead
//! of every picture until a limit far away: 4096 decoded textures, and 600
//! frames on the GPU. Any other texture is kept as before.
//! `FATES_MOVIE_STREAM=0` keeps a movie's pictures too, as before.

use crate::prim::TexRef;

/// pictures of one texture kept: the one being drawn, the one before (two
/// eyes, a picture in between) and one to spare.
pub const KEPT_VERSIONS: usize = 3;

/// whether the texture is a movie's picture, whose old pictures go.
pub fn few_kept(tex: TexRef) -> bool {
    !streaming_off() && zakuro_core::moflexhd::is_picture(tex.addr)
}

/// FATES_MOVIE_STREAM=0: every picture kept, as before (tests).
pub fn streaming_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("FATES_MOVIE_STREAM").is_some_and(|v| v == "0"))
}

/// moflexdec: a picture of a modder's HD movie: a texture the converter
/// writes that is larger than the game's own movies' (400x240 pictures in
/// 512x256 textures). Its pictures differ every time, so the renderer
/// skips the texture pack lookup for them (two hashes of the whole
/// texture; no pack can name a picture of a movie) and decodes their
/// texels on a few threads. The game's own movies are untouched.
pub fn hd_picture(tex: TexRef) -> bool {
    tex.width * tex.height > 512 * 256 && zakuro_core::moflexhd::is_hd_picture(tex.addr)
}

/// moflexdec: decode_texture over bands of whole tile rows, side by side on
/// up to four threads for a texture of 256K texels or more; each band comes
/// out as the whole texture's decode has it (flipped, a band's rows land
/// from the bottom up), so the texels are the same (unit test).
pub fn decode_bands(data: &[u8], format: zakuro_gpu::texture::TextureFormat, width: u32, height: u32, flip: bool, out: &mut [u8]) {
    use zakuro_gpu::texture::{decode_texture, TextureFormat};
    let (w, h) = (width as usize, height as usize);
    let bytes = format.bits_per_pixel() as usize / 8;
    let tiled = matches!(format, TextureFormat::Rgb8 | TextureFormat::Rgba8) && w.is_multiple_of(8) && h.is_multiple_of(8);
    let tile_rows = h / 8;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let parts = ((w * h) / (256 * 1024)).clamp(1, 4).min(cores.saturating_sub(1).max(1)).min(tile_rows.max(1));
    if !tiled || parts <= 1 || data.len() < w * h * bytes || out.len() < w * h * 4 {
        decode_texture(data, format, width, height, flip, out);
        return;
    }
    let per = tile_rows.div_ceil(parts);
    let (band_in, band_out) = (per * 8 * w * bytes, per * 8 * w * 4);
    let ins = data[..w * h * bytes].chunks(band_in);
    let out = &mut out[..w * h * 4];
    // flipped, the first band of rows is the last of the picture
    let outs: Vec<&mut [u8]> = if flip { out.rchunks_mut(band_out).collect() } else { out.chunks_mut(band_out).collect() };
    std::thread::scope(|scope| {
        let mut bands = ins.zip(outs);
        let own = bands.next();
        for (input, output) in bands {
            scope.spawn(move || decode_texture(input, format, width, (input.len() / (w * bytes)) as u32, flip, output));
        }
        if let Some((input, output)) = own {
            decode_texture(input, format, width, (input.len() / (w * bytes)) as u32, flip, output);
        }
    });
}
