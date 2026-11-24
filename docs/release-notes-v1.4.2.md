# Haiku 1.4.2

Haiku 1.4.2 brings back the rewards the 3DS only gave online (True Ganglari, the Battle and Visitation Seals and the StreetPass presents), adds Boss Music Stays (#20) and a Variable Refresh option for G-SYNC and FreeSync displays (#1), makes the first-time code build much quicker on slow PCs and fixes it on Steam Deck / Proton (#13, #17), and fixes a hair mod's textures breaking after a movie (#16), mods whose `romfs` folder sits a folder down (#16) and L3 toggling the bottom screen twice.

Haiku is a native Windows port of Fire Emblem Fates. The game's code is statically recompiled to x86-64 and runs on native PC code for graphics (Vulkan), audio, files, saves and input. Nothing is emulated. See the [README](https://github.com/tyle-beep/Haiku#features) for the full feature list.

**This release contains no game files.** You need your own legally obtained copy of Fire Emblem Fates Special Edition (US, European or Japanese), dumped and decrypted by you. Haiku builds the game's code from it on your PC.

## Downloads

| File | Size | What it is |
|---|---|---|
| `Haiku.zip` | 48.4 MB (48,418,512 bytes) | Haiku: `Haiku.exe` and everything it needs to build the game's code from your dump. Extract it and run `Haiku.exe`. |
| `Haiku.zip.sig` | 64 bytes | The signature of `Haiku.zip`, which Haiku's in-game updater checks. You don't need it to install by hand. |
| `Haiku-toolchain-source-w64devkit-2.11.0.zip` | 152.2 MB (152,193,966 bytes) | The source code of the bundled compiler (GCC 16.2.0, GNU Binutils 2.47, GMP, MPFR, MPC and MinGW-w64 v14.0.0, as w64devkit 2.11.0 builds them), offered under the GNU GPL version 3. Unchanged from 1.0. You don't need it to play. |

SHA-256:

```
968a39491dc6536392aa9603ffab11738985f545301de72443330d977a7210e9  Haiku.zip
2c9acf323c86b948c6fec9cba11158ce011d8edd2ce8f55b357f1d730080a557  Haiku-toolchain-source-w64devkit-2.11.0.zip
```

## Updating from 1.4.1 or 1.4

**In the game:** when Haiku has checked for updates (once a day), the Main Menu shows "Haiku 1.4.2 is available". Press **U** on the keyboard or **Y** on the controller, or click it, then choose **Install**. Haiku downloads it, checks it, puts it in place and starts again by itself.

**By hand**, if you turned the update check off or don't want to wait for it:

1. Close Haiku.
2. Extract the new `Haiku.zip` over your Haiku folder, replacing the files.
3. Run `Haiku.exe`.

**The first start after updating builds the game's code once more**, because this release changes some of the game's functions (for Online Rewards and Boss Music Stays). Haiku does it by itself in its window, showing its progress and the time left, and then the game starts. This time most of the game's code is compiled again (222 of its 254 files), so it takes nearly as long as the first build did when you installed: about 5 minutes on our test PC, longer on a slower one. If you have more than one version of the game installed (US, European, Japanese), each one does this once, the first time you play it. Your saves, settings, mods and Japanese voices are kept as they are.

**From 1.3 or older:** those versions can't update themselves, so update by hand as above. From 1.2, 1.1 or 1.0 the first start also removes the old `Haiku Setup.exe`, as the 1.3 notes describe.

## What's new

### Online Rewards

The 3DS gave some items only to players who battled and visited other players' castles online, and a few of them could be had no other way: **True Ganglari** (the "Ganglari" with 16 Mt), the **Battle Seal** and the **Visitation Seal**. With no online play left, they were out of reach.

- **Options > Gameplay > Online Rewards** (Unlocked by default) opens them up, through the game's own menus. Nothing changes until you choose to claim something.
- **My Castle: Crystal Ball > Connect > Inbox > Rewards:** every Battle Bonus and Visit Bonus can be claimed once, and each one is sent to your convoy, with the game's own message. A reward you already claimed stays claimed in that save.
- **Title screen > Extras > Presents** now finds the two StreetPass presents: the Emblem Shield, the Dark Crown and the Hoshido Crown. Pick them up at any castle's Accessory Shop, as on the 3DS.
- Most other Bonus items, Ryoma's Club among them, can also be found without going online, in shops or as gifts from your army in My Castle. The Inbox just saves you the wait.
- Setting the option to **Game's Own** locks the rewards you haven't claimed again; what you claimed stays yours. Saves stay compatible with the 3DS.
- **New cheat: Cheats > Items & Resources > Add an Item** puts any item the game has into your convoy, one per press (up to 99). Items that came from the Inbox are marked "Battle Bonus" or "Visit Bonus", which tells the two Ganglaris apart.

### Boss Music Stays (#20)

- **Options > Audio > Boss Music Stays** (off by default): once you fight a boss, its battle theme keeps playing as the map's music until that boss is defeated, retreats or joins you, as in Three Houses and Engage. Then the chapter's own music fades back in.
- It only changes which of the game's own tracks plays. Online battles and the last chapters are left as they are.

### Variable Refresh, for G-SYNC and FreeSync displays (#1)

- **Options > Display > Variable Refresh** (off by default). On, Haiku keeps the frame rate just under your display's refresh rate (162 FPS on a 170 Hz display) and paces every picture on its own steady clock. This fixes blurry, uneven motion when scrolling the map at Frame Rate "Display" (or 165 and up) on a variable-refresh display.
- Leave it off on a display with a fixed refresh rate: there it would repeat a picture now and then.

### HD movies for mods (#7)

- **Each picture of a mod's HD movie costs less to draw**: Haiku no longer looks for it in texture packs, and decodes it on several threads.
- **`haiku.log` gets a line after each HD movie** saying how fast it really played (pictures, seconds, pictures a second, the longest wait). If a movie plays slowly on your PC, that line in an issue tells us where to look.
- The game's own movies look and play exactly as before.

## Fixes

- **The first-time build of the game's code is much quicker on slow PCs, and no longer fails on Steam Deck / Proton** (#13, #17). One enormous function, which runs only once when the game starts, took the compiler minutes (an hour on a slow Celeron); it is now compiled with lighter optimisation, so it takes seconds and the game plays the same. A PC with 1 or 2 cores now compiles that many files at once, and the time left is shown after the first few files.
- **Under Wine and Proton** (Steam Deck), Haiku no longer stops the compiler for "doing nothing" (Wine doesn't report its processor time, which made Haiku think the compiler was stuck), and no longer suggests antivirus software there (#17).
- **A build that had to try a file again could stop moving and ignore Cancel** (#17); fixed. A compiler that won't start can no longer hold the build either, so **Cancel always stops it**.
- **A hair mod's textures (and other portraits and model textures) could break after a movie had played** (#16, new in 1.4.1). Textures the game put where a movie's pictures had been were handled like movie pictures for the rest of the session. Only a movie's own pictures are handled that way now.
- **A mod whose `romfs` folder sits a folder or two down, beside other folders, now loads its game files too**, not only its HD pictures (#16). For example Selena's hair as downloaded, with `Mod/romfs` beside `HD Textures (Citra)`. A mod with several such folders, which Haiku can't choose between, now says so in the Mods screen.
- **L3 (or M on the keyboard) shows or hides the bottom screen once per press.** A firm click used to show it on the press and hide it again on the release. In the On Demand mode, holding it still peeks.

## Known issues

- **Variable Refresh isn't turned on by itself**: Windows doesn't tell programs whether a display is using G-SYNC or FreeSync, so turn it on yourself. We couldn't test it on a variable-refresh display; if motion still isn't even with it on, please say so in #1.
- **Boss Music Stays:** after loading a save or a suspend, a boss you had fought but who is back to full HP isn't remembered; its music comes back the next time you fight it.
- **Online Rewards, not checked in game:** picking up the Presents' accessories at the Accessory Shop (the game's own step, which none of our test saves could reach). Other players' gifts (Inbox > Accessories), bond units and castle-battle loot are not covered: those came from other players.
- **On the update screen, with the Japanese game and Haiku's text in English,** a step's name ("Writing the game's code as C") can run into its "Done". It shows for a few seconds after updating; the build itself is fine.
- **On a very slow PC the first-time build still takes a long time** (about 4 to 5 hours on a 2-core Celeron, down from 8 to 11). It only happens once, and Haiku carries on from where it was if you close it. The Steam Deck / Proton fix was tested on Windows by simulating what the logs in #17 show; please report how it goes there.
- **Movies over 1024 pixels wide or high (1080p) can't be played.** The game's own movie decoder stops at 1024; such a movie is skipped. Use 1024x576 at most for 16:9.
- **16:9 movies are squashed slightly**: the game draws every movie over the whole 5:3 top screen. For 16:9 footage, letterbox it into **960x576** (exactly 5:3).
- **Picking a mod's other code patch** can only be done in `games\<version>\mods.toml` (for example `code_patch = 1` in the mod's entry). The Mods screen has no control for it yet.
- **Not checked in game:** Nyx Veil Fix's veil, Unofficial Gay Fates' paralogues and endings, and Playable Lilith's Lilith herself (she joins late in the story). These mods load, but we didn't reach those parts.
- **Haiku's compiler isn't offered for code mods** if your Haiku folder's path has letters outside plain English (A to Z). Zig still works there.
- With the Uncensored Patch, a save state taken during its subtitled opening movie doesn't resume. Save before or after the movie.
- Under a translation, the same-gender S-supports use general rules rather than hand-checked edits, so a line may keep a word for the wrong gender.
- Team If's translation has been tested less than the Uncensored Patch.
- The Steam overlay itself couldn't be tested for #14. If a menu still stutters with an overlay on, please attach `userdata\haiku\haiku.log` to the issue.
- If the first-time build still seems stuck on your PC (#13, #17), please attach `setup-work\setup.log` from your Haiku folder to the issue.
- Still being looked into: #3 (stuck on the Initializing screen) and #18 (Revelation Chapter 9's wind staying on screen: not reproduced yet; a save from just before it would help).

## Notes

- Requirements: Windows 10 or 11 (64-bit), a GPU with Vulkan, and about 3 GB of disk space for each version you install.
- Haiku only goes online for online battles, for the update check (which can be turned off in Options > Interface > Check for Updates), or if you choose to download Zig for code mods. Haiku's own compiler needs no download, and Haiku Setup never uses the network. Online Rewards need no connection: everything comes from your own game.
- Claimed Online Rewards don't mark your units as cheated for online battles: every player who played online had them on the 3DS. Add an Item is a cheat, but like Fill the Convoy it only fills the convoy and marks no unit.

## Legal

Haiku is an unofficial fan project, not affiliated with or endorsed by Nintendo or Intelligent Systems. Fire Emblem and Fire Emblem Fates are trademarks of Nintendo / Intelligent Systems. Haiku is MIT-licensed and provided as is, without warranty. The bundled compiler is GPL-3.0; its source is the third download above. Fan translations and other mods belong to their authors and are not part of Haiku.
