# Calculator

Works out calculations and unit or currency conversions typed in the main search.

| Keyword       | None: it answers the main search                                      |
| ------------- | --------------------------------------------------------------------- |
| Command       | The built-in `calculator.evaluate`                                    |
| On by default | Yes (`calculator = true`); currencies only with `calculator_currency` |
| User docs     | [Calculator](../../docs/features/calculator.md)                       |

A port of the former native Calculator; see [Built-ins as bundled extensions](../../docs/design/extension-platform.md#built-ins-as-bundled-extensions). The built-in registers no keyword, so the `calc` keyword in `package.json` is never typed.

## How it's built

- **`search(query)`**: decides whether the query reads as a calculation, has the `calculator` service work it out, and returns one row with the input as its title and the result as its `answer`. A currency conversion adds the rates' date, marked as cached when Dupin couldn't refresh them in the past 12 hours.
- **Actions:** **Copy number** copies the bare number, without units or grouping.
- **Capabilities:**
  - `calculator: ["evaluate"]`, for `calculator.evaluate(input)`: fend, with the European Central Bank's rates, which Dupin itself fetches in the background. The package never reaches the network.
  - `query: ["read"]`, to see what is typed in the main search. Dupin hands over only a query with a digit in it.

## Files

| Path              | What                                                                                                               |
| ----------------- | ------------------------------------------------------------------------------------------------------------------ |
| `package.json`    | The manifest.                                                                                                      |
| `main.ts`         | `search`, what counts as a calculation, and how currency answers read.                                             |
| `fixtures/*.json` | Arithmetic and units, what isn't a calculation, currencies, cached rates, no rates, rates refreshed between steps. |

## Test it

```sh
dupin ext test extensions/calculator
dupin ext render extensions/calculator extensions/calculator/fixtures/01-arithmetic-and-units.json arithmetic.png
```

Each fixture answers `calculator.evaluate` from `replies.calculator`, by expression, with `null` for one fend doesn't answer; any other expression fails as `unavailable`. `render` types the query in the main search, as the launcher hands it over. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
