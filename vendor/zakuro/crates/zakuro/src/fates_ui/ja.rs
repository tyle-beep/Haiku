//! jpprep: Haiku's own menus in Japanese, on the Japanese game (Fire Emblem
//! if, zakuro_core::region JPN), unless the player chose English for them
//! (Options > Interface > Menu Language: an English translation mod).
//!
//! The menus are written in English (options.rs, pulse.rs, states.rs,
//! mods.rs...); the kit (kit.rs) passes every text it measures, wraps or
//! draws through [`tr`], which gives the Japanese for it when there is one.
//! A text with values in it (`Slot 3`, `Turn 2 · Player Phase`) matches an
//! entry with `{}` where the values are, and they go to the same places in
//! the Japanese (in order). On the USA and EUR games, or with English
//! chosen, [`tr`] gives the text back as it is (one relaxed load).
//!
//! The Japanese follows the game's own terms where it has them (its menus'
//! ON/OFF, セーブ, ロード, ターン, 自軍フェイズ, マイキャッスル, 支援, Sランク,
//! カムイ, カンナ; Divine Pulse is 天刻の拍動, as Three Houses names it), and
//! the polite form (です/ます) the game speaks to the player in. The font
//! is the Japanese game's own menu font (System_L), made from the player's
//! dump at install (Haiku Setup, jpn::make_font); [`wanted`] says which of
//! its 7,089 letters the menus' atlas keeps.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// the player's choice: Haiku's menus in English on the Japanese game.
pub fn set_english(english: bool) {
    ENGLISH.store(english, Ordering::Relaxed);
}

/// whether Haiku's menus are in Japanese now. HAIKU_MENUS_JAPANESE=1 makes
/// them Japanese on any build, for looking at them (tests, snapshots).
#[inline]
pub fn active() -> bool {
    // jpn2: English under an English translation of the script
    (zakuro_core::region::is_jpn() || forced()) && !ENGLISH.load(Ordering::Relaxed) && !crate::jpn_english::active()
}

fn forced() -> bool {
    static FORCED: OnceLock<bool> = OnceLock::new();
    *FORCED.get_or_init(|| std::env::var("HAIKU_MENUS_JAPANESE").is_ok_and(|v| v == "1"))
}

/// the Japanese for one of the menus' texts, or the text as it is.
#[inline]
pub fn tr(text: &str) -> Cow<'_, str> {
    if !active() {
        return Cow::Borrowed(text);
    }
    translate(text).map_or(Cow::Borrowed(text), Cow::Owned)
}

/// the table: exact texts, and the ones with values (`{}`), English to
/// Japanese.
struct Table {
    exact: HashMap<&'static str, &'static str>,
    patterns: Vec<(Vec<&'static str>, Vec<&'static str>)>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut exact = HashMap::new();
        let mut patterns = Vec::new();
        for &(en, ja) in TEXTS {
            if en.contains("{}") {
                patterns.push((en.split("{}").collect(), ja.split("{}").collect()));
            } else {
                exact.insert(en, ja);
            }
        }
        // the longest patterns first, so a specific one wins
        patterns.sort_by_key(|(pieces, _): &(Vec<&str>, Vec<&str>)| std::cmp::Reverse(pieces.iter().map(|p| p.len()).sum::<usize>()));
        Table { exact, patterns }
    })
}

/// the Japanese for text, when the table has it.
pub fn translate(text: &str) -> Option<String> {
    let t = table();
    if let Some(ja) = t.exact.get(text) {
        return Some((*ja).to_owned());
    }
    for (en, ja) in &t.patterns {
        if let Some(values) = matches(text, en) {
            let mut out = String::new();
            for (i, piece) in ja.iter().enumerate() {
                out.push_str(piece);
                if i + 1 < ja.len() {
                    // a value that is itself one of the texts (a phase's name) too
                    let value = values.get(i).map(String::as_str).unwrap_or("");
                    out.push_str(&translate(value).unwrap_or_else(|| value.to_owned()));
                }
            }
            return Some(out);
        }
    }
    None
}

/// the values text has where pattern has `{}` (pattern split there).
fn matches(text: &str, pieces: &[&str]) -> Option<Vec<String>> {
    let first = pieces.first()?;
    let mut rest = text.strip_prefix(first)?;
    let mut values = Vec::new();
    for (i, piece) in pieces.iter().enumerate().skip(1) {
        let last = i + 1 == pieces.len();
        let at = if last {
            if !rest.ends_with(piece) {
                return None;
            }
            rest.len() - piece.len()
        } else if piece.is_empty() {
            return None;
        } else {
            rest.find(piece)?
        };
        if at == 0 {
            return None;
        }
        values.push(rest[..at].to_owned());
        rest = &rest[at + piece.len()..];
    }
    rest.is_empty().then_some(values)
}

/// whether text has Japanese in it (it wraps between any two letters).
pub fn is_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{3000}'..='\u{9FFF}').contains(&c) || ('\u{FF00}'..='\u{FFEF}').contains(&c))
}

