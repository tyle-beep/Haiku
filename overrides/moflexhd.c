/* moflexhd: higher-resolution MOFLEX movies (issue #7).
 *
 * The movie player (mw::mo, Mobiclip) runs as the game's own code. Before a
 * decoded picture goes to the YUV-to-RGB converter, it is put through a
 * deblocking filter (mwmofilterDeblockerApply -> MoDeblock), hand-written
 * ARMv6 SIMD that is by far the most expensive part of playing a movie here:
 * about 43% of the CPU of the game's own 400x240 movies, and 58% of a
 * 1024x576 one's (dev/reports/moflexhd.md). The player itself already treats
 * it as optional: it only deblocks while the decoder is three or more
 * pictures ahead, and shows pictures undeblocked whenever it falls behind.
 *
 * A picture larger than the console's 400x240 (a modder's HD movie) is shown
 * undeblocked here, always: MoDeblock answers "nothing filtered" (0), which
 * mwmofilterDeblockerApply turns into the undeblocked picture, the same
 * thing the player does when it is behind. The game's own movies (all
 * 400x240, and every one smaller) go through the game's filter as before,
 * byte for byte.
 *
 * FATES_MOVIE_STATS=1 (tests) also times the decoder and the filter, by the
 * CPU time of the thread running them (zakuro-core moflexhd.rs).
 */
#include <stdint.h>
#include "overrides.h"

/* zakuro-core moflexhd.rs */
extern volatile uint32_t moflexhd_counting;
extern void moflexhd_note(uint32_t kind, uint64_t cycles, uint32_t pixels);
enum { MOFLEXHD_DECODE, MOFLEXHD_DEBLOCK, MOFLEXHD_UNDEBLOCKED };

#ifdef _WIN32
__declspec(dllimport) void *__stdcall GetCurrentThread(void);
__declspec(dllimport) int __stdcall QueryThreadCycleTime(void *thread, uint64_t *cycles);

static uint64_t moflexhd_cycles(void) {
    uint64_t cycles = 0;
    QueryThreadCycleTime(GetCurrentThread(), &cycles);
    return cycles;
}
#else
/* Linux: Haiku's own build. Thread CPU time in nanoseconds, which
 * moflexhd_note takes as well as cycles ("cycles or nanoseconds"). */
#include <time.h>

static uint64_t moflexhd_cycles(void) {
    struct timespec ts;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts);
    return (uint64_t)ts.tv_sec * 1000000000u + (uint64_t)ts.tv_nsec;
}
#endif

/* words: the thread's cycles at the call, low and high, and the pixels */
static void moflexhd_timed(Context *ctx, const uint32_t *words, uint32_t kind) {
    (void)ctx;
    uint64_t start = words[0] | (uint64_t)words[1] << 32;
    moflexhd_note(kind, moflexhd_cycles() - start, words[2]);
}
static void moflexhd_decoded(Context *ctx, const uint32_t *words) { moflexhd_timed(ctx, words, MOFLEXHD_DECODE); }
static void moflexhd_deblocked(Context *ctx, const uint32_t *words) { moflexhd_timed(ctx, words, MOFLEXHD_DEBLOCK); }

/* MoDeblock(const MoDeblocking&): r0 points at the picture's description,
   width at +0x18 and height at +0x1C (as mwmofilterDeblockerApply fills it) */
RECOMP_OVERRIDE(0x00169118) {
    uint32_t width = mem_read32(ctx, ctx->r[0] + 0x18), height = mem_read32(ctx, ctx->r[0] + 0x1C);
    if ((uint64_t)width * height > 400u * 240u) {
        if (moflexhd_counting) moflexhd_note(MOFLEXHD_UNDEBLOCKED, 0, width * height);
        ctx->r[0] = 0;
        RETURN_TO(ctx->r[14]);
    }
    if (moflexhd_counting) {
        uint64_t now = moflexhd_cycles();
        CALL_THEN(RECOMP_ORIGINAL(0x00169118), moflexhd_deblocked, (uint32_t)now, (uint32_t)(now >> 32), width * height);
        return;
    }
    CALL(RECOMP_ORIGINAL(0x00169118));
}

