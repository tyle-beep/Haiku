/* bossmusic: Boss Music Stays (GitHub issue #20). Once a boss has been
 * fought, its battle theme stays on as the map's music, as in Three Houses
 * and Engage, until the boss is defeated or gone; then the map's own music
 * comes back. Off by default (bossmusic_on, set by the host from
 * pc.boss_music). Audio only: nothing here writes game memory or changes
 * what the game does, only which of its own BGM calls run and with which
 * name; online battles (versus) are left alone.
 *
 * ---- how Fates picks its map and battle music (USA v2 addresses) ----
 *
 * map::sound keeps its state at 0x006D7F38: +4 a fader proc, +8 TSound[6]
 * (0x41 bytes each: a type byte, then a name of 64; +0x188 the battle's
 * slot, +0x18C the battle's "mode", +0x18D the battle has a boss talk),
 * +0xC the pinch theme's name, +0x10 the boss theme of maps whose music the
 * player picks. map::sound::Setup (0x003998C4, at each map load and resume)
 * fills slot 0 with the map's BGM (map/config/<chapter>.bin, type 1) and a
 * slot with the pinch theme (type 3).
 *
 * The map BGMs (STRM_MAP_*) have three tracks: track 0 the map arrangement,
 * track 1 a mono layer, track 2 the battle arrangement. A battle cross-fades
 * them (map::sound::setTrackVolume, 0x003986C0: track mask, volume, fade):
 * OnFadeInBattle track 0 to 0.35 and track 2 to 1.0, OnFadeInBattleEnd
 * track 0 to 0, and back on the map OnFadeOutBattleEnd track 0 to 1.0 over
 * 5 s and OnFadeInMap track 2 to 0 over 2 s.
 *
 * map::sound::SetBattleInfo (0x003977DC), as each battle sequence starts,
 * picks the battle's mode: 4 when an enemy (force 1) in it has a battle
 * theme of its own, Person +0x88 (every chapter boss has one: STRM_BOSS_A1
 * to STRM_BOSS_E1, the amiibo heroes STRM_BOSS_MARTH...; in maps whose
 * music the player picks, an enemy with the leader skill SPID_Leader gets
 * the map's boss theme), its name put in a TSound slot of type 4 (+0x188
 * that slot); 3 when a unit is about to die (the pinch theme). For mode 3
 * and 4, OnFadeInBattle starts a ProcBGMFader (0x00398388) that stops the
 * map BGM and plays the slot's theme (from where it last was, which
 * ProcSoundMonitor keeps for every slot), OnFadeOutBattle (0x00397E68)
 * stops it (map::sound::stopBGM(1000), 0x00398DA4) and OnFadeInMap plays
 * the map BGM again (map::sound::Bgm::Map::Play(2000), 0x00398E58, through
 * map::sound::playBGM, 0x00398C68, which cross-fades: the old BGM stops and
 * the new one starts with the same fade). Events stop, duck and resume the
 * map BGM with Sound::BGMPlay/BGMStop/BGMVolume and Bgm::Map::Resume. The
 * phases have no music of their own.
 *
 * ---- what this does ----
 *
 * - SetBattleInfo, after the game's: a mode-4 battle engages its boss (the
 *   enemy whose theme it is; most recent first, up to BM_BOSSES).
 * - OnFadeOutBattle: when the battle's theme is the theme of the boss that
 *   should play (the most recently engaged one still on the map: force 1,
 *   HP above 0) its stop is skipped, so the theme carries on; it is now the
 *   map's music ("boss music").
 * - playBGM with the map BGM's name (Map::Play after a battle, Map::Resume
 *   after an event, the map's start): while a boss should play, its theme
 *   instead, from where it was; when that theme is already playing it is
 *   left alone and only its volume brought back (Sound::BGMVolume(100) in
 *   the call's place, and the track volumes that follow get the call's
 *   fade, not 0).
 * - a later battle with the same theme (the same boss, or another with the
 *   same theme) keeps it playing: its fader and the stops before it are
 *   skipped ("hold"). Any other battle keeps the boss music too, its track
 *   cross-fades skipped (the boss themes have no battle track: they would
 *   only mute it); a pinch or another boss's battle plays its own theme as
 *   the game does, and the boss music comes back after it.
 * - ProcSoundMonitor::Persistent (0x001EEF4C, every tick on a map), out of
 *   battle: when what plays is the map BGM or a boss theme this put on and
 *   another should play (the boss was defeated, retreated or joined, or a
 *   boss was found again after a resume), the change is made as the game
 *   makes it: Map::Play(2000) back to the map BGM, or playBGM(2000) to the
 *   boss theme, as a tail call in the monitor's place (its position count
 *   skips one tick).
 * - after a load or resume (Setup), the bosses are found again: an enemy
 *   with a theme of its own whose HP is below its maximum (Unit::GetMHPImpl,
 *   0x0052A364, called from the monitor, one unit a tick; the call returns
 *   into the monitor's override) counts as fought.
 * - left alone: online battles (versus::Config set), the maps whose music
 *   is volume only (the final chapters, config +0xD), and whatever plays
 *   that is not the map BGM or a boss theme (events' own music, the pinch
 *   theme): nothing is changed while it plays.
 *
 * FATES_BGM_LOG=1 logs every BGM change map::sound and Sound make (with the
 * game's tick) and what this does, to stderr; it logs with the option off
 * too. FATES_BOSS_MUSIC=0/1 overrides the setting (tests). */
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "overrides.h"