/// a letter a line never starts with (Japanese closing punctuation, small
/// kana that end a sound).
pub fn no_break_before(c: char) -> bool {
    "、。，．・：；？！）」』】〉》〕ー〜ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ…".contains(c)
}

/// whether the menus' font atlas keeps a letter of a large (Japanese) font:
/// printable ASCII and Latin-1, Japanese punctuation, kana, full-width
/// forms, and every letter of the Japanese texts here.
pub fn wanted(c: char) -> bool {
    static KANJI: OnceLock<std::collections::HashSet<char>> = OnceLock::new();
    let u = c as u32;
    if (0x20..0x7F).contains(&u) || (0xA0..0x100).contains(&u) || (0x3000..0x3100).contains(&u) || (0xFF00..0xFFF0).contains(&u) {
        return true;
    }
    if matches!(c, '·' | '×' | '∞' | '—' | '…' | '→' | '←' | '↑' | '↓' | '○' | '●' | '☆' | '★') {
        return true;
    }
    KANJI.get_or_init(|| TEXTS.iter().flat_map(|(_, ja)| ja.chars()).filter(|c| *c as u32 >= 0x3000).collect()).contains(&c)
}

/// English -> Japanese. `{}` marks a value (a number, a name, a key).
const TEXTS: &[(&str, &str)] = &[
    // the tabs and common words
    ("Options", "PC設定"),
    ("Display", "画面"),
    ("Bottom Screen", "下画面"),
    ("Audio", "サウンド"),
    ("Controls", "操作"),
    ("Interface", "表示"),
    ("Mods", "MOD"),
    ("Gameplay", "ゲーム"),
    ("Cheats", "チート"),
    ("On", "ON"),
    ("Off", "OFF"),
    ("Close", "閉じる"),
    ("Back", "戻る"),
    ("Select", "選ぶ"),
    ("Enter", "決定"),
    ("Yes", "はい"),
    ("No", "いいえ"),
    ("OK", "OK"),
    ("Cancel", "キャンセル"),
    ("Default", "標準"),
    ("Auto", "自動"),
    ("Show", "表示する"),
    ("Hide", "表示しない"),
    ("None", "なし"),
    ("Press again", "もう一度押すと実行"),
    ("Restore Defaults", "初期設定に戻す"),
    // Display
    ("Desktop", "デスクトップ"),
    ("Windowed", "ウィンドウ"),
    ("Borderless", "ボーダーレス"),
    ("Fullscreen", "フルスクリーン"),
    ("Native (5:3)", "元の比率 (5:3)"),
    ("Artwork", "アートワーク"),
    ("Extend", "画面を延長"),
    ("Black", "黒"),
    ("Window Mode", "ウィンドウモード"),
    ("Play in a window, a borderless window covering the display, or exclusive fullscreen.", "ウィンドウ、画面全体を覆う枠なしウィンドウ、フルスクリーンから選びます。"),
    ("Resolution", "解像度"),
    ("The window's size, or the display mode in fullscreen.", "ウィンドウの大きさ、またはフルスクリーンでの表示モードです。"),
    ("Monitor", "モニター"),
    ("Display {}", "ディスプレイ {}"),
    ("Which display to play on in fullscreen.", "フルスクリーンで使うディスプレイです。"),
    ("Render Scale", "描画倍率"),
    ("How many times the 3DS's resolution the game is drawn at. Applies when the game next starts.", "3DSの解像度の何倍で描画するかです。次にゲームを起動したときに反映されます。"),
    ("Renderer", "描画方式"),
    ("Native", "ネイティブ"),
    ("Compatibility", "互換"),
    ("Native draws with the PC's GPU, Compatibility as the 3DS did. Applies when the game next starts.", "ネイティブはPCのGPUで、互換は3DSと同じ方法で描画します。次にゲームを起動したときに反映されます。"),
    ("Aspect Ratio", "アスペクト比"),
    ("The shape of the top screen. Wider shows more of the battlefield, and the menus keep their shape. Auto fills the window.", "上画面の形です。横に広いほど戦場が広く見え、メニューの形は変わりません。自動ではウィンドウいっぱいに表示します。"),
    ("Ultrawide Sides", "ウルトラワイドの両端"),
    ("Beside menus, talks and other 5:3 screens: the game's own frame art, the picture carried on, or black.", "メニューや会話など5:3の画面の両側に、ゲームの枠のアート、延長した画面、黒のどれを表示するかです。"),
    ("VSync", "垂直同期"),
    ("Wait for the display between frames, so the picture never tears.", "画面の更新を待って描画し、映像のずれを防ぎます。"),
    ("Frame Rate", "フレームレート"),
    ("How many frames a second are shown. Display matches its refresh rate, or with VSync off and no cap, as many as the PC can draw.", "1秒間に表示するフレーム数です。「画面」はディスプレイのリフレッシュレートに合わせます。垂直同期OFFで上限なしなら、PCが描ける限り表示します。"),
    ("FPS Cap", "FPS上限"),
    ("The most frames a second shown, whatever else is set.", "ほかの設定にかかわらず、1秒間に表示するフレーム数の上限です。"),
    // vrrcap
    ("Variable Refresh", "可変リフレッシュレート"),
    ("For G-SYNC and FreeSync displays: the frame rate stays just under the refresh rate (162 FPS at 170 Hz), so motion stays even. Off for fixed refresh.", "G-SYNCやFreeSyncのディスプレイ用です。フレームレートをリフレッシュレートより少し低く保ち(170Hzで162FPS)、動きを均一にします。固定リフレッシュレートではOFFにします。"),
    ("Filtering", "フィルター"),
    ("How the picture is scaled to the window: smooth, sharp, or pixels.", "映像をウィンドウに合わせて拡大する方法です。なめらか、くっきり、ドットから選びます。"),
    // hfr60
    ("Smooth 30 FPS Scenes", "30FPSシーンの補間"),
    ("At 60 FPS, a frame is drawn between each two of the battles', 3D cutscenes' and title screen's, which the game runs at 30. Above 60 FPS every scene is smoothed.", "60FPSのとき、30FPSで動く戦闘・3Dイベントシーン・タイトル画面に補間フレームを加えます。60FPSを超えるときは常に補間します。"),
    ("Anti-Aliasing", "アンチエイリアス"),
    ("Full Draw Distance", "遠景も描画"),
    ("Grass and scenery are drawn however far away they are, at full detail, so nothing pops in as the camera nears it. Off, the 3DS's own distances.", "草や景色を遠くまで細かく描画し、カメラが近づいても急に現れないようにします。OFFでは3DSと同じ距離です。"),
    ("Battle Transition Blur", "戦闘切り替えのブラー"),
    ("The zoom blur as the map gives way to a fight and back. Off, the zoom is drawn at the display's frame rate.", "マップから戦闘に切り替わるときのズームブラーです。OFFではズームを画面のフレームレートで描画します。"),
    ("Show FPS", "FPSを表示"),
    ("Show the frame rate in the corner of the screen.", "画面の隅にフレームレートを表示します。"),
    // Bottom screen
    ("Display Mode", "表示方法"),
    ("The touch screen slides in when the game shows something on it, and away after. M or L3 shows or hides it.", "ゲームが下画面に何か表示したときに下画面が現れ、終わると隠れます。MかL3で表示を切り替えます。"),
    ("The touch screen always shows over the game. Drag its frame to move it, or a corner to size it.", "下画面をいつもゲームの上に表示します。枠をドラッグで移動、角をドラッグで大きさを変えられます。"),
    ("The touch screen sits beside the game, in the room a wide display leaves.", "横長の画面の空いた場所に、下画面をゲームの横に並べます。"),
    ("The touch screen slides in only while M or L3 is held, or after a tap of it.", "MかL3を押している間、または押したあとだけ下画面が現れます。"),
    ("Both screens as on the 3DS. F9 changes their arrangement.", "3DSと同じく2画面を表示します。F9で並べ方を変えます。"),
    ("Side", "位置"),
    ("Left", "左"),
    ("Right", "右"),
    ("Which side of the game the touch screen goes on.", "下画面をゲームのどちら側に置くかです。"),
    ("Size", "大きさ"),
    ("The touch screen's height next to the game's.", "ゲーム画面に対する下画面の高さです。"),
    ("Opacity", "不透明度"),
    ("How solid the touch screen is when drawn over the game.", "ゲームの上に重ねるときの下画面の濃さです。"),
    ("Position", "表示位置"),
    ("The corner or edge of the window the touch screen keeps to.", "下画面を置くウィンドウの角または辺です。"),
    ("The touch screen's height, as a share of the window's.", "ウィンドウの高さに対する下画面の高さです。"),
    ("Across", "横の位置"),
    ("How far the touch screen sits in from the side it keeps to.", "置いた辺から下画面までの横の距離です。"),
    ("Up and Down", "縦の位置"),
    ("How far the touch screen sits in from the top or bottom it keeps to.", "上端または下端から下画面までの距離です。"),
    ("Hide After", "隠れるまで"),
    ("How long the touch screen stays once the game stops showing anything on it.", "ゲームが下画面の表示を終えてから、下画面が隠れるまでの時間です。"),
    ("Unit Info", "ユニット情報"),
    ("Show a unit's details when the cursor rests on it, and in the unit list.", "カーソルを合わせたユニットとユニット一覧で、詳しい情報を表示します。"),
    ("Attack Forecast", "戦闘予測"),
    ("Show the forecast while choosing whom to attack, heal or trade with.", "攻撃、回復、交換の相手を選ぶときに予測を表示します。"),
    ("Combat", "戦闘"),
    ("Show the touch screen through a battle, up to the EXP bar and level-up.", "戦闘中、経験値とレベルアップまで下画面を表示します。"),
    ("Tutorials and Setup", "チュートリアルと設定画面"),
    ("Show tutorial pages and choices made on the touch screen, like your avatar's build.", "チュートリアルや、マイユニットの作成など下画面で選ぶ場面を表示します。"),
    ("Show After", "表示まで"),
    ("How long the cursor rests on a unit before its details come out.", "カーソルを合わせてから情報が出るまでの時間です。"),
    // Audio
    ("Master Volume", "全体の音量"),
    ("The volume of everything the game plays.", "ゲームのすべての音の音量です。"),
    ("Mute", "消音"),
    ("Silence the game.", "ゲームの音を消します。"),
    ("System Default", "システムの既定"),
    ("Output Device", "出力デバイス"),
    ("The speakers or headphones the sound plays on.", "音を鳴らすスピーカーまたはヘッドホンです。"),
    ("Mute in Background", "非アクティブ時に消音"),
    ("Silence the game while another window is in front.", "ほかのウィンドウが前にある間、ゲームの音を消します。"),
    ("Voices", "ボイス"),
    ("English", "English"),
    ("Japanese", "日本語"),
    ("Restart with These Voices", "このボイスで再起動"),
    ("Restart the game so the voices change. Anything you haven't saved will be lost.", "ボイスを変えるためにゲームを再起動します。セーブしていない進行状況は失われます。"),
    // Controls
    ("Button Layout", "ボタン配置"),
    ("Nintendo (A right)", "任天堂 (Aが右)"),
    ("Xbox (A bottom)", "Xbox (Aが下)"),
    ("Nintendo", "任天堂"),
    ("Xbox", "Xbox"),
    ("PlayStation", "PlayStation"),
    ("Keyboard", "キーボード"),
    ("Which face button confirms. Auto follows the controller connected.", "決定に使うボタンです。自動では接続したコントローラーに合わせます。"),
    ("Button Icons", "ボタンの表示"),
    ("Which button pictures the menus show.", "メニューに表示するボタンの絵です。"),
    ("Stick Dead Zone", "スティックの遊び"),
    ("How far a stick has to move before it counts.", "スティックをどれだけ倒すと入力になるかです。"),
    ("Rumble", "振動"),
    ("Let the controller vibrate.", "コントローラーを振動させます。"),
    ("Touch Cursor Speed", "タッチカーソルの速さ"),
    ("How fast the right stick moves the touch cursor.", "右スティックで動かすタッチカーソルの速さです。"),
    ("Battle Free Camera", "戦闘フリーカメラ"),
    ("Move the camera freely in battles and My Castle. {} switches it, {} gives the game's camera back.", "戦闘とマイキャッスルでカメラを自由に動かします。{}で切り替え、{}でゲームのカメラに戻します。"),
    ("Free camera", "フリーカメラ"),
    ("Camera reset", "カメラを戻す"),
    ("Free Camera Style", "フリーカメラの操作"),
    ("Orbit", "回り込み"),
    ("Fly", "飛行"),
    ("Turn around the fight with the right stick or a drag; the wheel zooms.", "右スティックかドラッグで戦闘の周りを回り、ホイールでズームします。"),
    ("Left stick or WASD moves, L/R or Q/E rise and sink, right stick or a drag looks, the wheel goes ahead.", "左スティックかWASDで移動、L/RかQ/Eで上下、右スティックかドラッグで向きを変え、ホイールで前に進みます。"),
    ("Walk in Castle View", "城の近景で歩く"),
    ("In My Castle's close-up view (L), walk with the left stick, turn the camera with the right stick or a drag, zoom with the wheel.", "マイキャッスルの近景(L)で、左スティックで歩き、右スティックかドラッグでカメラを回し、ホイールでズームします。"),
    ("Keyboard Controls", "キーボードの操作"),
    ("Choose the keys for each of the 3DS's buttons.", "3DSの各ボタンに使うキーを選びます。"),
    ("Controller Controls", "コントローラーの操作"),
    ("Choose the controller buttons for each of the 3DS's buttons.", "3DSの各ボタンに使うコントローラーのボタンを選びます。"),
    ("Put every key back as it was.", "すべてのキーを初期設定に戻します。"),
    ("Put every controller button back as it was.", "すべてのコントローラーのボタンを初期設定に戻します。"),
    ("Put the keys, buttons and controller options back as they were.", "キー、ボタン、コントローラーの設定を初期設定に戻します。"),
    ("Press a key", "キーを押してください"),
    ("Press a button", "ボタンを押してください"),
    ("Press, then press the key to use. Esc cancels.", "決定してから、使うキーを押してください。Escでキャンセルします。"),
    ("Press, then press the controller button to use. Esc cancels.", "決定してから、使うコントローラーのボタンを押してください。Escでキャンセルします。"),
    ("The key that opens and closes these options. Press, then press the key; it leaves any other binding.", "このPC設定を開閉するキーです。決定してからキーを押してください。ほかの割り当ては外れます。"),
    ("The button that opens and closes these options (Start and Select together do too). It leaves any other binding. Esc cancels.", "このPC設定を開閉するボタンです(STARTとSELECTの同時押しでも開閉します)。ほかの割り当ては外れます。Escでキャンセルします。"),
    // Interface
    ("Menu Size", "メニューの大きさ"),
    ("The size of these menus.", "このメニューの大きさです。"),
    ("Menu Language", "メニューの言語"),
    ("The language of Haiku's own menus. The game's text is Japanese (or a translation mod's).", "Haikuのメニューの言語です。ゲームの文章は日本語(または翻訳MODの言語)のままです。"),
    ("Language", "言語"),
    ("The game's language. It changes the next time the game starts.", "ゲームの言語です。次にゲームを起動したときに変わります。"),
    ("Boon/Bane Stats", "得意・苦手の能力値"),
    ("When choosing your avatar's boon and bane, show what they do to each stat.", "マイユニットの得意と苦手を選ぶとき、各能力値への影響を表示します。"),
    // Gameplay
    ("Game Speed", "ゲーム速度"),
    ("Fast Speed", "早送りの速度"),
    ("{} switches between 1\u{d7} and Fast Speed. Above 4\u{d7} the sound is muted.", "{}で1\u{d7}と早送りの速度を切り替えます。4\u{d7}を超えると音は消えます。"), // jpn
    ("Fast Forward", "早送り"),
    ("Skip Story", "ストーリーをスキップ"),
    ("Skip cutscenes, movies and the talk before battles. Talk during battles, boss quotes and choices still show.", "イベントシーン、ムービー、戦闘前の会話を飛ばします。戦闘中の会話、ボスのセリフ、選択肢は表示されます。"),
    ("Enemy Targeting Lines", "敵の攻撃目標ライン"),
    ("On the battle map, show a line from each enemy to the unit it will attack next enemy phase, as the enemy's own AI decides. The lines follow a unit while you move it.", "戦闘マップで、次の敵軍フェイズに各敵が攻撃するユニットへ線を引きます(敵の思考どおり)。ユニットを動かすと線も追いかけます。"),
    ("Growth Rates Button", "成長率ボタン"),
    ("A button on the lower screen's unit status that shows the unit's growth rates. G or the bound controller button does the same.", "下画面のユニット詳細に、成長率を表示するボタンを置きます。Gキーか割り当てたボタンでも表示します。"),
    ("Growths", "成長率"),
    ("Corrin's S Supports", "カムイのSランク"),
    ("Anyone", "誰とでも"),
    ("Game's Own", "ゲームどおり"),
    ("Corrin can reach S rank and marry anyone Corrin has a support with, of either gender. Takes effect the next time the game starts.", "カムイは支援のある相手なら性別を問わず、Sランクになり結婚できます。次にゲームを起動したときに反映されます。"),
    ("Corrin's S ranks are the game's own. Takes effect the next time the game starts.", "カムイのSランクはゲームどおりです。次にゲームを起動したときに反映されます。"),
    // castlegifts: Online Rewards
    ("Online Rewards", "通信の特典"),
    ("Unlocked", "受け取れる"),
    ("Claim each Battle and Visit Bonus once in My Castle's Inbox > Rewards, and get the StreetPass presents from Extras > Presents on the title screen.", "マイキャッスルの受信ボックス > 実績ボーナスで、戦闘実績と訪問実績のボーナスをそれぞれ1回ずつ受け取れます。タイトル画面のプレゼントで、すれちがいプレゼントも受信できます。"),
    ("The Battle and Visit Bonuses and the StreetPass presents need online play and StreetPass, so they stay locked.", "戦闘実績・訪問実績のボーナスとすれちがいプレゼントは、通信とすれちがい通信が必要なため受け取れません。"),
    ("Kana's Gender", "カンナの性別"),
    ("Ask", "毎回選ぶ"),
    ("Male", "男性"),
    ("Female", "女性"),
    ("Random", "ランダム"),
    ("When Kana's paralogue starts, choose Kana's gender: male, female or random.", "カンナの外伝が始まるときに、カンナの性別(男性、女性、ランダム)を選びます。"),
    ("As in the game: a male Corrin has a daughter, a female Corrin a son.", "ゲームどおり、カムイが男性なら娘、女性なら息子になります。"),
    ("Kana is a son or a daughter at random, decided when Kana's paralogue starts.", "カンナの外伝が始まるときに、息子か娘かをランダムで決めます。"),
    ("Kana's gender, whatever Corrin's.", "カムイの性別にかかわらず、カンナはこの性別になります。"),
    ("Exit to Desktop", "ゲームを終了"),
    ("Close the game and return to the desktop. Anything you haven't saved will be lost.", "ゲームを終了してデスクトップに戻ります。セーブしていない進行状況は失われます。"),
    ("Return to Title", "タイトルに戻る"),
    ("Restart the game from its opening, as the game's own soft reset does. Anything you haven't saved will be lost.", "ゲームのソフトリセットと同じく、最初からやり直します。セーブしていない進行状況は失われます。"),
    ("Exit to the desktop? Anything you haven't saved will be lost.", "ゲームを終了しますか？ セーブしていない進行状況は失われます。"),
    ("Return to the title screen? Anything you haven't saved will be lost.", "タイトル画面に戻りますか？ セーブしていない進行状況は失われます。"),
    ("Restart the game with the voices as set? Anything you haven't saved will be lost.", "このボイスでゲームを再起動しますか？ セーブしていない進行状況は失われます。"),
    ("Restart the game with the mods as set? Anything you haven't saved will be lost.", "このMODの設定でゲームを再起動しますか？ セーブしていない進行状況は失われます。"),
    ("The game already runs with these mods. Restart it anyway? Anything you haven't saved will be lost.", "すでにこのMODで起動しています。それでも再起動しますか？ セーブしていない進行状況は失われます。"),
    // the status and its growth view
    ("Boon", "得意"),
    ("Bane", "苦手"),
    ("Base", "基本"),
    ("Growth", "成長率"),
    ("Max", "上限"),
    ("Free Camera Off", "フリーカメラOFF"),
    ("Free Camera: {} to go back", "フリーカメラ: {}で戻る"),
    ("The free camera works while you walk around the castle", "フリーカメラは城を歩いているときに使えます"),
    ("{} FPS", "{} FPS"),
    // Divine Pulse
    ("Divine Pulse", "天刻の拍動"),
    ("Now", "現在"),
    ("Present", "現在"),
    ("Phase", "フェイズ"),
    ("Turn Back", "時を戻す"),
    ("Accept Defeat", "敗北を受け入れる"),
    ("Choose a moment to go back to.", "戻る時点を選んでください。"),
    ("No charges of Divine Pulse are left on this map.", "このマップでは天刻の拍動の残り回数がありません。"),
    ("Your army is about to fall. Turn back time?", "軍が敗れようとしています。時を戻しますか？"),
    ("Accept defeat?", "敗北を受け入れますか？"),
    ("The battle will be lost.", "この戦いは敗北となります。"),
    ("Divine Pulse has no limit here.", "このマップでは天刻の拍動を何度でも使えます。"),
    ("Turn back time to this moment?", "この時点まで時を戻しますか？"),
    ("Charges {}/{}", "残り {}/{}"),
    ("Charges \u{221e}", "残り \u{221e}"),
    ("Turn {} \u{b7} {} Phase", "ターン{} \u{b7} {}フェイズ"),
    ("Now \u{b7} Turn {} \u{b7} {} Phase", "現在 \u{b7} ターン{} \u{b7} {}フェイズ"),
    ("Turn {}", "ターン{}"),
    ("T{}", "T{}"),
    ("Player", "自軍"),
    ("Enemy", "敵軍"),
    ("Other", "他軍"),
    ("Turn back time on a battle map to undo actions, as in Three Houses and Engage. Open it from the map's menu or with {}; it also offers itself before a defeat.", "戦闘マップで時を戻して行動をやり直せます(風花雪月やエンゲージと同じ)。マップのメニューか{}で開き、敗北の前にも使えます。"),
    ("Divine Pulse Charges", "天刻の拍動の回数"),
    ("Story", "ストーリー"),
    ("Fixed", "固定"),
    ("Unlimited", "無制限"),
    ("As in Three Houses: 3 at first and more as the story goes on, up to 13 (10 on Lunatic). They refill every map.", "風花雪月と同じく、最初は3回でストーリーが進むと増え、最大13回(ルナティックでは10回)。マップごとに回復します。"),
    ("The same number every map, as Engage's 10 on Hard and Maddening. They refill every map.", "どのマップでも同じ回数です(エンゲージのハード・ルナティックの10回のように)。マップごとに回復します。"),
    ("As Engage's Normal: turn back time as often as you like.", "エンゲージのノーマルと同じく、何度でも時を戻せます。"),
    ("Charges per Map", "マップごとの回数"),
    ("How many times you can turn back time on each map.", "1マップで時を戻せる回数です。"),
    // Save states
    ("Save States", "ステートセーブ"),
    ("Save and load the whole game at any moment, in eight slots of this install's own. The game's own saves are never touched.", "いつでもゲーム全体をセーブ・ロードできます(このインストール専用の8スロット)。ゲームのセーブデータには触れません。"),
    ("Quick", "クイック"),
    ("Hotkey Slot", "ホットキーのスロット"),
    ("Slot {}", "スロット{}"),
    ("Empty", "空き"),
    ("Can't load", "ロードできません"),
    ("Empty. Press to save the game here.", "空きです。決定するとここにセーブします。"),
    ("Press to save, load or delete.", "決定するとセーブ、ロード、削除ができます。"),
    ("Turn on Save States first.", "先にステートセーブをONにしてください。"),
    ("Save states can't be used in an online battle.", "オンライン対戦中はステートセーブを使えません。"),
    ("Save Here", "ここにセーブ"),
    ("Load", "ロード"),
    ("Delete", "削除"),
    ("Slot {}: {}", "スロット{}: {}"),
    ("Save over slot {}? What it holds now will be lost.", "スロット{}に上書きしますか？ いまの内容は失われます。"),
    ("Load slot {}? Anything since your last save will be lost.", "スロット{}をロードしますか？ 最後にセーブしてからの進行状況は失われます。"),
    ("Slot {} is empty.", "スロット{}は空きです。"),
    ("Delete slot {}? This can't be undone.", "スロット{}を削除しますか？ 元に戻せません。"),
    ("Choose a slot to see what it holds.", "スロットを選ぶと内容を表示します。"),
    ("Made before Haiku checked mods", "MODの確認より前に作成"),
    ("Can't load with this setup: {}. Press to see why.", "この構成ではロードできません: {}。決定すると理由を表示します。"),
    ("Saved {}: {}{}. Press to save, load or delete.", "{}にセーブ: {}{}。決定するとセーブ、ロード、削除ができます。"),
    // Mods
    ("Unavailable", "使用不可"),
    ("Changes take effect when the game restarts.", "変更はゲームの再起動後に反映されます。"),
    ("No mods installed", "MODがありません"),
    ("Put mods in the mods folder next to Haiku.exe, then open this page again.", "Haiku.exeの隣のmodsフォルダにMODを入れて、もう一度この画面を開いてください。"),
    ("Open Mods Folder", "MODフォルダを開く"),
    ("Shows the mods folder: {}", "MODフォルダを表示します: {}"),
    ("Restarts the game with the mods as set here. Progress since you last saved is lost.", "ここでの設定でゲームを再起動します。最後にセーブしてからの進行状況は失われます。"),
    ("The game is running with the mods as set here.", "ゲームはこのMODの設定で起動しています。"),
    ("Apply and Restart", "適用して再起動"),
    ("No description.", "説明はありません。"),
    ("Mods folder", "MODフォルダ"),
    ("Each mod is a folder or a .zip. Mods made for Luma3DS work as they are.", "MODはフォルダか.zipです。Luma3DS用のMODはそのまま使えます。"),
    ("The list has changed since the game started. Restart to play with it.", "起動してから一覧が変わりました。再起動すると反映されます。"),
    ("Nothing has changed since the game started.", "起動してから変更はありません。"),
    ("On/Off", "ON/OFF"),
    ("Up", "上へ"),
    ("Down", "下へ"),
    ("Shift+Up/Down: Move", "Shift+上下: 並べ替え"),
    ("It goes on with the menu closed.", "メニューを閉じても続きます。"),
    ("Restarted with no mods", "MODなしで再起動しました"),
    ("Restarted with 1 mod", "MOD 1個で再起動しました"),
    ("Restarted with {} mods", "MOD {}個で再起動しました"),
    // online
    ("Your battle code", "対戦コード"),
    ("Allow", "許可する"),
    ("Don't allow", "許可しない"),
    ("Characters modified with cheats: {}", "チートで変更したキャラクター: {}"),
    ("Your router didn't open a port, so some players may not get in.", "ルーターのポートが開いていないため、参加できない相手がいるかもしれません。"),
    ("Send it to the player you're inviting.", "招待する相手に送ってください。"),
    ("Copied", "コピーしました"),
    ("Copy", "コピー"),
    ("Change", "変更"),
    ("Enter a name.", "名前を入力してください。"),
    ("Finish", "決定"),
    // updscreen: the game's code built before the game starts (pregame.rs)
    ("Updating", "アップデート中"),
    ("Building the game's code", "ゲームのコードを作成しています"),
    ("Starting the game", "ゲームを起動しています"),
    ("Haiku's own files", "Haiku のファイル"),
    ("Writing the game's code as C", "ゲームのコードを C に変換"),
    ("Compiling the game's code", "ゲームのコードをコンパイル"),
    ("Linking the game's code", "ゲームのコードをリンク"),
    ("Done", "完了"),
    ("Stop", "中止"),
    ("Keep Building", "続ける"),
    ("Stop and Quit", "中止して終了"),
    ("Try Again", "もう一度"),
    ("Open the Log", "ログを開く"),
    ("Quit", "終了"),
    ("The game's code couldn't be built", "ゲームのコードを作成できませんでした"),
    ("Stop building?", "作成を中止しますか？"),
    ("Haiku closes now, and carries on from where it stopped the next time it starts.", "Haiku を終了します。次回の起動時に、中断したところから再開します。"),
    ("The game starts by itself when this is done.", "完了すると、ゲームが自動で起動します。"),
    ("Builds the game's code again, from where it stopped.", "中断したところから、もう一度ゲームのコードを作成します。"),
    ("Opens Haiku's setup log, which has the compiler's messages.", "コンパイラーのメッセージが記録された Haiku のセットアップログを開きます。"),
    ("Closes Haiku. It tries again the next time it starts.", "Haiku を終了します。次回の起動時にもう一度試します。"),
    ("Goes on building the game's code.", "ゲームのコードの作成を続けます。"),
    ("Closes Haiku. It carries on from where it stopped the next time it starts.", "Haiku を終了します。次回の起動時に、中断したところから再開します。"),
    ("{} of {} files, about {} min left", "{} / {} ファイル、残り約 {} 分"),
    ("{} of {} files", "{} / {} ファイル"),
    ("Reading the game's code", "ゲームのコードを読み込み中"),
    ("Writing the C", "C に変換中"),
    ("Linking", "リンク中"),
    ("Putting the game's code in place", "ゲームのコードを配置中"),
    ("Loading the game's code", "ゲームのコードを読み込み中"),
    ("The game's code has to be built again from your installed game.", "インストールしたゲームから、ゲームのコードをもう一度作成します。"),
    ("Haiku was updated, so the game's code is built again from your installed game, once.", "Haiku がアップデートされたため、ゲームのコードをもう一度作成します（一度だけです）。"),
    // updater: the update's notice and screen (update.rs), Options >
    // Interface > Check for Updates; its failures stay in English
    ("Check for Updates", "アップデートの確認"),
    ("Once a day, ask GitHub (api.github.com) for a newer Haiku and say so on the Main Menu. Only Haiku's version is sent.", "1日1回、GitHub（api.github.com）に新しい Haiku があるか問い合わせ、メインメニューでお知らせします。送るのは Haiku のバージョンだけです。"),
    ("Haiku never checks for updates or connects to GitHub. New versions are at github.com/tyle-beep/Haiku.", "アップデートを確認せず、GitHub に接続しません。新しいバージョンは github.com/tyle-beep/Haiku にあります。"),
    ("Haiku {} is available", "Haiku {} が公開されています"),
    ("{} (you have Haiku {})", "{}（現在の Haiku: {}）"),
    ("Update", "アップデート"),
    ("Install", "インストール"),
    ("Later", "あとで"),
    ("Skip This Version", "このバージョンをスキップ"),
    ("Open the Download Page", "ダウンロードページを開く"),
    ("Download Page", "ダウンロードページ"),
    ("Updating to Haiku {}", "Haiku {} にアップデート中"),
    ("Downloading from GitHub", "GitHub からダウンロード中"),
    ("Checking its SHA-256 and signature", "SHA-256 と署名を確認中"),
    ("Unpacking", "展開中"),
    ("Installing", "インストール中"),
    ("{} of {} MB", "{} / {} MB"),
    ("{} MB, from GitHub", "{} MB（GitHub から）"),
    ("The update wasn't installed", "アップデートはインストールされませんでした"),
    ("Reminds you the next time Haiku starts.", "次回の起動時にもう一度お知らせします。"),
    ("Haiku starts again by itself.", "Haiku が自動で再起動します。"),
    ("Cancelled. Nothing was changed.", "キャンセルしました。何も変更していません。"),
    ("This release has no notes.", "このリリースには説明がありません。"),
    // code mods
    ("Done.", "完了しました。"),
    ("Finding the code the mods change...", "MODが変更するコードを探しています..."),
    ("Compiling... {} of {}", "コンパイル中... {} / {}"),
    ("Downloading the compiler... {}%", "コンパイラーをダウンロード中... {}%"),
    ("Code mods need the compiler: nothing was downloaded", "コードMODにはコンパイラーが必要です: 何もダウンロードしていません"),
    ("Off while it runs", "実行中はOFF"),
    ("The game could not be started. Check that the game folder holds Fates-USA.cxi, then start it again.", "ゲームを起動できませんでした。ゲームのデータを確認して、もう一度起動してください。"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_with_values() {
        assert_eq!(translate("Divine Pulse").as_deref(), Some("天刻の拍動"));
        assert_eq!(translate("Slot 3").as_deref(), Some("スロット3"));
        assert_eq!(translate("Turn 2 \u{b7} Player Phase").as_deref(), Some("ターン2 \u{b7} 自軍フェイズ"));
        assert_eq!(translate("Charges 2/3").as_deref(), Some("残り 2/3"));
        assert_eq!(translate("Delete slot 4? This can't be undone.").as_deref(), Some("スロット4を削除しますか？ 元に戻せません。"));
        assert_eq!(translate("Something Haiku never says"), None);
        // the USA build (the region is USA unless set): every text as it is
        assert_eq!(tr("Divine Pulse"), "Divine Pulse");
    }

    #[test]
    fn every_entry_is_unique_and_its_letters_are_kept() {
        let mut seen = std::collections::HashSet::new();
        for (en, ja) in TEXTS {
            assert!(seen.insert(*en), "twice: {en}");
            assert_eq!(en.matches("{}").count(), ja.matches("{}").count(), "{en}");
            assert!(ja.chars().all(wanted), "{ja}");
        }
        assert!(is_cjk("ターン終了") && !is_cjk("End Turn"));
        assert!(no_break_before('。') && !no_break_before('ア'));
    }
}
