# Emoji & Symbols

Type `emoji`, then Space, to pick an emoji or symbol ("Emoji & Symbols"). With nothing typed, the list starts with Smileys & Emotion and runs in Unicode's order under its groups: People & Body, Animals & Nature, Food & Drink, Travel & Places, Activities, Objects, Symbols and Flags, then Text Symbols, such as `€`, `→`, `±` and `≠`, that aren't emoji. Type to narrow it by name or keyword in English or Japanese: `thumbs`, `euro`, `にっこり` or `日本` all find what you would expect. The groups give way to one list ranked as apps are, with the emoji you pick most often for a query first. Backspace on an empty query returns to app search.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/emoji/browse-dark.png" />
<img alt="Emoji &amp; Symbols before anything is typed: the Smileys &amp; Emotion group, with grinning face selected and its details shown." src="../images/emoji/browse-light.png" width="680" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/emoji/search-dark.png" />
<img alt="Emoji &amp; Symbols with “thumbs” typed: thumbs up selected with its name, Japanese name, code points and keywords, then thumbs down and crossed fingers." src="../images/emoji/search-light.png" width="680" />
</picture>

The selected row shows the character, its name, its Japanese name, its code points and the keywords that find it. Enter copies the character and closes the panel. Right Arrow, ⌘K or the Actions link opens its other actions:

| Action                             | Does                                                                                                                                                                                 |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Copy                               | Copies the character. Enter does this.                                                                                                                                               |
| Paste                              | Pastes it into the app you were using, which needs Accessibility access; without it, the character is copied and a notice explains why ([Permissions](./permissions.md#extensions)). |
| Copy 👍🏻 light skin tone, and so on | For an emoji that takes a skin tone, copies it in each of the five tones, light to dark.                                                                                             |
| Send to…                           | Hands the character to another feature, as any text result does ([Send to](./send-to.md)).                                                                                           |

The list holds Emoji 15.1, which macOS 14.4 and later draw, with names and keywords from CLDR 45.

## Turning it on and off

Emoji & Symbols is a [bundled extension](./extensions.md#bundled-extensions) called `emoji`: it ships inside Dupin and is on from the start, whether or not the extensions folder is on. Its switch is in Settings → Extensions. It declares one capability, `clipboard:paste`, which Paste needs and which the bundled copy holds from the start; Settings → Extensions can revoke it, and Settings → Permissions lists it under Accessibility. Typing never runs extension code: Dupin holds the whole list and matches it itself.

To change it, such as to add your own keywords or other characters, copy it into the extensions folder and edit it there:

```sh
dupin ext eject emoji
```

Your copy replaces the bundled one while the extensions folder is on; deleting it brings the bundled one back. [Extensions](./extensions.md#bundled-extensions) describes how a changed copy is treated.

## How it is built

The package is [`extensions/emoji`](../../extensions/emoji): `main.ts` hands Dupin the list through [`items()`](./extension-api.md#itemsctx) from `catalogue.ts`, which `scripts/generate.nu` writes from Unicode's `emoji-test.txt` and CLDR's English and Japanese annotations. Fully-qualified emoji outside the Component group are listed once each; an emoji's five single-tone variants become its skin-tone actions, and variants that mix tones are left out. Text Symbols are the characters CLDR annotates that aren't emoji or ASCII. `fixtures/` holds the [`dupin ext test`](./extensions.md#testing-and-snapshots) replays of browsing, English and Japanese searches, symbols, and each action.

Moving to a newer Unicode or CLDR release means running the generator with `--emoji` and `--cldr` and committing what it writes. Only move as far as the oldest supported macOS draws.
