# window-jump

**いま使っているアプリの別ウィンドウ**へ、ショートカットキーか「修飾キー＋タッチパッドのスワイプ」で切り替えるツールです。macOS の `Cmd+`` ` や BetterTouchTool の「同じアプリのウィンドウを巡回」に当たる操作を、**GNOME（Wayland / X11）** と **Windows** で使えます。本体は Rust 製のネイティブバイナリで、Wayland で他に手段がない部分だけを小さな GNOME Shell 拡張が受け持ちます。

[English README](README.md)

- **ショートカット**: 既定は `Super+Alt+N`（次）/ `Super+Alt+P`（前）。設定で変更できます。
- **GNOME のスワイプ**: `Ctrl` を押しながら 3 本指でスワイプ。登録した組み合わせ（指の本数＋修飾キー）だけを GNOME から横取りするので、修飾キーなしの 3 本指スワイプは今まで通りワークスペース切り替えに使えます。指を離さずに動かし続けると何枚も進み、戻すと戻ります。
- **Windows のスワイプ**: Windows の設定でタッチパッドのジェスチャーにショートカットを割り当てます（後述）。
- **並び順**: `stable`（固定順で全ウィンドウを巡回）、`position`（画面の左から右）、`mru`（直近 2 枚の往復。素早く続けて押すと古い方へ進む。Alt+Tab と同じ感覚）。
- **他のツールからも使える**: `window-jump next` / `window-jump prev` は普通のコマンドなので、GNOME のカスタムショートカット、AutoHotkey、keyd、マウスのユーティリティなどにも割り当てられます。

## 仕組み

```
GNOME                                       Windows
─────                                       ───────
window-jump next / daemon                   window-jump next / daemon
        │  D-Bus（セッションバス）                  │  Win32
        ▼                                           ▼
window-jump@hjosugi.github.io               EnumWindows, SetForegroundWindow,
（GNOME Shell 拡張。コンポジタ内で動く）    RegisterHotKey
 ・ウィンドウ一覧 / アクティブ化
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

対応する GNOME Shell: 46〜50（CI で 46、実機で 50 を確認）。

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
3. タッチパッド: **設定 → Bluetooth とデバイス → タッチパッド → 高度なジェスチャー** で、たとえば「3 本指で左にスワイプ」を **カスタム ショートカット** にして `Win+Alt+N` を、右に `Win+Alt+P` を割り当てます。

ウィンドウは実行ファイル（`notepad.exe`、`Code.exe` など）単位でまとめます。UWP アプリは ApplicationFrameHost の枠の裏にある本体のプロセスで判定します。別の仮想デスクトップのウィンドウはまだ対象外です。

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

## 設定

`window-jump config init` でコメント付きのファイルを `~/.config/window-jump/config.toml`（Windows は `%APPDATA%\window-jump\config.toml`）に作ります。項目はすべて省略可能で、既定値は次の通りです。

```toml
order = "stable"          # "stable" | "position" | "mru"
workspace = "all"         # "all" | "current"
include_minimized = true
mru_session_ms = 1000     # order = "mru" のとき、これより短い間隔の連打は 1 回の巡回として続く

[gesture]
threshold = 120.0         # スワイプとみなすまでの指の移動量（GNOME のワークスペース 1 枚分が 400）
repeat_step = 220.0       # 指を離さずに動かしたとき、さらにこの量ごとに 1 枚進む。0 で 1 スワイプ 1 回

[[hotkey]]
keys = "Super+Alt+N"      # 修飾キー: Super (Win), Ctrl, Alt, Shift
action = "next"

[[hotkey]]
keys = "Super+Alt+P"
action = "prev"

[[swipe]]                 # GNOME のみ
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

`hotkey = []` / `swipe = []` で無効にできます。編集したら daemon を再起動してください。

ヒント:

- スワイプの修飾キーに `Super` 単体は避けてください。離したときに GNOME のアクティビティ画面が開きます。`Ctrl`、`Ctrl+Alt`、`Shift` なら問題ありません。
- ホットキーが登録できない場合（GNOME や他のアプリが使用中）は daemon の起動時に表示されます。`window-jump daemon -v` で全イベントをログに出せます。

## 開発

```sh
cargo test                     # Rust のユニットテスト（プラットフォーム非依存のコア）
npm test                       # 拡張の純粋な JS 部分のユニットテスト
cargo build --all-targets && ./test/e2e/gnome.sh
                               # 使い捨ての headless GNOME Shell での e2e テスト:
                               # 本物のウィンドウと、仮想キーボードで押したホットキー
pwsh test/e2e/windows.ps1 -Bin target/debug/window-jump.exe   # Windows 上で
nix flake check                # Nix ビルド（cargo test を含む）
```

e2e テストは動作中のデスクトップには一切触れません。専用の gnome-shell・D-Bus・HOME・dconf を立ち上げて使い捨てます。D-Bus プロトコルは [gnome-extension/README.md](gnome-extension/README.md) を参照してください。CI では上記すべてを Ubuntu（GNOME 46）と Windows Server 2022 で実行しています。

## ライセンス

MIT
