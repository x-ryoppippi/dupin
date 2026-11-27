# Recent items

Before you type, Dupin lists the apps and files you use most.

## Open it

Show the panel with an empty query.

## What you see

- Up to ten apps and files from your usage history, ranked by how recently and how often you used them.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/recent-items/recent-dark.png" />
<img alt="The panel before anything is typed, listing recent items: the Reports folder, Notes, Budget 2026.numbers, Calendar, Meeting notes.md and Maps." src="../images/recent-items/recent-light.png" width="680" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/recent-items/clip-dark.webp" />
<img alt="With nothing typed, the panel lists recent items: the Reports folder, Notes, Budget 2026.numbers and others; ↓ moves through them." src="../images/recent-items/clip-light.webp" width="680" />
</picture>

## What you can do

These rows have the regular app and [file actions](./file-actions.md); see [Keys](../usage.md#keys).

## Configuration

```toml
show_recent = true # show recently and frequently used apps and files before typing
```

| Key           | Default | Meaning                                                                                |
| ------------- | ------- | -------------------------------------------------------------------------------------- |
| `show_recent` | `true`  | `false` keeps the panel empty before typing. Settings → Search → **Show recent items** |