/* VX2DecoderUnpack(VX2DecoderData&, const void*, unsigned int): one picture
   decoded; the decoder's size is in its state, r0's first word: width at +4,
   height at +8 (VX2DecoderInit) */
RECOMP_OVERRIDE(0x001666E0) {
    if (moflexhd_counting) {
        uint32_t state = mem_read32(ctx, ctx->r[0]);
        uint32_t pixels = state ? mem_read32(ctx, state + 4) * mem_read32(ctx, state + 8) : 0;
        uint64_t now = moflexhd_cycles();
        CALL_THEN(RECOMP_ORIGINAL(0x001666E0), moflexhd_decoded, (uint32_t)now, (uint32_t)(now >> 32), pixels);
        return;
    }
    CALL(RECOMP_ORIGINAL(0x001666E0));
}

/* zakuro-core moflexhd.rs: says once that a movie this large can't play */
extern void moflexhd_unplayable(uint32_t width, uint32_t height);

/* mw::mo::helper::Player::DecodeVideoData(void), each turn of the player
   thread. The game's decoder (VX2DecoderInit) takes pictures up to 1024
   wide, and its textures are at most 1024x1024 (the 3DS's limit), so a
   larger movie never shows a picture: on the console and here before, the
   player waited for one forever, a black screen the game never left (and
   one over 1024 high would have its pictures written past its textures).
   Such a movie is now skipped as Start skips one: the movie thread
   (is::app::movie::MovieThread, whose player is at +0x140) is marked as
   having shown a picture (+0x6EF), which MovieLeave wants before it looks
   at the buttons, with none newer to wait for (+0xB8 -1, as before the
   first), and overrides/qol.c's MovieLeave gives it the Start
   press (moflexhd_skip); it marks the movie skipped, blacks out and stops
   the sound. Once it skips (+0x6F0), the player stops (state 7, which
   StartPlayback and the movie thread's loop leave on) and the movie ends
   the way a skipped one does. Nothing is decoded meanwhile. */
uint32_t moflexhd_skip;

RECOMP_OVERRIDE(0x002269F8) {
    uint32_t player = ctx->r[0];
    for (uint32_t screen = 0; screen < 2; screen++) {
        int32_t stream = (int32_t)mem_read32(ctx, player + 0x48C + screen * 4);
        if (stream < 0 || stream > 7) continue;
        uint32_t decoder = mem_read32(ctx, player + 0x30C + (uint32_t)stream * 4);
        uint32_t info = player + 0x32C + (uint32_t)stream * 0x28;
        uint32_t width = mem_read32(ctx, info + 0x14), height = mem_read32(ctx, info + 0x18);
        int unplayable = width > 1024 || height > 1024 || (decoder && !mem_read32(ctx, decoder + 4));
        if (unplayable) {
            uint32_t thread = player - 0x140;
            moflexhd_unplayable(width, height);
            if (mem_read8(ctx, thread + 0x6F0)) {
                mem_write32(ctx, player + 0xC, 7);
                moflexhd_skip = 0;
            } else {
                mem_write8(ctx, thread + 0x6EF, 1);
                mem_write32(ctx, thread + 0xB8, 0xFFFFFFFFu); /* no picture to wait for */
                moflexhd_skip = 1;
            }
            RETURN_TO(ctx->r[14]);
        }
    }
    CALL(RECOMP_ORIGINAL(0x002269F8));
}

/* mw::mo::mobiclip::Decoder::GetNbPushedFrame(int&): a decoder whose
   VX2DecoderInit failed (a movie over 1024 wide) has no state and leaves the
   count unwritten, and the player's callers (FlushPendingFrames,
   VideoImageRequestStart) then use whatever the stack held as a number of
   pictures. Such a decoder now has none (a safeguard; the skip above keeps
   the player from it). */
RECOMP_OVERRIDE(0x005C506C) {
    if (!mem_read32(ctx, ctx->r[0] + 4)) {
        mem_write32(ctx, ctx->r[1], 0);
        ctx->r[0] = 0;
        RETURN_TO(ctx->r[14]);
    }
    CALL(RECOMP_ORIGINAL(0x005C506C));
}
