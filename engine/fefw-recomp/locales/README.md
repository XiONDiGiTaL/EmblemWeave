# In-game menu translations

The F1 settings menu reads optional translation tables from `locales/<name>.ini`
next to `fefw_rt.dll` (or `fefw.exe`, wherever the binary lives).

## Enabling a language

By default (`[gfx] ui_language = auto` in `fefw.ini`, or no entry) the menu uses
the table for the Windows display language when there is one: `zh.ini` for
Simplified Chinese, and for any other language a table named after it
(`pt-BR.ini`, then `pt.ini`). Otherwise it is in English.

A language picked in the launcher (the button at the foot of its sidebar) is
written to `ui_language` and covers the menu as well. Setting the key by hand to
a table name without `.ini` also works; the launcher keeps a name it does not
know about. `en` forces English. The menu reads the setting when the game starts.

## Table format

UTF-8 text, one pair per line:

```
English source string = Translated string
```

- The key is the exact English string as it appears in the menu (trimmed).
- Lines starting with `#` or `;` are comments.
- A tab can be used as the separator instead of `=` for keys that themselves
  contain an equals sign.
- A missing key, a partial table, or a broken file is safe: the menu simply
  falls back to the English string.
- Format specifiers (`%s`, `%d`, `%.1f`, `%%`, ...) in a translation must match
  the English key in type and order. The menu hands some strings to printf, so
  an entry whose specifiers differ is dropped at load, stays English, and is
  listed in the log. `%%` is a literal `%` and needs no counterpart beyond the
  key's own `%%`.
- A `*` width or precision reads an argument of its own, so it is part of the
  comparison: `%*d` must stay `%*d` and may not become `%d`, and `%.*f` may not
  become `%.2f`. Both sides are compared as a list of arguments
  (`%*d` is `*` then `d`, `%ls` is `ls`), so moving an argument from the width
  to the value, or reordering two conversions, is caught as well.
- A space is a printf flag, so `99% de` reads as the conversion `% d`: such a
  translation is dropped. Write the percent as `%%` in a string that has format
  specifiers.
- `%n` is refused outright: it writes through a pointer the caller never
  passed, and no menu string uses one, so any entry that contains it is dropped
  whatever the English key says.
- A percent that is not a conversion is plain text and is left alone, which is
  why prose such as `1% low` or `100% (native)` needs no escaping.

## Where the tables live

`make-release.ps1` puts this folder into the release package, and EmblemWeave
copies any `<name>.ini` it finds there into the run folder
(`<game folder>/locales/`) before the first start and before every Play. A file
that is already there is left alone, so a translation the player edited or
added wins over the shipped one.

## Keeping the shipped table up to date

`zh.ini` is the exception: the launcher carries the current one inside itself and
replaces a copy the player never edited. It knows an untouched copy from the
hash it records in `zh.ini.shipped` beside it, or, for releases before that
record existed, from a built-in list. An edited `zh.ini` is kept as it is; to
take the new translations instead, delete it and open the launcher, or merge
the two by hand.

```
<package>/locales/zh.ini   ->   <game folder>/locales/zh.ini
```

Delete the file instead if the player's own copy is the one to keep. Both the
package copy and the run-folder copy are plain text, so a local edit can be
merged by hand before it is dropped in. The same rule holds for a table the
player wrote themselves: it lives in the run folder, and EmblemWeave never
touches it.

## Font

Chinese/Japanese/Korean text needs a font that has those glyphs. The runtime
loads the first font it finds from:

1. `[gfx] ui_font` in `fefw.ini` (a path to a `.ttf`/`.ttc` font file),
2. `fonts/ui.ttf` next to the binary,
3. `C:\Windows\Fonts\msyh.ttc` (Microsoft YaHei),
4. `C:\Windows\Fonts\simhei.ttf`, then `Deng.ttf`.

Without one of these the menu renders boxes for CJK characters; English
tables need no font setup.

## Ship checklist for a new language

1. Copy `zh.ini` as a starting point or translate from the English keys.
2. Keep the file UTF-8 **without** BOM.
3. Name it after the language code (`de.ini`, `fr.ini`, ...).
4. Test with `[gfx] ui_language = <code>` and confirm the menu after a game
   restart.
