# Calculator

Type a calculation in app search, such as `12*3`, `2^10`, `(1+2)/3` or
`15% of 200`. Convert units with `5 km to mi`, `100 f to c` or `1 GiB in MB`.
The answer appears above apps. Press Enter to copy the displayed number without
units or the approximation label. Turn Calculator off in **Settings → Search**.

## Configuration

Set `calculator = false` or turn off **Calculator** in Settings → Search to hide calculator results. Changes apply immediately.

```toml
calculator = true # show calculations and unit conversions in app search
calculator_currency = false # allow downloading exchange rates when Calculator is on
```

Turn on **Currency conversion** in Settings → Search to convert currencies in
app search. Try `100 USD to JPY`, `€50 in GBP` or `10 GBP` (which converts to EUR).
Dupin checks the European Central Bank's public reference feed in the background
at most once every 12 hours. The request contains no search text. The calculator
uses the last good rates offline and shows their observation date. It labels
rates as cached if they have not been refreshed in the past 12 hours.
Rates are stored in `~/Library/Caches/Dupin/calculator`, or in
`$XDG_CACHE_HOME/dupin/calculator` when that variable is set.