/* bossmusic: the host's word (pc.boss_music), and counts for the run summary */
uint32_t bossmusic_on = 0;
uint32_t bossmusic_engaged = 0;  /* bosses engaged (battles and found again) */
uint32_t bossmusic_kept = 0;     /* boss themes kept on after a battle */
uint32_t bossmusic_played = 0;   /* boss themes played in the map BGM's place */
uint32_t bossmusic_held = 0;     /* battles that kept the boss theme playing */
uint32_t bossmusic_back = 0;     /* back to the map BGM (the boss gone) */

/* ---- the game (USA v2) ---- */
#define BM_STATE        0x006D7F38u /* map::sound's state */
#define BM_SITUATION    0x006D7ED0u /* map::Situation*: 0 off a map */
#define BM_FORCES       0x006D9F44u /* Force::Get's forces, 12 bytes each, units linked at +0x14 */
#define BM_SOUND        0x006DC588u /* SoundPrivate*: +0x578 the BGM handles (0x20 each), +0xB30 the current */
#define BM_MONITOR      0x006D7F88u /* ProcSoundMonitor*: +0x30 each slot's position */
#define BM_BGM_FLAGS    0x006DC424u /* Sound's BGM commands: byte enabled, +4 the pause count */
#define BM_VERSUS       0x006DA318u /* versus::Config*: set in online and local battles */
#define BM_CONFIG       0x0077956Cu /* StructList of map/config/<chapter>.bin: +0x10 count, +0x14 records */
#define BM_GAMETIME     0x006DCA84u /* GameTime: +0 ticks */

#define BM_SET_BATTLE   0x003977DCu /* map::sound::SetBattleInfo(const BattleInfo*, bool) */
#define BM_FADE_OUT     0x00397E68u /* map::sound::OnFadeOutBattle(void) */
#define BM_PLAY         0x00398C68u /* map::sound::(anon)::playBGM(const char*, int fade, long position, int) */
#define BM_STOP         0x00398DA4u /* map::sound::(anon)::stopBGM(int fade) */
#define BM_TRACK        0x003986C0u /* map::sound::(anon)::setTrackVolume(unsigned mask, float, int fade) */
#define BM_FADER        0x00398388u /* map::sound::(anon)::ProcBGMFader::Create(ProcInst*, Type, int slot) */
#define BM_MAP_PLAY     0x00398E58u /* map::sound::Bgm::Map::Play(int fade) */
#define BM_SETUP        0x003998C4u /* map::sound::Setup(void) */
#define BM_MONITOR_TICK 0x001EEF4Cu /* ProcSoundMonitor::Persistent(void) */
#define BM_BGM_VOLUME   0x00420858u /* Sound::BGMVolume(int volume, int fade) */
#define BM_BGM_PLAY     0x004201A0u /* Sound::BGMPlay(const char*, int fade, SoundStartable*) */
#define BM_BGM_STOP     0x00420244u /* Sound::BGMStop(int fade) */
#define BM_GET_MHP      0x0052A364u /* Unit::GetMHPImpl(bool, bool, Type) const */

