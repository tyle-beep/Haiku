/* castlegifts: the gifts the 3DS only gave through online play and
 * StreetPass, given through the game's own menus (Options > Gameplay >
 * Online Rewards; the host sets castlegifts_on, qol.rs).
 *
 * 1. My Castle > Crystal Ball > Connect > Inbox > Rewards > Battle Bonuses
 *    and Visit Bonuses. GameData's AchieveBonus table (GameData::Initialize
 *    hands AchieveBonus::Initialize its +0x5C block) lists 80 rewards a
 *    kind, each a threshold and an item: True Ganglari at 400 Battle
 *    Points, the Battle Seal at 9999, Ryoma's Club at 50 Visit Points, the
 *    Visitation Seal at 9999... The points are the castle's own (CastleData
 *    +0x32C battle, +0x330 visit), raised only by the feedback other players'
 *    castle battles and visits left online or through StreetPass
 *    (castle::ProcLink::CheckMsgData), so offline they stay 0 and every
 *    entry stays locked. AchieveBonusMenuItem::GetType (0x0048B63C) is what
 *    every part of the list asks (BuildAttribute, Draw, ACall, OnSelect, the
 *    split window): 1 locked, 2 can be claimed, 3 claimed already. With the
 *    option on, GetType answers each Battle and Visit Bonus as it would with
 *    the points at their most (claimable, or claimed already), without
 *    touching the points: the save keeps the real ones. A claim is then the game's
 *    own ACall: unit::Item::New for the reward, Transporter::Add (or "The
 *    convoy is already full."), the reward's bit in the castle's
 *    FlagManagerNoName (+0x4CC battle, +0x4D0 visit; saved with the castle),
 *    and "<item> has been sent to the convoy." A reward claimed once answers
 *    3, "You have already received this item.", in every later visit.
 *
 * 2. Title > Extras > Presents. "Download any available presents?" reads
 *    the game's StreetPass box (nn::cec) for the two presents the game
 *    knows (link::ProcPresent::ProcUpdate): an Awakening StreetPass present
 *    (bit 1: the Emblem Shield accessory) and an event present (bit 2: the
 *    Dark Crown and the Hoshido Crown). PC has no StreetPass, so the box is
 *    empty and the game says "No new presents are available." ProcUpdate::
 *    End (0x003EE920) compares what was had (+0x94C) with what the box gave
 *    (+0x950), sets GameUserGlobalData's bits (0x80, 0x100) for the new
 *    ones and says "You've downloaded ... Pick it up from the in-game
 *    Accessory Shop."; the Accessory Shop then hands them over once per
 *    save (ProcAcceShop::AwakeAccePresent, BossAccePresent). With the
 *    option on, the box is answered as holding both presents.
 *
 * With the option off both overrides only call the originals. The menus
 * run nowhere else, so the game's timing is unchanged everywhere else.
 */
#include "overrides.h"

uint32_t castlegifts_on = 0;       /* the host: Online Rewards */
uint32_t castlegifts_unlocked = 0; /* GetType answers for a bonus, unlocked */
uint32_t castlegifts_presents = 0; /* presents the box gave */

#define CG_CASTLE 0x006D1998u  /* Castle::Get's instance */
#define CG_PRESENTS_HAD 0x94Cu  /* ProcUpdate: the presents had before */
#define CG_PRESENTS_NOW 0x950u  /* ProcUpdate: the presents the box gave */
#define CG_PRESENTS_ALL 3u      /* Awakening (1), event (2) */

/* Castle::GetData(), as GetType itself reaches it: the visited castle's
   data while visiting, the player's own otherwise */
static uint32_t cg_castle_data(Context *ctx) {
    uint32_t castle = mem_read32(ctx, CG_CASTLE);
    if (!castle)
        return 0;
    uint32_t mode = mem_read32(ctx, castle + 4);
    uint32_t holder = castle + 8;
    if (mode) {
        uint8_t kind = mem_read8(ctx, mode);
        if (kind == 1 || (kind == 2 && mem_read8(ctx, mode + 9) == 1))
            holder = castle + 0xC;
    }
    uint32_t list = mem_read32(ctx, holder);
    return list ? mem_read32(ctx, list) : 0;
}

#define CG_ACHIEVE 0x006D363Cu   /* AchieveBonus's four words: +4 the battle list, +8 the visit list */
#define CG_ITEM_MENU 0xCu         /* the item's menu */
#define CG_MENU_KIND 0x134u       /* the menu's kind: 0 battle, 1 visit, 2 path */
#define CG_ITEM_INDEX 0x38u       /* the item's row */
#define CG_FLAGS_BATTLE 0x4CCu    /* CastleData: the claimed Battle Bonuses */
#define CG_FLAGS_VISIT 0x4D0u     /* CastleData: the claimed Visit Bonuses */

/* castle::anonymous_namespace::AchieveBonusMenuItem::GetType(void)
   With the option on, a Battle or Visit Bonus answers as the original does
   with the points at their most: 3 when the castle's FlagManagerNoName has
   its bit (FlagManagerNoName::Get: bits at +4, byte index >> 3, bit
   index & 7), else 2. Nothing is written: the first version raised the
   points for the call and put them back in a CALL_THEN tail, and a test
   ended with the raised points still in the castle's data (a tail that
   didn't run would have saved them), so the answer is computed here. The
   Path Bonus (kind 2) and anything unexpected go to the original. */
RECOMP_OVERRIDE(0x0048B63C) {
    uint32_t item = ctx->r[0];
    uint32_t data = castlegifts_on ? cg_castle_data(ctx) : 0;
    uint32_t menu = data ? mem_read32(ctx, item + CG_ITEM_MENU) : 0;
    uint32_t kind = menu ? (mem_read32(ctx, menu + CG_MENU_KIND) & 0xFF) : 2;
    /* AchieveBonus::GetBattle / GetVisit: the {entries, count} pair */
    uint32_t list = kind > 1 ? 0 : mem_read32(ctx, CG_ACHIEVE + (kind == 0 ? 4 : 8));
    if (!list) {
        CALL(RECOMP_ORIGINAL(0x0048B63C));
        return;
    }
    uint32_t row = mem_read32(ctx, item + CG_ITEM_INDEX);
    uint32_t answer = 0;
    if (mem_read32(ctx, list) && (int32_t)row >= 0 && row < mem_read32(ctx, list + 4)) {
        uint32_t flags = mem_read32(ctx, data + (kind == 0 ? CG_FLAGS_BATTLE : CG_FLAGS_VISIT));
        uint32_t bits = flags ? mem_read32(ctx, flags + 4) : 0;
        uint32_t claimed = bits ? (mem_read8(ctx, bits + (row >> 3)) >> (row & 7)) & 1 : 0;
        answer = claimed ? 3 : 2;
        castlegifts_unlocked++;
    }
    ctx->r[0] = answer;
    ctx->budget -= 40; /* about what the original runs */
    RETURN_TO(ctx->r[14]);
}

/* link::ProcPresent::ProcUpdate::End(void) */
RECOMP_OVERRIDE(0x003EE920) {
    if (castlegifts_on) {
        uint32_t update = ctx->r[0];
        uint32_t had = mem_read32(ctx, update + CG_PRESENTS_HAD);
        uint32_t now = mem_read32(ctx, update + CG_PRESENTS_NOW);
        uint32_t add = CG_PRESENTS_ALL & ~(had | now);
        if (add) {
            mem_write32(ctx, update + CG_PRESENTS_NOW, now | add);
            castlegifts_presents += (add & 1) + ((add >> 1) & 1);
        }
    }
    CALL(RECOMP_ORIGINAL(0x003EE920));
}
