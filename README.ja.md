# touchpad-snap

**いま使っているアプリの別ウィンドウ**へ、ショートカットキーか「修飾キー＋タッチパッドのスワイプ」で切り替えるツールです。macOS の `Cmd+`` ` や BetterTouchTool の「同じアプリのウィンドウを巡回」に当たる操作を、**GNOME（Wayland / X11）**、**KDE Plasma**、**Sway・Hyprland・niri**、**Windows** で使えます。本体は Rust 製のネイティブバイナリで、Wayland で他に手段がない部分だけを小さな GNOME Shell 拡張または KWin スクリプトが受け持ちます。

[English README](README.md)

- **ショートカット**: 既定は `Super+Alt+N`（次）/ `Super+Alt+P`（前）。設定で変更できます。
- **GNOME のスワイプ**: `Ctrl` を押しながら 3 本指で左右にスワイプ、または `Alt` を押しながら 3 本指で上下にスワイプ（設定で変更できます）。登録した組み合わせ（指の本数＋修飾キー）だけを GNOME から横取りするので、修飾キーなしの 3 本指スワイプは今まで通りワークスペース切り替えとアクティビティ画面に使えます。横取りはアプリ単位でも絞れます（[`when`](#条件when)）。対象外のアプリでは GNOME のジェスチャーのままです。指を離さずに動かし続けると何枚も進み、戻すと戻ります。
- **Windows のスワイプ**: Windows Precision Touchpad なら、GNOME と同じ「修飾キー＋3 本指スワイプ」が使えます。ただし Windows 標準の 3 / 4 本指ジェスチャーは先にオフにする必要があります（後述）。
- **その他のデスクトップのスワイプ**: 省略可能な `--features libinput` 付きでビルドすると、KDE・Sway/Hyprland/niri・X11 のウィンドウマネージャーでは libinput から直接スワイプを読み取れます（後述）。GNOME では拡張を使う経路が優先です。
- **切り替え先の表示**: 切り替えるたびに、ウィンドウのタイトルと「アプリ · 2 / 5」（GNOME ではアプリのアイコンも）を画面中央に一瞬表示します。スワイプで何枚も進むときに今どこにいるかが分かります。Windows では daemon 経由のときに表示されます。
- **並び順**: `stable`（固定順で全ウィンドウを巡回）、`position`（画面の左から右）、`mru`（Alt+Tab と同じ動き。1 回押すと直近 2 枚を往復し、修飾キーを押したまま続けて押すと古い方へ進み、離したところで確定する）。
- **他のツールからも使える**: `touchpad-snap next` / `touchpad-snap prev` は普通のコマンドなので、GNOME のカスタムショートカット、AutoHotkey、keyd、マウスのユーティリティなどにも割り当てられます。

## 仕組み

```
GNOME                                       Windows
─────                                       ───────
touchpad-snap next / daemon                   touchpad-snap next / daemon
        │  D-Bus（セッションバス）                  │  Win32
        ▼                                           ▼
touchpad-snap@hjosugi.github.io               EnumWindows, SetForegroundWindow,
（GNOME Shell 拡張。コンポジタ内で動く）    RegisterHotKey, Raw Input
 ・ウィンドウ一覧 / アクティブ化             （Precision Touchpad）
 ・daemon のホットキーを登録
 ・修飾キー＋スワイプを横取り
```

Wayland では普通のプログラムが他のアプリのウィンドウを列挙・フォーカスしたり、グローバルなキーや GNOME が使うタッチパッドジェスチャーを受け取ったりできません。拡張は GNOME Shell の中で動き、その部分だけをセッションバスで提供します。どのウィンドウに切り替えるか、スワイプが何を意味するか、設定ファイルなどのロジックはすべて Rust 側にあります。daemon が終了すると、登録したキーやスワイプはその場で解放されます。

KDE Plasma では小さな KWin スクリプトが拡張の役割を果たします（[KDE Plasma](#kde-plasma) を参照）。ホットキーはデスクトップの GlobalShortcuts ポータルから受け取るため、`touchpad-snap daemon` もそのまま使えます。

## インストール

### GNOME（ディストリビューションを問わず）

1. [最新リリース](https://github.com/hjosugi/touchpad-snap/releases/latest)から次を取得します。
   - `touchpad-snap-<version>-x86_64-unknown-linux-musl.tar.gz`（または `aarch64`）: 依存のない静的バイナリ。`touchpad-snap` を `~/.local/bin` など `PATH` の通った場所へ。
   - `touchpad-snap@hjosugi.github.io.shell-extension.zip`
2. 拡張を入れて、**一度ログアウトしてログインし直します**（Wayland の GNOME は新しい拡張をログイン時にしか見つけません）。
   ```sh
   gnome-extensions install --force touchpad-snap@hjosugi.github.io.shell-extension.zip
   # ログインし直したあとで:
   gnome-extensions enable touchpad-snap@hjosugi.github.io
   ```
3. 確認し、daemon を起動し、ログイン時に自動で起動するようにします。
   ```sh
   touchpad-snap doctor
   touchpad-snap autostart enable     # ~/.config/autostart/touchpad-snap.desktop を作成
   touchpad-snap daemon &             # 今のセッション用
   ```

Linux では `touchpad-snap autostart enable --systemd` を使うと、XDG の代わりに `~/.config/systemd/user/touchpad-snap.service` を作成して有効化します。systemd がグラフィカルセッションと一緒に起動し、クラッシュ時は `Restart=on-failure` で再起動します。ログは `journalctl --user -u touchpad-snap`、状態は `systemctl --user status touchpad-snap` で確認できます。削除は `touchpad-snap autostart disable --systemd` です。XDG と systemd はどちらか一方だけを使ってください。リリースの tarball にはパッケージ用の `touchpad-snap.service` も同梱されています。

対応する GNOME Shell: 46〜51。CI では 46（Ubuntu 24.04）、48（Debian 13）、49（Fedora 43）、50（Fedora 44）と Arch Linux の最新版（現時点で 51）で e2e テストを実行しています。47 は未テストです。

### NixOS / Nix flake

flake は NixOS 用と Home Manager 用のモジュールを、それぞれ `default` と
`touchpad-snap` という名前で公開しています。

```nix
# flake.nix
{
  inputs.touchpad-snap.url = "github:hjosugi/touchpad-snap";

  outputs = { self, nixpkgs, touchpad-snap, ... }: {
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      modules = [
        touchpad-snap.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

```nix
# configuration.nix
services.touchpad-snap = {
  enable = true;
  settings = {
    cycle = { order = "mru"; };
    bindings = [
      {
        trigger = { type = "hotkey"; keys = "Super+Alt+N"; };
        actions = [ { type = "window.next"; } ];
      }
    ];
  };
};
```

このモジュールは次を行います。

- `touchpad-snap` パッケージを入れ、`touchpad-snap daemon` を
  **ユーザー** systemd サービスとして `graphical-session.target` に結び付け、
  `Restart=on-failure` で動かします。
- `settings`（`config.json` スキーマ）を
  `/etc/touchpad-snap/config.json` に書き出し、daemon を
  `--config /etc/touchpad-snap/config.json` 付きで起動します。
- GNOME Shell 拡張を入れ、`programs.dconf.profiles.user.databases` 経由で
  `org.gnome.shell.enabled-extensions` の**システム既定値**に
  `touchpad-snap@hjosugi.github.io` を追加します。

オプションは `enable`（既定 `false`）、`package`、`settings`（既定 `{}`）、
`systemd.enable`（既定 `true`）、`gnome.enable`（既定 `true`）です。
`gnome.enable = true` のとき拡張パッケージは `gnome.extensionPackage` で
選べます。daemon を自分で管理するなら `systemd.enable = false`、GNOME 以外
のデスクトップなら `gnome.enable = false` にします。

dconf のデータベースは `user-db:user` の後ろに置かれる*既定値*です。すでに
`enabled-extensions` を自分で書いているユーザー（拡張アプリでオン・オフした
場合など）ではユーザーの値が優先され、システム既定値は使われません。最初の
切り替え後に一度ログアウトしてログインし直し、次のどちらかを行ってください。

```sh
gnome-extensions enable touchpad-snap@hjosugi.github.io
```

または、ユーザー側のキーを消してシステム既定値に戻します。

```sh
dconf reset /org/gnome/shell/enabled-extensions
```

モジュールを使わない場合も、パッケージは直接指定できます。

```nix
environment.systemPackages = [
  touchpad-snap.packages.${pkgs.stdenv.hostPlatform.system}.touchpad-snap
  touchpad-snap.packages.${pkgs.stdenv.hostPlatform.system}.gnome-shell-extension
];
```

そのあと拡張を有効にし（`gnome-extensions enable touchpad-snap@hjosugi.github.io`
か dconf）、`touchpad-snap autostart enable` を実行します。NixOS 以外でも
`nix run github:hjosugi/touchpad-snap -- doctor` で試せます。

#### Home Manager

```nix
{
  imports = [ inputs.touchpad-snap.homeManagerModules.default ];

  services.touchpad-snap = {
    enable = true;
    settings = {
      cycle = { order = "mru"; };
      bindings = [
        {
          trigger = { type = "hotkey"; keys = "Super+Alt+N"; };
          actions = [ { type = "window.next"; } ];
        }
      ];
    };
  };
}
```

オプション名は NixOS モジュールと同じです。設定は
`$XDG_CONFIG_HOME/touchpad-snap/config.json`（通常は
`~/.config/touchpad-snap/config.json`）に書き出されます。これは daemon が
既定で読むパスなので、ユーザーサービスは `touchpad-snap daemon` をそのまま
実行します。`gnome.enable` は Home Manager の
`programs.gnome-shell.extensions` を使い、これも
`org/gnome/shell/enabled-extensions` を書き込みます。このリストは丸ごと
管理されるため、ほかの拡張も `programs.gnome-shell.extensions` に追加して
ください（既存リストへの追記にはなりません）。

以前の名前は **window-jump** で、バイナリは初回起動時に
`~/.config/window-jump/config.toml` を
`~/.config/touchpad-snap/config.toml` へコピーします。この TOML はまだ
メモリ上で読まれますが、`touchpad-snap config import` を一度実行して新しい
`config.json` を書き出してください。どちらのモジュールも新しいファイルを
自分で書く（NixOS は明示的に `--config` を渡す）ため、この自動コピーは
起きません。古い設定は `settings` に移してください。

### Windows 10 / 11

1. `touchpad-snap-<version>-x86_64-pc-windows-msvc.zip`（または `aarch64`）を取得し、`%LOCALAPPDATA%\Programs\touchpad-snap` など消さない場所に展開します。
2. ログイン時の自動起動と、今すぐの起動:
   ```powershell
   .\touchpad-snap.exe autostart enable   # ログイン時に touchpad-snapw.exe（コンソール窓なし）を起動
   .\touchpad-snapw.exe                    # 今すぐ起動
   ```
3. タッチパッド（任意）: スワイプには **Windows Precision Touchpad** が必要です（最近のノート PC はほぼ対応）。**設定 → Bluetooth とデバイス → タッチパッド** で **3 本指のジェスチャー** と **4 本指のジェスチャー** をオフ（*何もしない*）にしてから、GNOME と同じ `swipe` 設定を使います。既定では `Ctrl` を押しながら 3 本指で左右に、または `Alt` を押しながら 3 本指で上下にスワイプします。Windows 標準のジェスチャーを有効にしたままだと、そちらも同時に反応します（生の HID 入力からは横取りできません）。

`touchpad-snapw.exe` はタスクトレイ（通知領域）にアイコンを表示します。メニューには登録できたホットキーの数（他のアプリに取られて登録できなかったものはその名前）と、**Reload config**（設定の再読み込み）、**Open config file**（設定ファイルを開く）、**Start at login**（ログイン時に起動、現在の状態にチェック）、**Quit**（終了）が並びます。終了するためにタスク マネージャーを開く必要はありません。

ウィンドウは実行ファイル（`notepad.exe`、`Code.exe` など）単位でまとめます。UWP アプリは ApplicationFrameHost の枠の裏にある本体のプロセスで判定します。

別の仮想デスクトップのウィンドウも、`cycle.workspace` が `"all"`（既定）なら対象に含まれ、`"current"` なら除外されます。いまのデスクトップが `ws 0`、ほかのデスクトップが正の番号です。別デスクトップのウィンドウへのジャンプはベストエフォートです。デスクトップを切り替える公開 API がないため、`Win+Ctrl+左/右` を最大 8 回ずつ送り、届かなければエラーを表示します。中断された UWP ウィンドウは表示されません。

手動テスト: `Win+Ctrl+D` で 2 つ目のデスクトップを作り、そこで 2 つ目の Notepad を開き（またはタスク ビューで移動し）、`Win+Ctrl+左` で戻ってから `touchpad-snap list`（2 つ目の Notepad が正の `ws`）と、1 つ目の Notepad での `touchpad-snap next` を確認します。

### ソースから

```sh
cargo install --git https://github.com/hjosugi/touchpad-snap touchpad-snap
```

GNOME 拡張は `gnome-extension/touchpad-snap@hjosugi.github.io` ディレクトリです。`~/.local/share/gnome-shell/extensions/` にコピーしてください。

### window-jump からの移行

touchpad-snap は、バージョン 0.5 より前は **window-jump** という名前でした。中身は同じで、名前だけがすべて変わっています。上の手順で touchpad-snap を入れたあと、次のようにします。

- **設定**: `~/.config/window-jump/config.toml`（Windows は `%APPDATA%\window-jump`）は、touchpad-snap を初めて動かしたときに新しい `touchpad-snap` フォルダへコピーされます。古いファイルはそのまま残り、以後は読まれません。
- **GNOME**: 拡張を入れ替えます。新しい拡張が見つからないときは、一度ログインし直してください。
  ```sh
  gnome-extensions disable window-jump@hjosugi.github.io
  gnome-extensions enable touchpad-snap@hjosugi.github.io
  ```
- **ログイン時の起動**: `touchpad-snap autostart enable` が window-jump の登録を置き換えます。Windows では、先にタスクトレイのアイコンから window-jump を終了してください。`--systemd` を付けると、古い `window-jump.service` ユーザーユニットも削除します。
- **ポータル経由のホットキー**: 環境変数は `TOUCHPAD_SNAP_HOTKEYS` になりました（`WINDOW_JUMP_HOTKEYS` も引き続き読みます）。アプリ ID も名前と一緒に変わったので、デスクトップが割り当ての確認をもう一度表示します。
- `touchpad-snap doctor` を実行すると、window-jump のうちまだ動いているものが分かります。
- 古いバイナリ（`cargo uninstall window-jump`、または `window-jump.exe` と `window-jumpw.exe` を削除）と `~/.local/share/gnome-shell/extensions/window-jump@hjosugi.github.io` は消して構いません。

## 使い方

| コマンド | |
|---|---|
| `touchpad-snap next` / `prev` | フォーカス中のアプリの次 / 前のウィンドウへ。`--order`、`--workspace` で設定を上書き。 |
| `touchpad-snap daemon [-v]` | 設定したホットキーとスワイプを待ち受ける。 |
| `touchpad-snap list [--json]` | アプリごとにウィンドウを一覧表示。 |
| `touchpad-snap activate <id>` | ID を指定して 1 枚フォーカス。 |
| `touchpad-snap doctor` | 設定・バックエンド・ウィンドウ一覧をまとめて確認。 |
| `touchpad-snap config init\|import\|schema\|path\|check` | 設定ファイルの作成 / 取り込み / スキーマ表示 / 場所 / 検証。 |
| `touchpad-snap autostart enable\|disable [--systemd]` | ログイン時の daemon 自動起動（既定は XDG、`--systemd` で systemd ユーザーサービス）。 |

### daemon を使わない場合

コマンドを好きなショートカットツールに割り当てます。GNOME なら **設定 → キーボード → ショートカットの表示とカスタマイズ → カスタムショートカット** でコマンドに `touchpad-snap next` を指定します（ウィンドウの列挙とフォーカスのため、GNOME では拡張は引き続き必要です）。

## Sway・Hyprland・niri

Sway・Hyprland・niri はウィンドウ一覧とフォーカスを独自の IPC で公開しているため、拡張も daemon もなしで動きます。`SWAYSOCK`・`HYPRLAND_INSTANCE_SIGNATURE`・`NIRI_SOCKET` からコンポジタを判別し、ソケットが無い場合は `XDG_CURRENT_DESKTOP` から分かりやすく案内します。`touchpad-snap list`・`next`・`prev`・`activate`・`doctor` はそのまま使えます。

これらのコンポジタにはグローバルホットキーの共通 API がないため、`touchpad-snap daemon` の代わりにコンポジタ側の設定でコマンドを割り当てます。

Sway（`~/.config/sway/config`）:

```
bindsym $mod+n exec touchpad-snap next
bindsym $mod+p exec touchpad-snap prev
```

Hyprland（`~/.config/hypr/hyprland.conf`）:

```
bind = SUPER, N, exec, touchpad-snap next
bind = SUPER, P, exec, touchpad-snap prev
```

niri（`~/.config/niri/config.kdl`）:

```
binds {
    MOD+N { spawn "touchpad-snap" "next"; }
    MOD+P { spawn "touchpad-snap" "prev"; }
}
```

`touchpad-snap daemon` はホットキーには不要です（実行するとコンポジタ設定への割り当てを促すメッセージを出します）。省略可能な `libinput` フィーチャー付きでビルドすると、タッチパッドのスワイプのために daemon を動かし続けます（次節）。無い場合はメッセージを出すだけです。切り替え先の表示は今のところ GNOME / Windows 向けです。`touchpad-snap doctor` が判別したコンポジタ名を表示します。

## KDE Plasma

Plasma は GNOME Shell 拡張に対応していません。代わりに touchpad-snap は小さな KWin スクリプトを `$XDG_RUNTIME_DIR/touchpad-snap/` に書き出し、`org.kde.KWin.Scripting` 経由でプロセスごとの名前で読み込みます。スクリプトは `workspace.windowList()` を専用のセッションバスサービスへ送り、返ってきたフォーカス指示を実行します。システム設定へのインストールや有効化は不要で、`touchpad-snap` コマンド（または daemon）が動いている間だけ存在し、終了時にアンロードされます。`touchpad-snap list`・`next`・`prev`・`activate`・`doctor` はそのまま使え、`doctor` は検出したセッションと KWin スクリプトが利用できるかを表示します。MRU 順は KWin のスタッキング順によるベストエフォートです。

`touchpad-snap daemon` はホットキーを GlobalShortcuts ポータルから受け取り（デスクトップが確認ダイアログを表示します）、ウィンドウの列挙とフォーカスには KWin ブリッジを使います。`xdg-desktop-portal-kde` が必要です。ポータルが使えない場合、KDE では daemon がグローバルホットキーを登録できません。**システム設定 → ショートカット** で `touchpad-snap next` / `touchpad-snap prev` をコマンドのショートカットとして割り当て、daemon は使わないでください。タッチパッドのスワイプと切り替え先の表示は今のところ GNOME / Windows 向けで、KDE ではウィンドウの列挙とフォーカスのみです。

ブリッジは通常は自動で後片付けされます。touchpad-snap が強制終了された場合、読み込んだスクリプトはポーリングを止めますが KWin が再起動するまで登録されたまま残ります。ログアウトしてログインし直すか（この方法で読み込んだ KWin スクリプトは永続化されません）、読み込んだプロセスの pid を付けてアンロードします。

```sh
qdbus6 org.kde.KWin /Scripting unloadScript touchpad-snap-kde-bridge-<pid>
```

生成ファイルは次のように削除できます。

```sh
rm -f "$XDG_RUNTIME_DIR"/touchpad-snap/kde-bridge-*.js
```

## GlobalShortcuts ポータル（KDE、GNOME 48 以降）

通常 `touchpad-snap daemon` は GNOME 拡張経由でホットキーを登録します。代わりにデスクトップへ登録させるには、`TOUCHPAD_SNAP_HOTKEYS=portal` を指定します。

```sh
TOUCHPAD_SNAP_HOTKEYS=portal touchpad-snap daemon
```

デスクトップが確認ダイアログを表示し、割り当てを記憶します。KDE では daemon のホットキーをこのポータルから受け取ります（上記の KWin スクリプトはウィンドウ一覧とフォーカス用です）。GNOME 48 以降も同じポータルに対応しています。GNOME ではウィンドウ一覧・HUD・スワイプに引き続き拡張を使い、ホットキーだけをポータルへ移します。未指定なら GNOME は従来どおり拡張のグラブを使い、それ以外のデスクトップはポータルを自動で試します（Sway・Hyprland・niri では上記のコンポジタ設定による割り当てが通常です）。ポータルの有無とアプリ ID（`io.github.hjosugi.TouchpadSnap`）の登録結果は `touchpad-snap doctor` で確認できます。アプリ ID を必要とするデスクトップでは、`.desktop` ファイル名と一致していれば受理されます。ポータルのセッションは 1 回しかバインドできないため、ホットキーの変更は daemon の再起動後に反映されます。

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

ログアウトしてログインし直すと反映されます。`touchpad-snap doctor` の `gestures` 行に、フィーチャーが組み込まれているか、タッチパッドが見つかったか、`/dev/input` を読めるかが表示されます。`input` グループに入るとキーボードを含むすべての入力デバイスを読めるようになる点に注意してください。それが許容できない場合は、対象を絞った udev ルールを使ってください。

あとは通常どおり daemon を動かします。Sway・Hyprland・niri ではホットキーは従来どおりコンポジタ設定から、KDE などのポータル対応デスクトップでは GlobalShortcuts から登録します。daemon は設定した `swipe` をタッチパッドから処理します。GNOME では拡張を使い続けてください。拡張はスワイプを横取りできるので、シェル側でも同じ操作が起きません。

libinput はコンポジタが同じジェスチャーを見るのを止められません。touchpad-snap は横取りしないため、コンポジタ側でもワークスペース切り替えやウィンドウ移動など、その指に割り当てられた動作が同時に起きる可能性があります。コンポジタが使っていない修飾キー＋指の本数（既定の `Ctrl`＋3 本指はたいてい安全です）を使うか、コンポジタ側のジェスチャーをオフにしてください。Windows Precision Touchpad と同じ制約です。

## 設定

`touchpad-snap config init` で既定の JSON ファイルを
`~/.config/touchpad-snap/config.json`（Windows は
`%APPDATA%\touchpad-snap\config.json`）に作ります。項目はすべて省略可能で、
省略した項目は既定値になります（`"bindings": []` を明示すると全バインディング
を無効にできます）。既定値は次の通りです（バインディングも省略せずに書いて
います）。

```json
{
  "$schema": "https://raw.githubusercontent.com/hjosugi/touchpad-snap/main/schema/config.schema.json",
  "version": 2,
  "cycle": {
    "order": "stable",
    "workspace": "all",
    "include_minimized": true,
    "mru_session_ms": 1000
  },
  "gesture": { "threshold": 120, "repeat_step": 220 },
  "hud": { "enabled": true, "duration_ms": 700 },
  "bindings": [
    {
      "name": "Next window of this app",
      "trigger": { "type": "hotkey", "keys": "Super+Alt+N" },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "name": "Previous window of this app",
      "trigger": { "type": "hotkey", "keys": "Super+Alt+P" },
      "actions": [ { "type": "window.prev" } ]
    },
    {
      "name": "Next window (Ctrl + swipe left)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "left" },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "name": "Previous window (Ctrl + swipe right)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "right" },
      "actions": [ { "type": "window.prev" } ]
    },
    {
      "name": "Next window (Alt + swipe up)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "up" },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "name": "Previous window (Alt + swipe down)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "down" },
      "actions": [ { "type": "window.prev" } ]
    }
  ]
}
```

- `trigger` はホットキー `{ "type": "hotkey", "keys": "Super+Alt+N" }`
  （修飾キーは Super (Win)、Ctrl、Alt、Shift。キーは最後）か、スワイプ
  `{ "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "left" }`
  （`fingers` は 3〜5、`direction` は left / right / up / down、modifiers は
  `ctrl` / `alt` / `shift` / `super` の組み合わせ）です。
- `actions` は順番に実行するチェーンです。現在使えるのは `window.next`、
  `window.prev`、`delay`（`{ "type": "delay", "ms": 100 }`）、`exec`（下記）、
  `keys`（下記）です。ウィンドウ・ワークスペース・アプリ操作は今後のリリースです。
  それらを使ったバインディングは警告を出して読み飛ばします。
- `keys` は、フォーカス中のウィンドウにキーボードショートカットを打ちます
  （`{ "type": "keys", "keys": "Ctrl+Shift+Tab" }`）。書き方はホットキーと同じで、
  修飾キーのあとに、英数字、`F1`〜`F24`、矢印、`Tab`、`Space`、`Return`、`Escape`、
  `Home`、`End`、`PageUp`、`PageDown`、`Backspace`、`Delete`、記号のどれかを書きます。
  トリガーの修飾キーはショートカットに含まれません。`Alt` を押しながらのスワイプで
  `Ctrl+Tab` を送れば、`Alt+Ctrl+Tab` ではなく `Ctrl+Tab` として届きます。Windows では
  そのあいだ修飾キーをいったん離して、打ち終わったら押し直します。GNOME では仮想
  キーボードが、指で押しているキーを離せないため、修飾キーから指を離したときに
  打ちます（3 秒を超えて押し続けると捨てます）。GNOME 拡張はプロトコル 5 が必要です
  （更新後は一度ログインし直してください）。ほかのデスクトップでは未対応と表示します。
- `name` は任意で、ログと `config check` にだけ使います。
  `enabled: false` にすると、バインディングをファイルに残したまま登録しません。
- `repeat` は、指を離さずに動かしたとき `gesture.repeat_step` ごとに
  スワイプを再度発火するかです。省略時は `window.next` / `window.prev` を
  含むチェーンなら true、それ以外は false です。
- `when`（任意）で、フォーカス中のアプリ・OS・デスクトップごとに
  バインディングを絞り込めます。詳しくは下の「条件（`when`）」を参照してください。
- `cycle`、`gesture`、`hud` は従来どおりです。`cycle.order` は `stable`、
  `position`、`mru`。`cycle.workspace` は `all` か `current`。
  `gesture.threshold` はスワイプとみなすまでの指の移動量（GNOME の
  ワークスペース 1 枚分が 400）。`gesture.repeat_step` は追加で 1 枚進む
  ごとの移動量（0 で 1 スワイプ 1 回）。`hud` は切り替え後の表示です。

### `exec`: コマンドを実行する

`exec` はジェスチャーやホットキーが発火したときにプログラムを実行します。
プログラムと引数を配列で渡すと、シェルを介さずそのまま実行します。予期しない
分割や展開は起こりません。

```json
{ "type": "exec", "command": ["ghostty", "--working-directory", "~"] }
```

文字列を 1 つ渡す場合は Linux では `sh -c`、Windows では `cmd /C` で実行され、
`"shell": true` が必要です。

```json
{ "type": "exec", "command": "notify-send \"$TOUCHPAD_SNAP_WINDOW_TITLE\"", "shell": true }
```

`"shell": true` の無い文字列、`"shell": true` 付きの配列はバインディングの
エラーになります。

コマンドは daemon から切り離されて実行されます（Linux では独自のプロセス
グループ、Windows ではコンソールウィンドウなし、stdin は `/dev/null` または
`NUL`）。daemon は待たず、別スレッドが終了を回収します。
`touchpad-snap daemon --verbose` は終了ステータスと stderr をログに出します。
子プロセスには、トリガー開始時点でフォーカスされていたウィンドウと、トリガー
自体が環境変数で渡されます。

| 変数 | 値 |
| --- | --- |
| `TOUCHPAD_SNAP_WINDOW_ID` | フォーカス中のウィンドウ id |
| `TOUCHPAD_SNAP_APP_ID` | アプリ id（`org.gnome.TextEditor.desktop`、実行ファイルのパスなど） |
| `TOUCHPAD_SNAP_APP_NAME` | アプリ名 |
| `TOUCHPAD_SNAP_WINDOW_TITLE` | ウィンドウのタイトル |
| `TOUCHPAD_SNAP_TRIGGER` | トリガー（`swipe-3-alt-up`、`hotkey-Super+Alt+N` など） |

ウィンドウから得られる変数だけを設定し、daemon 自身の環境は変更しません。
`exec` は `"repeat": true` を指定しない限り 1 回のスワイプで 1 回だけ発火
します（`window.next` / `window.prev` を含むチェーンの既定は従来どおり true）。
`config check` は配列で指定したプログラムが `PATH` に無いときに警告します。
シェル文字列はチェックできないため、その旨だけを表示します。

**信頼できる設定ファイルだけを使ってください。** 設定ファイルは `exec` により
あなたの権限で任意のコマンドを実行できます。サンドボックスはありません。他人の
設定を取り込む前に `exec` アクションを確認してください。GUI も取り込み前に
`exec` を表示する必要があります。
### 条件（`when`）

同じ操作を、場所によって別の動作にできます。

```json
{
  "bindings": [
    {
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "down" },
      "when": { "app": ["firefox", "org.chromium.Chromium"] },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "down" },
      "actions": [ { "type": "window.prev" } ]
    }
  ]
}
```

- `app`: アプリのパターン（大文字小文字を区別しない glob、`*` 使用可）。
  デスクトップファイル ID（`list` が `[org.gnome.TextEditor.desktop]` と
  表示するもの）、アプリ名、WM クラスのいずれかに一致します。Windows では
  ID が実行ファイルのパス（`code.exe`）になります。`firefox`、`Firefox`、
  `firefox.desktop` のどれでも一致します。
- `not_app`: 除外するアプリ（同じパターン）。
- `os`: `linux` または `windows`（`*` も可）。
- `desktop`: `gnome`、`kde`、`sway`、`hyprland`、`niri`（`*` も可）。
  1 つの設定ファイルをどのマシンでも使えます。
- `when` が空か無い場合はどこでも適用されます。書いた条件はすべて満たす
  必要があります。

同じトリガーの有効なバインディングのうち、`when` が一致するものが `when`
の無いものより優先されます。同点ならファイル内で先のものが勝ち、どれも
一致しなければ何も起きません。フォーカス中のアプリとプラットフォームは
トリガーの開始時に取得し、そのジェスチャーの間は変わりません。同じ
ホットキーを複数のアプリ用に書けるよう、ホットキーの重複は許可されます
（条件の無いものを最後に置くとフォールバックになります）。

`config check` は決して発火しない条件（未知の `os` / `desktop`、`app` と
`not_app` の両方にある同じパターン、`not_app: ["*"]`）を報告します。また、
このマシンでは無効なだけのバインディング（Linux 上での `windows` など）も
一覧します。どのウィンドウにも一致しないアプリパターンは実行時にしか
分からないため報告しません。

**GNOME ではスワイプの横取りもアプリ単位です。** Firefox 専用のスワイプは
アプリ条件付きで拡張へ送られ、拡張はスワイプ開始時にフォーカス中のアプリを
確認し、それ以外のアプリでは GNOME にジェスチャーを譲ります。したがって
条件に合わないアプリでは、修飾キーなしの 3 本指スワイプが従来どおり
ワークスペース切り替えとして動きます。これはプロトコル 4 の機能です。
古い拡張でも動作しますが、アプリ条件付きスワイプは全アプリで横取りされ、
対象外のアプリでは何も起きません（起動時に daemon が警告します）。
`not_app` だけのバインディングは GNOME の肯定的なアプリ条件に変換できない
ため、全アプリで横取りし、除外したアプリでは何もしません。

GNOME の横取りは（指の本数、修飾キー、アプリ）単位で、方向は含みません。
アプリ条件付きの修飾キーなし 3 本指バインディングは、そのアプリでの
3 本指スワイプを全方向で横取りするため、バインドしていない方向は GNOME に
渡らず何も起きません。

Windows の生タッチパッド入力はジェスチャーを返却できません。入力が届いた
時点で Windows 自身のジェスチャー処理も動いています。Windows 標準の 3 / 4
本指ジェスチャーをオフにし、シェルのジェスチャーと重ならないよう修飾キー＋
指で割り当ててください。省略可能な `libinput` バックエンドも同じで、横取り
せずに観測するだけなので、コンポジタ側でも同じジェスチャーが動きます。

ファイル内の既知のオブジェクトは厳格です。綴り間違いや未知のフィールドは
JSON パス付きのエラーになります。ただしバインディングは 1 つずつ読むため、
壊れたバインディングは報告して読み飛ばし、ほかはそのまま読み込みます。
`config check` は `bindings[i]` ごとの問題と、`PATH` に無い `exec` プログラム
を列挙し、壊れたバインディングがあるときだけ非ゼロで終了します。未知の
trigger / action 種別や、このビルドが知らない `when` のキー（新しいビルドの
機能）は警告扱いで読み飛ばし、終了コードは 0 のままです。daemon も同じ方針で、
有効なバインディングだけで動き続けます。
`config schema` は Rust の型から生成した JSON Schema を表示します。リポジトリ
の `schema/config.schema.json` が `$schema` の指す先で、エディタの検証・補完に
使えます。

daemon 実行中にファイルを編集すると、2 秒ほどで取り込みます。壊れた編集は
daemon のログに出して、それまでの設定のまま動き続けます。ファイルを削除すると
組み込みの既定値に戻ります。

### `config.toml` からの移行

`config.json` が無いときは、隣にある `config.toml` をメモリ上で読み込みます
（`--config something.toml` と明示した場合も TOML として読みます）。
`touchpad-snap doctor` と `config check` はその状態を表示します。
`touchpad-snap config import` で変換して `config.json` を書き出します
（`--force` で既存の JSON を上書き）。`config init` は JSON の既定値を書き、
`config path` は JSON のパスを表示します。

ヒント:

- スワイプの修飾キーに `Super` 単体は避けてください。離したときに GNOME のアクティビティ画面（Windows ではスタートメニュー）が開くことがあります。`Ctrl`、`Ctrl+Alt`、`Shift` なら問題ありません。
- Windows では先に標準の 3 / 4 本指ジェスチャーをオフにしてください。Precision Touchpad は同じ指の動きを生の HID 入力として報告しますが、Windows のシェルも独立してジェスチャーを処理するため、横取りはできません。
- ホットキーが登録できない場合（GNOME や他のアプリが使用中）は daemon の起動時に表示されます。`touchpad-snap daemon -v` で全イベントをログに出せます。

## 開発

```sh
cargo test                     # Rust のユニットテスト（プラットフォーム非依存のコア）
npm test                       # 拡張の純粋な JS 部分のユニットテスト
cargo build --all-targets && ./test/e2e/gnome.sh
                               # 使い捨ての headless GNOME Shell での e2e テスト:
                               # 本物のウィンドウと、仮想キーボードで押したホットキー
pwsh test/e2e/windows.ps1 -Bin target/debug/touchpad-snap.exe   # Windows 上で
pwsh test/e2e/windows.ps1 -Bin target/debug/touchpad-snap.exe -VirtualDesktops
                               # 2 つ目の仮想デスクトップ上のウィンドウも確認
nix build .#checks.x86_64-linux.module-eval
                               # NixOS モジュール・生成される設定・ユニットを評価する
                               # （KVM 不要）
nix build .#checks.x86_64-linux.vm-touchpad -L
                               # VM 内の本物の GNOME セッションで、仮想タッチパッドから
                               # スワイプする（KVM が必要）
nix flake check                # Nix のチェック一式: ビルド（cargo test を含む）、
                               # module-eval、VM テスト
```

e2e テストは動作中のデスクトップには一切触れません。専用の gnome-shell・D-Bus・HOME・dconf を立ち上げて使い捨てます。この gnome-shell は入力デバイスを読まないため、タッチパッドの経路は VM で確かめます。uinput の仮想タッチパッドから 3 本指スワイプを libinput・mutter・GNOME Shell に流し、Ctrl＋3 本指の左スワイプと Alt＋3 本指の上下スワイプでは、ワークスペースもアクティビティ画面も動かずにウィンドウが切り替わること、修飾キーなしの 3 本指では今まで通りどちらも動くことを確認します。D-Bus プロトコルは [gnome-extension/README.md](gnome-extension/README.md) を参照してください。CI では上記すべてを GNOME 46・48・49・50・51（Ubuntu、Debian、Fedora、Arch のコンテナ）、NixOS の VM、Windows Server 2022 で実行しています。

## ライセンス

MIT