#define U_NEXT   0x14u
#define U_PERSON 0x9Cu
#define U_FORCE  0xA4u
#define U_HP     0xF3u
#define P_BGM    0x88u /* Person: the battle theme of its own */

#define BM_BOSSES 8
#define BM_NAME   64

enum { CUR_UNKNOWN, CUR_MAP, CUR_BOSS, CUR_OTHER, CUR_PENDING };

typedef struct {
    uint32_t unit, person, name; /* name: a guest string (a TSound slot's, or Person +0x88) */
} BmBoss;

/* everything kept with Divine Pulse's moments and saved states (keep.c):
   plain values and guest addresses only */
typedef struct {
    uint32_t on;          /* the word as last seen (a change re-derives) */
    uint32_t count;       /* bosses[0] the most recent */
    BmBoss bosses[BM_BOSSES];
    uint32_t in_battle, mode, hold, skip_once;
    uint32_t cur, cur_name, cur_sound, cur_id; /* what plays: CUR_*, a boss theme's name, its sound */
    uint32_t fix_fade;    /* the fade the next track volumes get (a kept theme's volume back) */
    uint32_t derive, derive_unit; /* 0 find the bosses again, 1 done; the unit after which to look */
    uint32_t call, call_tls, call_lr, call_this, call_unit; /* a GetMHPImpl call out */
    uint32_t tsound;      /* the TSound array the list is for */
} BmState;
static BmState bm;

void bossmusic_keep(void (*add)(const char *, void *, uint32_t)) {
    add("bossmusic.state", &bm, sizeof bm);
}

static int bm_log_on(void) {
    static int on = -1;
    if (on < 0) on = getenv("FATES_BGM_LOG") != 0;
    return on;
}

static int bm_enabled(void) {
    static int env = -2;
    if (env == -2) {
        const char *v = getenv("FATES_BOSS_MUSIC");
        env = v ? (strcmp(v, "0") != 0) : -1;
    }
    if (env >= 0) return env;
    return *(volatile uint32_t *)&bossmusic_on != 0;
}

static uint32_t bm_tick(Context *ctx) { return mem_read32(ctx, BM_GAMETIME); }

static void bm_cstr(Context *ctx, uint32_t a, char *out) {
    int i = 0;
    if (a)
        for (; i < BM_NAME - 1; i++) {
            uint8_t b = mem_read8(ctx, a + (uint32_t)i);
            if (!b) break;
            out[i] = (b >= 0x20 && b < 0x7F) ? (char)b : '?';
        }
    out[i] = 0;
}

#define BM_LOG(...) do { if (bm_log_on()) { fprintf(stderr, "[bgm] t=%u ", bm_tick(ctx)); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

static int bm_same(Context *ctx, uint32_t a, uint32_t b) {
    if (!a || !b) return 0;
    if (a == b) return 1;
    for (uint32_t i = 0; i < BM_NAME; i++) {
        uint8_t x = mem_read8(ctx, a + i), y = mem_read8(ctx, b + i);
        if (x != y) return 0;
        if (!x) return 1;
    }
    return 1;
}

static uint32_t bm_tsound(Context *ctx) { return mem_read32(ctx, BM_STATE + 8); }

/* the BGM handle that plays, and its sound (0 when stopped: Stop detaches it) */
static uint32_t bm_handle(Context *ctx) {
    uint32_t sp = mem_read32(ctx, BM_SOUND);
    return sp ? sp + 0x578u + mem_read32(ctx, sp + 0xB30u) * 0x20u : 0;
}
static uint32_t bm_sound_now(Context *ctx) {
    uint32_t h = bm_handle(ctx);
    return h ? mem_read32(ctx, h + 4) : 0;
}
static uint32_t bm_sound_id(Context *ctx, uint32_t sound) { return sound ? mem_read32(ctx, sound + 0xB0u) : 0xFFFFFFFFu; }

