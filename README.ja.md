# window-jump

**いま使っているアプリの別ウィンドウ**へ、ショートカットキーか「修飾キー＋タッチパッドのスワイプ」で切り替えるツールです。macOS の `Cmd+`` ` や BetterTouchTool の「同じアプリのウィンドウを巡回」に当たる操作を、**GNOME（Wayland / X11）**、**Sway・Hyprland・niri**、**Windows** で使えます。本体は Rust 製のネイティブバイナリで、Wayland で他に手段がない部分だけを小さな GNOME Shell 拡張が受け持ちます。

[English README](README.md)

- **ショートカット**: 既定は `Super+Alt+N`（次）/ `Super+Alt+P`（前）。設定で変更できます。
- **GNOME のスワイプ**: `Ctrl` を押しながら 3 本指でスワイプ。登録した組み合わせ（指の本数＋修飾キー）だけを GNOME から横取りするので、修飾キーなしの 3 本指スワイプは今まで通りワークスペース切り替えに使えます。指を離さずに動かし続けると何枚も進み、戻すと戻ります。
- **Windows のスワイプ**: Windows Precision Touchpad なら、GNOME と同じ「修飾キー＋3 本指スワイプ」が使えます。ただし Windows 標準の 3 / 4 本指ジェスチャーは先にオフにする必要があります（後述）。
- **その他のデスクトップのスワイプ**: 省略可能な `--features libinput` 付きでビルドすると、KDE・Sway/Hyprland/niri・X11 のウィンドウマネージャーでは libinput から直接スワイプを読み取れます（後述）。GNOME では拡張を使う経路が優先です。
- **切り替え先の表示**: 切り替えるたびに、ウィンドウのタイトルと「アプリ · 2 / 5」（GNOME ではアプリのアイコンも）を画面中央に一瞬表示します。スワイプで何枚も進むときに今どこにいるかが分かります。Windows では daemon 経由のときに表示されます。
- **並び順**: `stable`（固定順で全ウィンドウを巡回）、`position`（画面の左から右）、`mru`（Alt+Tab と同じ動き。1 回押すと直近 2 枚を往復し、修飾キーを押したまま続けて押すと古い方へ進み、離したところで確定する）。
- **他のツールからも使える**: `window-jump next` / `window-jump prev` は普通のコマンドなので、GNOME のカスタムショートカット、AutoHotkey、keyd、マウスのユーティリティなどにも割り当てられます。

## 仕組み

```
GNOME                                       Windows
─────                                       ───────
window-jump next / daemon                   window-jump next / daemon
        │  D-Bus（セッションバス）                  │  Win32
        ▼                                           ▼
window-jump@hjosugi.github.io               EnumWindows, SetForegroundWindow,
（GNOME Shell 拡張。コンポジタ内で動く）    RegisterHotKey, Raw Input
 ・ウィンドウ一覧 / アクティブ化             （Precision Touchpad）
 ・daemon のホットキーを登録
 ・修飾キー＋スワイプを横取り
```

Wayland では普通のプログラムが他のアプリのウィンドウを列挙・フォーカスしたり、グローバルなキーや GNOME が使うタッチパッドジェスチャーを受け取ったりできません。拡張は GNOME Shell の中で動き、その部分だけをセッションバスで提供します。どのウィンドウに切り替えるか、スワイプが何を意味するか、設定ファイルなどのロジックはすべて Rust 側にあります。daemon が終了すると、登録したキーやスワイプはその場で解放されます。

## インストール

### GNOME（ディストリビューションを問わず）

