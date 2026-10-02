# Keyboard

Shortcuts are settings, not code. Every action in `app::actions` has one binding in
`settings/keyboard.rs`, all of them changeable in the settings window, and all of them
reachable from the main menu as well.

## Defaults

| Action | Shortcut |
| --- | --- |
| Play / pause | `Space` |
| Next track | `N` |
| Previous track | `P` |
| Seek forward | `]` |
| Seek backward | `[` |
| Volume up | `Ctrl+Up` |
| Volume down | `Ctrl+Down` |
| Mute | `M` |
| Toggle shuffle | `S` |
| Cycle repeat | `R` |
| Toggle favourite | `F` |
| Focus search | `Ctrl+K` |
| Select all | `Ctrl+A` |
| Add library folder | `Ctrl+O` |
| Rescan library | `Ctrl+R` |
| Rescan folder | `Ctrl+Shift+R` |
| Import playlist | `Ctrl+Shift+I` |
| New playlist | `Ctrl+N` |
| Settings | `Ctrl+,` |
| Toggle sidebar | `Ctrl+B` |
| Tracks | `Ctrl+1` |
| Albums | `Ctrl+2` |
| Artists | `Ctrl+3` |
| Folders | `Ctrl+4` |
| Favourites | `Ctrl+5` |
| Playlists | `Ctrl+6` |
| Track information | `Ctrl+I` |
| Copy path | `Ctrl+Shift+C` |
| Show in file manager | `Ctrl+Shift+E` |
| Quit | `Ctrl+Q` |

Volume is on `Ctrl+Up` and `Ctrl+Down` rather than bare arrows: the arrows belong to list
navigation, and a shortcut that changes meaning depending on which view is open is worse than
one extra modifier.

## Writing a binding

A binding is `Modifiers+Key`. Modifiers are `Ctrl`, `Alt`, `Shift` and `Super`, in any order
and any case; the key is a name (`Up`, `Page_Down`, `F5`, `Space`, `Escape`, `BracketLeft`) or
a single character (`q`, `[`, `,`). Both spellings of the punctuation keys work: `[` and
`BracketLeft` mean the same key.

`Shortcut` normalises what it is given — `n`, `N` and `Ctrl+N` describe one key — and rejects
bindings with no key or with an unknown modifier part. Bindings that clash with another action
are reported in the settings window rather than silently overwriting each other.

## From a binding to GTK

`app::ui::accels` converts a binding to the string `set_accels_for_action` wants: modifiers in
the fixed order `<Ctrl><Alt><Shift><Super>` followed by the keyval nick, with no separator.
Single letters go down (`n`), named keys keep their capitalisation (`Up`, `F5`, `Page_Up`), and
punctuation uses the GDK spelling (`space`, `comma`, `bracketleft`). The conversion is a pure
function and is unit tested, because a malformed accelerator is silently ignored by GTK and
would otherwise only show up as a key that does nothing.

## Rebinding

The settings window records the key you press into the row you chose. `Restore Defaults` puts
the table above back and re-installs the accelerators immediately, so there is no restart.