/* what plays is the one this put on (or saw start) */
static int bm_cur_playing(Context *ctx) {
    uint32_t s = bm_sound_now(ctx);
    return s && s == bm.cur_sound && bm_sound_id(ctx, s) == bm.cur_id;
}

static void bm_saw_play(Context *ctx, uint32_t kind, uint32_t name) {
    bm.cur = kind;
    bm.cur_name = name;
    bm.cur_sound = bm_sound_now(ctx);
    bm.cur_id = bm_sound_id(ctx, bm.cur_sound);
}

/* whether the feature may act here: on, on a map, not online, the map's music not volume only */
static int bm_active(Context *ctx) {
    if (!bm_enabled()) return 0;
    if (!mem_read32(ctx, BM_SITUATION)) return 0;
    if (mem_read32(ctx, BM_VERSUS)) return 0;
    uint32_t n = mem_read32(ctx, BM_CONFIG + 0x10);
    if (n) {
        uint32_t rec = mem_read32(ctx, mem_read32(ctx, BM_CONFIG + 0x14));
        if (rec && mem_read8(ctx, rec + 0xD)) return 0;
    }
    return 1;
}

/* a boss is still to be fought on: it is the unit it was, an enemy, alive */
static int bm_alive(Context *ctx, const BmBoss *b) {
    uint32_t forces = mem_read32(ctx, BM_FORCES);
    if (!forces || !b->unit) return 0;
    uint32_t u = mem_read32(ctx, forces + 1 * 12);
    for (int guard = 0; u && guard < 256; guard++, u = mem_read32(ctx, u + U_NEXT))
        if (u == b->unit)
            return mem_read32(ctx, u + U_PERSON) == b->person && mem_read8(ctx, u + U_HP) > 0;
    return 0;
}

/* the boss whose theme should play: the most recently engaged still there */
static const BmBoss *bm_wanted(Context *ctx) {
    if (bm.tsound != bm_tsound(ctx)) return 0;
    for (uint32_t i = 0; i < bm.count; i++)
        if (bm_alive(ctx, &bm.bosses[i]))
            return &bm.bosses[i];
    return 0;
}

static void bm_engage(Context *ctx, uint32_t unit, uint32_t name, const char *how) {
    uint32_t person = mem_read32(ctx, unit + U_PERSON);
    uint32_t i;
    for (i = 0; i < bm.count; i++)
        if (bm.bosses[i].unit == unit && bm.bosses[i].person == person)
            break;
    if (i == bm.count && bm.count < BM_BOSSES) bm.count++;
    if (i >= BM_BOSSES) i = BM_BOSSES - 1;
    for (; i > 0; i--) bm.bosses[i] = bm.bosses[i - 1];
    bm.bosses[0].unit = unit;
    bm.bosses[0].person = person;
    bm.bosses[0].name = name;
    bossmusic_engaged++;
    if (bm_log_on()) {
        char n[BM_NAME];
        bm_cstr(ctx, name, n);
        BM_LOG("boss %s: unit %08X hp %u, theme %s (%u engaged)", how, unit, mem_read8(ctx, unit + U_HP), n, bm.count);
    }
}

/* the TSound slot with this name, for its position (ProcSoundMonitor's), or -1 */
static int bm_slot_of(Context *ctx, uint32_t name) {
    uint32_t ts = bm_tsound(ctx);
    if (!ts) return -1;
    for (int i = 0; i < 6; i++) {
        uint32_t s = ts + (uint32_t)i * 0x41u;
        if (mem_read8(ctx, s) && bm_same(ctx, s + 1, name)) return i;
    }
    return -1;
}
static uint32_t bm_position(Context *ctx, uint32_t name) {
    int slot = bm_slot_of(ctx, name);
    uint32_t mon = mem_read32(ctx, BM_MONITOR);
    return (slot >= 0 && mon) ? mem_read32(ctx, mon + 0x30u + (uint32_t)slot * 4u) : 0;
}

