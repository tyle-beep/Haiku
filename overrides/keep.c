/* pulsefix: what the overrides keep of the console outside its memory.
 *
 * Some overrides hold, in their own C variables, things that belong to the
 * console as much as its memory and registers do:
 *
 *   - where a thread is in a chain of guest calls an override makes, which
 *     return into the override (snd_native.c's SndSeq, qol.c's QolCall,
 *     cameras.c's WalkStep, cheats.c's Job, ...): the thread's registers
 *     say it is in the middle of the chain, the record says where;
 *   - guest objects an override made and goes on using (info_ui.c's line
 *     VertexBuffer, made with the game's operator new);
 *   - what an override follows of the game while it runs (the battle free
 *     camera's BattleSeq, the walk's proc, a support's pending flag, ...).
 *
 * Divine Pulse's moments (zakuro-core state/rewind.rs) and saved states
 * (state/mod.rs) put the console's memory back as it was; these variables
 * have to go back with it, or the game goes on with an override that
 * believes another moment: info_ui.c went on drawing into a VertexBuffer
 * the moment had not made yet (memory the game's heap gave to something
 * else), a thread came back into an override with another thread's record.
 *
 * Each file lists its variables (<file>_keep); the host takes the list once
 * (haiku_keep_regions) and copies those bytes with each moment and state,
 * and back. Only plain values go in: guest addresses, counters, floats,
 * never a host pointer. The tails registry (recomp.h, CALL_THEN) is kept
 * by the host the same way, through recomp_tails_keep/put.
 */
#include "overrides.h"

typedef struct {
    const char *name;
    void *at;
    uint32_t size;
} KeepRegion;

typedef void (*KeepAdd)(const char *name, void *at, uint32_t size);

#define KEEP_MAX 96
static KeepRegion keep_regions[KEEP_MAX];
static uint32_t keep_count;

static void keep_add(const char *name, void *at, uint32_t size) {
    if (keep_count < KEEP_MAX) {
        keep_regions[keep_count].name = name;
        keep_regions[keep_count].at = at;
        keep_regions[keep_count].size = size;
        keep_count++;
    }
}

void info_ui_keep(KeepAdd add);
void snd_native_keep(KeepAdd add);
void qol_keep(KeepAdd add);
void amiibo_keep(KeepAdd add);
void mods_keep(KeepAdd add);
void cameras_keep(KeepAdd add);
void cheats_keep(KeepAdd add);
void supports_keep(KeepAdd add);
void randomizer_keep(KeepAdd add);
void gfx_eft_keep(KeepAdd add);
void transblur_keep(KeepAdd add);
void boot3_keep(KeepAdd add);
void bossmusic_keep(KeepAdd add);

/* the variables, listed once; count back, the list in *out */
RECOMP_EXPORT uint32_t haiku_keep_regions(const KeepRegion **out) {
    if (!keep_count) {
        info_ui_keep(keep_add);
        snd_native_keep(keep_add);
        qol_keep(keep_add);
        amiibo_keep(keep_add);
        mods_keep(keep_add);
        cameras_keep(keep_add);
        cheats_keep(keep_add);
        supports_keep(keep_add);
        randomizer_keep(keep_add);
        gfx_eft_keep(keep_add);
        transblur_keep(keep_add);
        boot3_keep(keep_add);
        bossmusic_keep(keep_add);
    }
    *out = keep_regions;
    return keep_count;
}
