# Strategy envelope — the shareable strategy document

A **strategy envelope** is the unit of sharing for a lemon strategy: a small,
versioned JSON document that wraps a strategy together with everything needed to
re-run it — a name, the strategy itself (as lemon `source` text or a lowered
`spec` tree), the engine config, and the universe window. It is pure data:
**an envelope never contains market data**, only *series names* and a *universe*,
so it is safe to store, publish, and re-run in another browser.

## Format

```jsonc
{
  "format": 1,                          // envelope version (this build: 1)
  "name": "Cheap quality rotation",
  "description": "…",                   // optional
  "author": "…",                        // optional

  // Exactly ONE of the following two:
  "source": "rank(pe) < 20",            // lemon text, or
  "spec":   { "op": "…", … },           // a lowered Expr tree

  "config": { "fee_ratio": 0.001, "delist_after": 10 },  // optional engine knobs
  "universe": {                         // names + window only, never data
    "from": 20180101,
    "to":   20251231,
    "symbols_hint": "sp500",            // a named universe the runner resolves
    "symbols": ["AAPL", "MSFT"]         // …or an explicit list
  },
  "engine_version": "yuzu-core 0.x"     // optional reproducibility pin
}
```

- `format` — must equal the envelope version this build understands (currently
  `1`). Bumped only on a breaking change to the envelope shape, never for a new
  engine op.
- `source` **xor** `spec` — provide exactly one. `source` is human-writable lemon;
  `spec` is the lowered `Expr` tree (`schema/lemon-spec.schema.json`). Both lower
  to the same tree the engine evaluates.
- `config` — the engine `BacktestConfig` knobs (fees, slippage, delist, benchmark,
  bootstrap, …). Interpreted by the engine/runner; opaque to the language layer.
- `universe` — the date window and the symbol universe (a hint like `"sp500"`
  the runner resolves, or an explicit `symbols` list). **No prices or
  fundamentals** — the runner supplies data locally (e.g. a BYO-key fetch, see
  [`fmp-data-source.md`](./fmp-data-source.md)).

The machine-readable schema is
[`schema/strategy-envelope.schema.json`](../schema/strategy-envelope.schema.json).

## Validation

```bash
lemon check strategy.json        # one or more files (or stdin)
```

`lemon check` reports actionable errors and exits non-zero on any problem, so a
registry or the web app can reject malformed submissions before storing them. It
checks, in order:

1. the document is valid JSON with no unknown envelope keys;
2. `format` matches this build;
3. `name` is non-empty;
4. exactly one of `spec` / `source` is present;
5. `source` parses as lemon **or** `spec` deserializes into a valid `Expr`
   (unknown ops, missing required fields, and wrong shapes are all caught);
6. `config`, if present, is an object.

The same check is available as a library entry point,
`lemon::envelope::check(doc) -> Result<Checked, Vec<String>>`, returning the
resolved `Expr` tree on success — and in the browser via the `lemon-wasm`
`check_envelope(doc)` export, which returns
`{"ok":true,"name","spec"}` or `{"ok":false,"errors":[…]}` (never throws), so a
web app / registry can validate submissions client-side.

## Reproducibility

An envelope reproduces **bit-for-bit** only when three things match:

1. **the same engine version** — pin it in `engine_version`; op semantics are
   fixed by golden tests, but a version bump can add or change behavior;
2. **the same data panels** — same series, same symbols, same date window,
   same adjustment. The envelope names the universe; it does not carry the data,
   so two runners must load the *same* panels to agree (this is the crux of the
   "run it yourself" model — see the data-licensing note in issue #30);
3. **the same `config`** — fees, slippage, delist rules, benchmark, and bootstrap
   seed all affect the equity curve.

The strategy tree itself round-trips losslessly: `source` → `Expr` (via `lemon
check` / `lemon::parse`) is deterministic, and a `spec` envelope stores the tree
directly.