/* a tail call: fn runs in place of the override, returning to its caller */
static void bm_tail(Context *ctx, uint32_t fn) {
    ctx->r[15] = fn;
    ctx->thumb = 0;
    ctx->budget -= 4;
    CALL(recomp_call);
}

/* ---- map::sound::Setup: a map loaded or resumed: a new list ---- */

static void bm_setup_done(Context *ctx, const uint32_t *words) {
    (void)words;
    memset(&bm.count, 0, sizeof bm - offsetof(BmState, count));
    bm.tsound = bm_tsound(ctx);
    BM_LOG("map sound set up: map BGM slot 0, mode %u", mem_read8(ctx, bm.tsound + 0x18Cu));
}

RECOMP_OVERRIDE(0x003998C4) {
    CALL_THEN(RECOMP_ORIGINAL(0x003998C4), bm_setup_done, 0);
}

/* ---- map::sound::SetBattleInfo: a battle sequence starts (and ends, with 0) ---- */

static void bm_battle_info(Context *ctx, const uint32_t *words) {
    uint32_t info = words[0];
    if (!info) {
        if (bm.in_battle) BM_LOG("battle over%s", bm.hold ? " (the boss music held)" : "");
        bm.in_battle = 0;
        bm.mode = 0;
        bm.hold = 0;
        return;
    }
    uint32_t ts = bm_tsound(ctx);
    if (!ts) return;
    bm.in_battle = 1;
    bm.mode = mem_read8(ctx, ts + 0x18Cu);
    bm.hold = 0;
    uint32_t slot = mem_read32(ctx, ts + 0x188u);
    uint32_t name = (bm.mode >= 3 && slot < 6) ? ts + slot * 0x41u + 1 : 0;
    if (bm_log_on()) {
        char n[BM_NAME];
        bm_cstr(ctx, name, n);
        BM_LOG("battle: mode %u%s%s, talk %u", bm.mode, name ? ", theme " : "", n, mem_read8(ctx, ts + 0x18Du));
    }
    if (!bm_active(ctx) || bm.mode != 4 || !name) return;
    if (bm.tsound != ts) {
        memset(&bm.count, 0, sizeof bm - offsetof(BmState, count));
        bm.tsound = ts;
        bm.in_battle = 1;
        bm.mode = 4;
    }
    /* the boss: the enemy of the two (the game looks at the defender, +0x204, first) */
    uint32_t sides[2] = {mem_read32(ctx, info + 0x204u), mem_read32(ctx, info + 4u)}, boss = 0;
    for (int i = 0; i < 2 && !boss; i++) {
        uint32_t u = sides[i];
        uint32_t f = u ? mem_read32(ctx, u + U_FORCE) : 0;
        if (f && mem_read8(ctx, f + 8) == 1) boss = u;
    }
    if (!boss) return;
    /* the same theme already plays as the map's music: it keeps playing */
    if (bm.cur == CUR_BOSS && bm_cur_playing(ctx) && bm_same(ctx, bm.cur_name, name) &&
        !mem_read32(ctx, BM_BGM_FLAGS + 4)) {
        bm.hold = 1;
        bossmusic_held++;
        BM_LOG("hold: the theme plays on through this battle");
    }
    bm_engage(ctx, boss, name, "engaged");
}

RECOMP_OVERRIDE(0x003977DC) {
    CALL_THEN(RECOMP_ORIGINAL(0x003977DC), bm_battle_info, ctx->r[0], ctx->r[1]);
}

/* ---- the battle's own theme: kept, or stopped as the game does ---- */

static void bm_fade_out_done(Context *ctx, const uint32_t *words) {
    (void)words;
    bm.skip_once = 0;
}