1. [最新リリース](https://github.com/hjosugi/window-jump/releases/latest)から次を取得します。
   - `window-jump-<version>-x86_64-unknown-linux-musl.tar.gz`（または `aarch64`）: 依存のない静的バイナリ。`window-jump` を `~/.local/bin` など `PATH` の通った場所へ。
   - `window-jump@hjosugi.github.io.shell-extension.zip`
2. 拡張を入れて、**一度ログアウトしてログインし直します**（Wayland の GNOME は新しい拡張をログイン時にしか見つけません）。
   ```sh
   gnome-extensions install --force window-jump@hjosugi.github.io.shell-extension.zip
   # ログインし直したあとで:
   gnome-extensions enable window-jump@hjosugi.github.io
   ```
3. 確認し、daemon を起動し、ログイン時に自動で起動するようにします。
   ```sh
   window-jump doctor
   window-jump autostart enable     # ~/.config/autostart/window-jump.desktop を作成
   window-jump daemon &             # 今のセッション用
   ```

対応する GNOME Shell: 46〜51。CI では 46（Ubuntu 24.04）、48（Debian 13）、49（Fedora 43）、50（Fedora 44）と Arch Linux の最新版（現時点で 51）で e2e テストを実行しています。47 は未テストです。

### NixOS / Nix flake

```nix
# flake.nix
inputs.window-jump.url = "github:hjosugi/window-jump";

# configuration.nix（specialArgs などで window-jump を渡す）
environment.systemPackages = [
  window-jump.packages.${pkgs.stdenv.hostPlatform.system}.window-jump
  window-jump.packages.${pkgs.stdenv.hostPlatform.system}.gnome-shell-extension
];
```

そのあと拡張を有効にし（`gnome-extensions enable window-jump@hjosugi.github.io` か dconf）、`window-jump autostart enable` を実行します。NixOS 以外でも `nix run github:hjosugi/window-jump -- doctor` で試せます。

### Windows 10 / 11

1. `window-jump-<version>-x86_64-pc-windows-msvc.zip`（または `aarch64`）を取得し、`%LOCALAPPDATA%\Programs\window-jump` など消さない場所に展開します。
2. ログイン時の自動起動と、今すぐの起動:
   ```powershell
   .\window-jump.exe autostart enable   # ログイン時に window-jumpw.exe（コンソール窓なし）を起動
   .\window-jumpw.exe                    # 今すぐ起動
   ```
3. タッチパッド（任意）: スワイプには **Windows Precision Touchpad** が必要です（最近のノート PC はほぼ対応）。**設定 → Bluetooth とデバイス → タッチパッド** で **3 本指のジェスチャー** と **4 本指のジェスチャー** をオフ（*何もしない*）にしてから、GNOME と同じ `[[swipe]]` 設定を使います。既定では `Ctrl` を押しながら 3 本指で左右にスワイプします。Windows 標準のジェスチャーを有効にしたままだと、そちらも同時に反応します（生の HID 入力からは横取りできません）。

`window-jumpw.exe` はタスクトレイ（通知領域）にアイコンを表示します。メニューには登録できたホットキーの数（他のアプリに取られて登録できなかったものはその名前）と、**Reload config**（設定の再読み込み）、**Open config file**（設定ファイルを開く）、**Start at login**（ログイン時に起動、現在の状態にチェック）、**Quit**（終了）が並びます。終了するためにタスク マネージャーを開く必要はありません。

ウィンドウは実行ファイル（`notepad.exe`、`Code.exe` など）単位でまとめます。UWP アプリは ApplicationFrameHost の枠の裏にある本体のプロセスで判定します。

別の仮想デスクトップのウィンドウも、`workspace = "all"`（既定）なら対象に含まれ、`"current"` なら除外されます。いまのデスクトップが `ws 0`、ほかのデスクトップが正の番号です。別デスクトップのウィンドウへのジャンプはベストエフォートです。デスクトップを切り替える公開 API がないため、`Win+Ctrl+左/右` を最大 8 回ずつ送り、届かなければエラーを表示します。中断された UWP ウィンドウは表示されません。

手動テスト: `Win+Ctrl+D` で 2 つ目のデスクトップを作り、そこで 2 つ目の Notepad を開き（またはタスク ビューで移動し）、`Win+Ctrl+左` で戻ってから `window-jump list`（2 つ目の Notepad が正の `ws`）と、1 つ目の Notepad での `window-jump next` を確認します。

### ソースから

```sh
cargo install --git https://github.com/hjosugi/window-jump window-jump
```

GNOME 拡張は `gnome-extension/window-jump@hjosugi.github.io` ディレクトリです。`~/.local/share/gnome-shell/extensions/` にコピーしてください。

## 使い方

| コマンド | |
|---|---|
| `window-jump next` / `prev` | フォーカス中のアプリの次 / 前のウィンドウへ。`--order`、`--workspace` で設定を上書き。 |
| `window-jump daemon [-v]` | 設定したホットキーとスワイプを待ち受ける。 |
| `window-jump list [--json]` | アプリごとにウィンドウを一覧表示。 |
| `window-jump activate <id>` | ID を指定して 1 枚フォーカス。 |
| `window-jump doctor` | 設定・拡張・ウィンドウ一覧をまとめて確認。 |
| `window-jump config init\|path\|check` | 設定ファイルの作成 / 場所 / 検証。 |
| `window-jump autostart enable\|disable` | ログイン時の daemon 自動起動。 |

### daemon を使わない場合

コマンドを好きなショートカットツールに割り当てます。GNOME なら **設定 → キーボード → ショートカットの表示とカスタマイズ → カスタムショートカット** でコマンドに `window-jump next` を指定します（ウィンドウの列挙とフォーカスのため、GNOME では拡張は引き続き必要です）。

## Sway・Hyprland・niri

Sway・Hyprland・niri はウィンドウ一覧とフォーカスを独自の IPC で公開しているため、拡張も daemon もなしで動きます。`SWAYSOCK`・`HYPRLAND_INSTANCE_SIGNATURE`・`NIRI_SOCKET` からコンポジタを判別し、ソケットが無い場合は `XDG_CURRENT_DESKTOP` から分かりやすく案内します。`window-jump list`・`next`・`prev`・`activate`・`doctor` はそのまま使えます。

これらのコンポジタにはグローバルホットキーの共通 API がないため、`window-jump daemon` の代わりにコンポジタ側の設定でコマンドを割り当てます。

Sway（`~/.config/sway/config`）:

```
bindsym $mod+n exec window-jump next
bindsym $mod+p exec window-jump prev
```

Hyprland（`~/.config/hypr/hyprland.conf`）:

```
bind = SUPER, N, exec, window-jump next
bind = SUPER, P, exec, window-jump prev
```

niri（`~/.config/niri/config.kdl`）:

```
binds {
    MOD+N { spawn "window-jump" "next"; }
    MOD+P { spawn "window-jump" "prev"; }
}
```

`window-jump daemon` はホットキーには不要です（実行するとコンポジタ設定への割り当てを促すメッセージを出します）。省略可能な `libinput` フィーチャー付きでビルドすると、タッチパッドのスワイプのために daemon を動かし続けます（次節）。無い場合はメッセージを出すだけです。切り替え先の表示は今のところ GNOME / Windows 向けです。`window-jump doctor` が判別したコンポジタ名を表示します。

## GlobalShortcuts ポータル（KDE、GNOME 48 以降）

通常 `window-jump daemon` は GNOME 拡張経由でホットキーを登録します。代わりにデスクトップへ登録させるには、`WINDOW_JUMP_HOTKEYS=portal` を指定します。

```sh
WINDOW_JUMP_HOTKEYS=portal window-jump daemon
```

デスクトップが確認ダイアログを表示し、割り当てを記憶します。KDE では KWin スクリプトが不要で、GNOME 48 以降も同じポータルに対応しています。GNOME ではウィンドウ一覧・HUD・スワイプに引き続き拡張を使い、ホットキーだけをポータルへ移します。未指定なら GNOME は従来どおり拡張のグラブを使い、それ以外のデスクトップはポータルを自動で試します（Sway・Hyprland・niri では上記のコンポジタ設定による割り当てが通常です）。ポータルの有無とアプリ ID（`io.github.hjosugi.WindowJump`）の登録結果は `window-jump doctor` で確認できます。アプリ ID を必要とするデスクトップでは、`.desktop` ファイル名と一致していれば受理されます。ポータルのセッションは 1 回しかバインドできないため、ホットキーの変更は daemon の再起動後に反映されます。

## libinput ジェスチャー（KDE、Sway・Hyprland・niri、X11）

GNOME では拡張、Windows では Precision Touchpad がスワイプを担当します。それ以外のデスクトップでは、省略可能な `libinput` フィーチャーを付けてビルドすると、libinput-gestures のように libinput から直接ジェスチャーを読めます。

```sh
cargo install --locked --path . --features libinput   # チェックアウトから
# または
cargo build --release --features libinput
```

`input` クレートは libinput と libudev を使うため、ビルドには開発用ファイルが必要です（Debian/Ubuntu なら `libinput-dev libudev-dev pkg-config`、Fedora なら `libinput-devel libudev-devel`、このリポジトリでは `nix develop`）。

`/dev/input/event*` を読むには、ユーザーが `input` グループに入っている必要があります。

```sh
sudo usermod -aG input "$USER"
```

ログアウトしてログインし直すと反映されます。`window-jump doctor` の `gestures` 行に、フィーチャーが組み込まれているか、タッチパッドが見つかったか、`/dev/input` を読めるかが表示されます。`input` グループに入るとキーボードを含むすべての入力デバイスを読めるようになる点に注意してください。それが許容できない場合は、対象を絞った udev ルールを使ってください。

あとは通常どおり daemon を動かします。Sway・Hyprland・niri ではホットキーは従来どおりコンポジタ設定から、KDE などのポータル対応デスクトップでは GlobalShortcuts から登録します。daemon は設定した `[[swipe]]` をタッチパッドから処理します。GNOME では拡張を使い続けてください。拡張はスワイプを横取りできるので、シェル側でも同じ操作が起きません。

libinput はコンポジタが同じジェスチャーを見るのを止められません。window-jump は横取りしないため、コンポジタ側でもワークスペース切り替えやウィンドウ移動など、その指に割り当てられた動作が同時に起きる可能性があります。コンポジタが使っていない修飾キー＋指の本数（既定の `Ctrl`＋3 本指はたいてい安全です）を使うか、コンポジタ側のジェスチャーをオフにしてください。Windows Precision Touchpad と同じ制約です。

## 設定

`window-jump config init` でコメント付きのファイルを `~/.config/window-jump/config.toml`（Windows は `%APPDATA%\window-jump\config.toml`）に作ります。項目はすべて省略可能で、既定値は次の通りです。

```toml
order = "stable"          # "stable" | "position" | "mru"
workspace = "all"         # "all" | "current"
include_minimized = true
mru_session_ms = 1000     # order = "mru" で他のツールから window-jump next を呼ぶとき、これより短い間隔の連打を 1 回の巡回とみなす

[gesture]
threshold = 120.0         # スワイプとみなすまでの指の移動量（GNOME のワークスペース 1 枚分が 400）
repeat_step = 220.0       # 指を離さずに動かしたとき、さらにこの量ごとに 1 枚進む。0 で 1 スワイプ 1 回

[hud]                     # 切り替え後の表示
enabled = true
duration_ms = 700

[[hotkey]]
keys = "Super+Alt+N"      # 修飾キー: Super (Win), Ctrl, Alt, Shift
action = "next"

[[hotkey]]
keys = "Super+Alt+P"
action = "prev"

[[swipe]]                 # GNOME / Windows（Precision Touchpad）/ libinput ビルド
fingers = 3               # 3〜5
modifiers = ["ctrl"]
direction = "left"        # left | right | up | down
action = "next"

[[swipe]]
fingers = 3
modifiers = ["ctrl"]
direction = "right"
action = "prev"
```

`hotkey = []` / `swipe = []` で無効にできます。daemon はファイルの変更を 2 秒ほどで取り込みます。書き間違えた場合はログに出して、それまでの設定のまま動き続けます。

ヒント:

- スワイプの修飾キーに `Super` 単体は避けてください。離したときに GNOME のアクティビティ画面（Windows ではスタートメニュー）が開くことがあります。`Ctrl`、`Ctrl+Alt`、`Shift` なら問題ありません。
- Windows では先に標準の 3 / 4 本指ジェスチャーをオフにしてください。Precision Touchpad は同じ指の動きを生の HID 入力として報告しますが、Windows のシェルも独立してジェスチャーを処理するため、横取りはできません。
- ホットキーが登録できない場合（GNOME や他のアプリが使用中）は daemon の起動時に表示されます。`window-jump daemon -v` で全イベントをログに出せます。

## 開発

```sh
cargo test                     # Rust のユニットテスト（プラットフォーム非依存のコア）
npm test                       # 拡張の純粋な JS 部分のユニットテスト
cargo build --all-targets && ./test/e2e/gnome.sh
                               # 使い捨ての headless GNOME Shell での e2e テスト:
                               # 本物のウィンドウと、仮想キーボードで押したホットキー
pwsh test/e2e/windows.ps1 -Bin target/debug/window-jump.exe   # Windows 上で
pwsh test/e2e/windows.ps1 -Bin target/debug/window-jump.exe -VirtualDesktops
                               # 2 つ目の仮想デスクトップ上のウィンドウも確認
nix build .#checks.x86_64-linux.vm-touchpad -L
                               # VM 内の本物の GNOME セッションで、仮想タッチパッドから
                               # スワイプする（KVM が必要）
nix flake check                # Nix のチェック一式: ビルド（cargo test を含む）と VM テスト
```

e2e テストは動作中のデスクトップには一切触れません。専用の gnome-shell・D-Bus・HOME・dconf を立ち上げて使い捨てます。この gnome-shell は入力デバイスを読まないため、タッチパッドの経路は VM で確かめます。uinput の仮想タッチパッドから 3 本指スワイプを libinput・mutter・GNOME Shell に流し、Ctrl＋3 本指ではワークスペースが切り替わらずにウィンドウが切り替わること、修飾キーなしの 3 本指では今まで通りワークスペースが切り替わることを確認します。D-Bus プロトコルは [gnome-extension/README.md](gnome-extension/README.md) を参照してください。CI では上記すべてを GNOME 46・48・49・50・51（Ubuntu、Debian、Fedora、Arch のコンテナ）、NixOS の VM、Windows Server 2022 で実行しています。

## ライセンス

MIT
