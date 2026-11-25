# Calculator

Type a calculation in app search, such as `12*3`, `2^10`, `(1+2)/3` or
`15% of 200`. Convert units with `5 km to mi`, `100 f to c` or `1 GiB in MB`.
The answer appears above apps. Press Enter to copy the displayed number without
units or the approximation label. Turn Calculator off in **Settings → Search**.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/calculator/arithmetic-dark.png" />
<img alt="The panel with “12*3” typed and its answer, 36." src="../images/calculator/arithmetic-light.png" width="680" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/calculator/units-dark.png" />
<img alt="The panel with “5 km to mi” typed and the answer, approx. 3.1068559612 mi." src="../images/calculator/units-light.png" width="680" />
</picture>

Calculator is a [bundled extension](./extensions.md#bundled-extensions), `calculator`,
that works each calculation out with Dupin's calculator. Dupin hands it what you type
in the main search only when it has a digit in it, so names and words never reach it.
Settings → Extensions lists it as a built-in command, with its `query` and `calculator`
grants; revoking either stops it answering. `dupin ext eject calculator` copies it to
your extensions folder to change it. A changed copy sees nothing you type until you
turn it on and allow it `query` in Settings → Extensions.

## Configuration

Set `calculator = false` or turn off **Calculator** in Settings → Search to hide calculator results. Changes apply immediately.

```toml
calculator = true # show calculations and unit conversions in app search
calculator_currency = false # allow downloading exchange rates when Calculator is on
```

Turn on **Currency conversion** in Settings → Search to convert currencies in
app search. Try `100 USD to JPY`, `€50 in GBP` or `10 GBP` (which converts to EUR).
Dupin itself checks the European Central Bank's public reference feed in the background
at most once every 12 hours, and hands the rates to the calculator; the extension never
reaches the network. The request contains no search text. The calculator
uses the last good rates offline and shows their observation date. It labels
rates as cached if they have not been refreshed in the past 12 hours.
Rates are stored in `~/Library/Caches/Dupin/calculator`, or in
`$XDG_CACHE_HOME/dupin/calculator` when that variable is set.