RECOMP_OVERRIDE(0x00397E68) {
    uint32_t ts = bm_tsound(ctx);
    uint32_t mode = ts ? mem_read8(ctx, ts + 0x18Cu) : 0;
    int held = bm.hold;
    bm.hold = 0;
    if ((mode == 3 || mode == 4) && bm_active(ctx)) {
        uint32_t slot = mem_read32(ctx, ts + 0x188u);
        uint32_t name = slot < 6 ? ts + slot * 0x41u + 1 : 0;
        const BmBoss *w = bm_wanted(ctx);
        int playing = bm_cur_playing(ctx) && bm_same(ctx, bm.cur_name, name);
        if (bm_log_on()) {
            char n[BM_NAME];
            bm_cstr(ctx, name, n);
            BM_LOG("battle fades out: mode %u theme %s, boss to play %08X (hp %u), playing %d", mode, n, w ? w->unit : 0,
                   w ? mem_read8(ctx, w->unit + U_HP) : 0, playing);
        }
        if (mode == 4 && w && playing && bm_same(ctx, w->name, name)) {
            /* the boss's theme carries on as the map's music */
            if (!held && bm.cur != CUR_BOSS) {
                bm_saw_play(ctx, CUR_BOSS, w->name);
                bossmusic_kept++;
            }
            bm.skip_once = 1;
            BM_LOG("keep: the boss theme stays on");
            CALL_THEN(RECOMP_ORIGINAL(0x00397E68), bm_fade_out_done, 0);
            return;
        }
    }
    CALL(RECOMP_ORIGINAL(0x00397E68));
}

/* map::sound::stopBGM */
RECOMP_OVERRIDE(0x00398DA4) {
    if (bm.hold || bm.skip_once) {
        BM_LOG("stop %d ms skipped (%s)", (int)ctx->r[0], bm.hold ? "hold" : "kept");
        bm.skip_once = 0;
        RETURN_TO(ctx->r[14]);
    }
    BM_LOG("stop %d ms", (int)ctx->r[0]);
    CALL(RECOMP_ORIGINAL(0x00398DA4));
}

/* ProcBGMFader::Create: the battle theme's fader (mode 3 and 4) */
RECOMP_OVERRIDE(0x00398388) {
    if (bm.hold) {
        BM_LOG("fader for slot %u skipped (hold)", ctx->r[2]);
        RETURN_TO(ctx->r[14]);
    }
    BM_LOG("fader: type %u slot %u", ctx->r[1] & 0xFF, ctx->r[2]);
    CALL(RECOMP_ORIGINAL(0x00398388));
}

/* setTrackVolume: the map BGM's layers */
RECOMP_OVERRIDE(0x003986C0) {
    float vol = vfp_s(ctx, 0);
    if (bm.cur == CUR_BOSS && bm_enabled() && bm_cur_playing(ctx)) {
        if (bm.in_battle && bm.mode != 3 && bm.mode != 4) {
            /* a battle without a theme of its own: the boss theme plays on as it is */
            BM_LOG("track %X to %.2f in %d ms skipped (the boss music plays on)", ctx->r[0], (double)vol, (int)ctx->r[1]);
            RETURN_TO(ctx->r[14]);
        }
        if (bm.fix_fade && !ctx->r[1]) ctx->r[1] = bm.fix_fade;
        if (ctx->r[0] & 1) bm.fix_fade = 0;
    }
    BM_LOG("track %X to %.2f in %d ms", ctx->r[0], (double)vol, (int)ctx->r[1]);
    CALL(RECOMP_ORIGINAL(0x003986C0));
}

/* ---- playBGM: the map BGM, or the boss theme in its place ---- */

static void bm_played(Context *ctx, const uint32_t *words) {
    bm_saw_play(ctx, words[0], words[1]);
}

RECOMP_OVERRIDE(0x00398C68) {
    uint32_t name = ctx->r[0], fade = ctx->r[1];
    uint32_t ts = bm_tsound(ctx);
    int map_bgm = ts && name == ts + 1;
    /* a BGM played on purpose: the hold is over (its own stop must run, or
       the old theme would go on beside the new one) */
    bm.hold = 0;
    char n[BM_NAME];
    if (bm_log_on()) bm_cstr(ctx, name, n);
    if (map_bgm && bm_active(ctx)) {
        const BmBoss *w = bm_wanted(ctx);
        if (w) {
            if ((bm.cur == CUR_BOSS || bm.cur == CUR_OTHER) && bm_cur_playing(ctx) && bm_same(ctx, bm.cur_name, w->name) &&
                !mem_read32(ctx, BM_BGM_FLAGS + 4)) {
                /* already playing (kept on, or the boss's battle theme as its
                   battle ends: OnFadeInMap plays the map BGM before
                   OnFadeOutBattle stops the battle's): only its volume back */
                if (bm.cur != CUR_BOSS) {
                    bm_saw_play(ctx, CUR_BOSS, w->name);
                    bossmusic_kept++;
                }
                bm.fix_fade = fade < 1000 ? 1000 : fade;
                BM_LOG("play %s: the boss theme plays on (volume back in %u ms)", n, bm.fix_fade);
                ctx->r[0] = 100;
                ctx->r[1] = bm.fix_fade;
                bm_tail(ctx, BM_BGM_VOLUME);
                return;
            }
            char b[BM_NAME];
            bm_cstr(ctx, w->name, b);
            ctx->r[0] = w->name;
            ctx->r[2] = bm_position(ctx, w->name);
            bossmusic_played++;
            BM_LOG("play %s -> boss theme %s at %u, fade %u", n, b, ctx->r[2], fade);
            CALL_THEN(RECOMP_ORIGINAL(0x00398C68), bm_played, CUR_BOSS, w->name);
            return;
        }
    }
    BM_LOG("play %s at %u, fade %u", n, ctx->r[2], fade);
    CALL_THEN(RECOMP_ORIGINAL(0x00398C68), bm_played, map_bgm ? CUR_MAP : CUR_OTHER, map_bgm ? 0 : name);
}

/* ---- Sound::BGMPlay / BGMStop: events' own music (logged; nothing changed) ---- */

static void bm_event_played(Context *ctx, const uint32_t *words) {
    (void)words;
    /* the same sound still (BGMPlay of what already plays does nothing): as it was */
    uint32_t s = bm_sound_now(ctx);
    if (s && s == bm.cur_sound && bm_sound_id(ctx, s) == bm.cur_id) return;
    bm_saw_play(ctx, CUR_OTHER, 0);
}

RECOMP_OVERRIDE(0x004201A0) {
    if (bm_log_on()) {
        char n[BM_NAME];
        bm_cstr(ctx, ctx->r[0], n);
        BM_LOG("Sound::BGMPlay %s fade %d", n, (int)ctx->r[1]);
    }
    CALL_THEN(RECOMP_ORIGINAL(0x004201A0), bm_event_played, 0);
}

RECOMP_OVERRIDE(0x00420244) {
    BM_LOG("Sound::BGMStop fade %d", (int)ctx->r[0]);
    CALL(RECOMP_ORIGINAL(0x00420244));
}

/* ---- ProcSoundMonitor::Persistent: the change when the boss is gone ---- */

/* a boss found again after a load: an enemy with a theme of its own, hurt */
static void bm_found(Context *ctx, uint32_t unit, uint32_t mhp) {
    uint32_t hp = mem_read8(ctx, unit + U_HP);
    uint32_t person = mem_read32(ctx, unit + U_PERSON);
    BM_LOG("boss check: unit %08X hp %u of %u", unit, hp, mhp);
    if (hp && hp < mhp && person)
        bm_engage(ctx, unit, mem_read32(ctx, person + P_BGM), "found again (hurt)");
}

/* the next enemy after `after` with a theme of its own (0: none left) */
static uint32_t bm_next_candidate(Context *ctx, uint32_t after) {
    uint32_t forces = mem_read32(ctx, BM_FORCES);
    if (!forces) return 0;
    uint32_t u = mem_read32(ctx, forces + 1 * 12);
    int passed = after == 0;
    for (int guard = 0; u && guard < 256; guard++, u = mem_read32(ctx, u + U_NEXT)) {
        if (!passed) { passed = u == after; continue; }
        uint32_t p = mem_read32(ctx, u + U_PERSON);
        if (p && mem_read32(ctx, p + P_BGM) && mem_read8(ctx, u + U_HP)) {
            int known = 0;
            for (uint32_t i = 0; i < bm.count; i++) known |= bm.bosses[i].unit == u;
            if (!known) return u;
        }
    }
    return 0;
}

static void bm_monitor_original(Context *ctx) {
    CALL(RECOMP_ORIGINAL(0x001EEF4C));
}

RECOMP_OVERRIDE(0x001EEF4C) {
    if (bm.call && bm.call_tls == ctx->tls) {
        /* GetMHPImpl came back here after leaving for the host */
        bm.call = 0;
        bm_found(ctx, bm.call_unit, ctx->r[0]);
        ctx->r[0] = bm.call_this;
        ctx->r[14] = bm.call_lr;
        bm_monitor_original(ctx);
        return;
    }
    uint32_t on = bm_enabled();
    if (on != bm.on) {
        bm.on = on;
        bm.derive = 0;
        bm.derive_unit = 0;
    }
    if (bm.in_battle || !mem_read32(ctx, BM_SITUATION)) {
        bm_monitor_original(ctx);
        return;
    }
    if (!on || !bm_active(ctx)) {
        /* switched off (or not to act here) while a boss theme of ours plays: the map BGM back */
        if (bm.cur == CUR_BOSS && bm_cur_playing(ctx) && mem_read8(ctx, BM_BGM_FLAGS) && !mem_read32(ctx, BM_BGM_FLAGS + 4)) {
            BM_LOG("boss music off: back to the map BGM");
            bm.cur = CUR_PENDING;
            ctx->r[0] = 2000;
            bm_tail(ctx, BM_MAP_PLAY);
            return;
        }
        bm_monitor_original(ctx);
        return;
    }
    if (bm.tsound != bm_tsound(ctx)) {
        memset(&bm.count, 0, sizeof bm - offsetof(BmState, count));
        bm.tsound = bm_tsound(ctx);
    }
    /* the bosses again after a load: one unit a tick */
    if (!bm.derive) {
        uint32_t u = bm_next_candidate(ctx, bm.derive_unit);
        if (!u) {
            bm.derive = 1;
        } else {
            bm.derive_unit = u;
            bm.call = 1;
            bm.call_tls = ctx->tls;
            bm.call_lr = ctx->r[14];
            bm.call_this = ctx->r[0];
            bm.call_unit = u;
            ctx->r[0] = u;
            ctx->r[1] = 1;
            ctx->r[2] = 0;
            ctx->r[3] = 0;
            ctx->r[14] = BM_MONITOR_TICK;
            ctx->r[15] = BM_GET_MHP;
            ctx->thumb = 0;
            ctx->budget -= 4;
            CALL(recomp_call);
            /* it came straight back */
            bm.call = 0;
            bm_found(ctx, u, ctx->r[0]);
            ctx->r[0] = bm.call_this;
            ctx->r[14] = bm.call_lr;
            bm_monitor_original(ctx);
            return;
        }
    }
    /* the change, out of battle, when the map BGM or a boss theme of ours plays */
    if ((bm.cur == CUR_MAP || bm.cur == CUR_BOSS) && bm_cur_playing(ctx) &&
        mem_read8(ctx, BM_BGM_FLAGS) && !mem_read32(ctx, BM_BGM_FLAGS + 4)) {
        const BmBoss *w = bm_wanted(ctx);
        uint32_t ts = bm_tsound(ctx);
        if (bm.cur == CUR_BOSS && !w) {
            BM_LOG("the boss is gone: back to the map BGM");
            bm.cur = CUR_PENDING;
            bossmusic_back++;
            ctx->r[0] = 2000;
            bm_tail(ctx, BM_MAP_PLAY);
            return;
        }
        if (w && (bm.cur == CUR_MAP || !bm_same(ctx, bm.cur_name, w->name)) && ts && mem_read8(ctx, ts)) {
            BM_LOG("a boss to play: its theme in the map BGM's place");
            bm.cur = CUR_PENDING;
            ctx->r[0] = ts + 1;
            ctx->r[1] = 2000;
            ctx->r[2] = 0;
            ctx->r[3] = 0;
            bm_tail(ctx, BM_PLAY);
            return;
        }
    }
    bm_monitor_original(ctx);
}
