# UAT checklist

Run `just uat` and work down this list. Each item says what to look for and
which scenario in the walkthrough covers it.

The script asserts its own expectations as it goes — including that the things
which *should* be refused still are — and prints a red line plus a non-zero exit
if anything behaves differently. A green "All 10 scenarios behaved as expected"
means the mechanics work.

`just test-cli` goes further: it drives the binary and the web UI and checks the
actual output — messages, exit codes, the arithmetic in a section, HTML
escaping. Run it first; if it is green, everything below is about judgement
rather than mechanics: whether the output is *right*, and whether it is usable.

```console
just uat            # the full walkthrough        (or: uat/run.sh)
just walk           # same, pausing between steps (or: uat/run.sh --step)
just ui             # the web UI                  (or: uat/run.sh ui)
just sandbox        # poke at the sandbox         (or: uat/run.sh shell)
just gm show CH-150 # run one command against the sandbox
just clean          # remove build output and the sandbox
```

The sandbox is `uat/workspace/`, rebuilt on each run and gitignored. Nothing
outside it is touched.

---

## 1. The file format

| # | Check | Where | ✓ |
|---|---|---|---|
| 1.1 | One file is created. No config directory, no server, no other install. | Scenario 1 | |
| 1.2 | `gm info` reports CRS, vertical datum, model and material counts. | Scenario 2 | |
| 1.3 | Levels and depths in `gm show` agree: depth = surface − level, and thicknesses sum to the model extent. | Scenario 2 | |
| 1.4 | The deepest layer has a base. The section closes at "(base of model)". | Scenario 2 | |
| 1.5 | Bounded parameters show value **and** range, e.g. `20 (19-21) kN/m3`. | Scenario 2 | |
| 1.6 | The London Clay Su profile shows both points and says what depth is measured from. | Scenario 2 | |
| 1.7 | Exported JSON round-trips: `gm export`, then import into a fresh file, gives the same models. | Scenario 9 | |

**Judgement call:** is `Bounded` (`value`/`lower`/`upper`/`unit`, optionally a
depth `profile`) the right primitive for how your team records parameters? It is
the one thing that would be expensive to change later.

## 2. Editing and history

| # | Check | Where | ✓ |
|---|---|---|---|
| 2.1 | A plain `sqlite3 UPDATE` is picked up by `gm status` with no import step. | Scenario 3 | |
| 2.2 | `gm diff` names the model, the layer, and the size of the move (`+0.60 m`). | Scenario 3 | |
| 2.3 | `gm log` is newest-first and shows author, time and message. | Scenario 4 | |
| 2.4 | `gm show <model> --rev <hash>` returns the *old* geometry, not the current one. | Scenario 4 | |
| 2.5 | `gm checkout HEAD --force` discards a bad working-tree edit. | Scenario 5 | |

**Judgement call:** is "the tables are the working tree" the right model, or
would you rather editing went only through `gm`? It is what makes any SQLite
client a valid editor, but it also means a careless `UPDATE` can put the working
tree in a state that will not commit.

## 3. Validation

| # | Check | Where | ✓ |
|---|---|---|---|
| 3.1 | An inverted succession is reported as an **error**, naming model, field path and the levels involved. | Scenario 5 | |
| 3.2 | `gm commit` refuses while that error stands. | Scenario 5 | |
| 3.3 | After rollback, the six-chainage route validates with **0 errors, 0 warnings**. | Scenario 5 | |

**Judgement calls**, and the ones most worth your attention:

- Is the **error / warning** split drawn in the right place? Errors block a
  commit; warnings do not. Currently errors are: layers out of order or
  zero-thickness, base level above the deepest layer, a layer referencing an
  undefined material, a value outside its own bounds, a friction angle outside
  0–90°.
- Are the **warning thresholds** sensible for your ground? Unit weight outside
  10–30 kN/m³, γ_w outside 9–11 kN/m³. Say if these are wrong.
- Is anything **missing** that ought to be caught? Try breaking a model in the
  sandbox shell and see whether it is noticed.
- Is anything caught that **should not** be? A validator that rejects plausible
  ground gets switched off.

## 4. Working with other people

| # | Check | Where | ✓ |
|---|---|---|---|
| 4.1 | `gm clone` produces a copy holding the whole history. | Scenario 6 | |
| 4.2 | `gm push` fast-forwards the other copy and reports what moved. | Scenario 7 | |
| 4.3 | `gm pull` on a diverged history changes nothing and says what to do next. | Scenario 7 | |
| 4.4 | Edits to **different** chainages merge with no questions, and both survive. | Scenario 7 | |
| 4.5 | Edits to the **same** chainage conflict, nothing is written, and the working tree stays clean. | Scenario 8 | |
| 4.6 | The merge commit shows in `gm log` tagged as a merge, with two parents. | Scenario 7 / web UI | |

**Judgement call:** on a conflict you currently resolve by hand — check out one
side, or edit and re-commit. Is that acceptable, or do you need assisted
resolution before this is usable on a live job?

**Known limit to confirm you can live with:** a remote is a **filesystem path**.
This works over a shared drive or a synced folder, but there is no network
transport yet.

## 5. Integrity

| # | Check | Where | ✓ |
|---|---|---|---|
| 5.1 | `gm verify` re-hashes every object and reports OK. | Scenario 9 | |
| 5.2 | Object count grows by ~2 per commit, not by the whole file. Compare `gm info` before and after a commit. | Scenario 2 vs 9 | |

To satisfy yourself that verification is real rather than decorative, corrupt a
stored object by hand and check that it is caught:

```console
just sandbox
sqlite3 a13.gm "UPDATE gm_blob SET content = '{}' WHERE hash = (
                  SELECT hash FROM gm_blob
                   WHERE hash NOT IN (SELECT hash FROM gm_commit) LIMIT 1);"
gm verify          # must report a corrupt object
gm clone a13.gm copy.gm    # must refuse rather than copy the corruption
```

## 6. Reading it from other tools

| # | Check | Where | ✓ |
|---|---|---|---|
| 6.1 | DuckDB reads the file via `ATTACH ... (TYPE sqlite)` with no export step. | Scenario 10 | |
| 6.2 | `layer_intervals` gives depths and thicknesses without hand arithmetic. | Scenario 10 | |
| 6.3 | The flat `gm.file/1` JSON is legible enough to hand to someone with no tooling. | Scenario 9 | |

Try your own query in the sandbox shell — `gm sql "..."`, or open the file in
any SQLite browser. The tables to look at are `ground_models`, `ground_layers`,
`materials` and the `layer_intervals` view.

## 7. The web UI

`just ui`, then <http://127.0.0.1:8765/>.

| # | Check | Where | ✓ |
|---|---|---|---|
| 7.1 | The drawn section is to scale and matches the numbers in the layer table. | any model page | |
| 7.2 | The buried channel reads correctly along the route: absent at CH-000, thickest at CH-150, gone again by CH-250. | CH-000 → CH-250 | |
| 7.3 | The water table sits at the right depth, with the conventional marker. | any model page | |
| 7.4 | A material keeps the same colour on every page and in the index swatches. | index vs model pages | |
| 7.5 | History → a commit shows what that revision changed. | /history | |
| 7.6 | Validation issues appear on the model page they concern, not only on their own page. | model page | |
| 7.7 | It refuses to write: `curl -X POST http://127.0.0.1:8765/` returns 405. | | |
| 7.8 | It is not reachable from another machine (loopback only). | | |

**Judgement call:** read-only is deliberate — editing goes through the CLI where
it is validated and attributed. Is a read-only UI enough, or do you need to edit
in the browser?

---

## Signing off

Record against each section: **accepted**, **accepted with comments**, or
**rejected**, and note anything you want changed.

The three decisions hardest to reverse later, so worth deciding now:

1. **`Bounded` as the numeric primitive** — every parameter in the format is
   shaped by this.
2. **Content hashes as identity** — models are addressed by `model_key` and
   materials by `material_key`, and the surrogate `id` columns are derived from
   content. Changing this would change every hash in every existing file.
3. **The error / warning line in validation** — moving something from warning to
   error later will reject files that were valid when they were written.

Everything in `docs/format.md` under "Deviations from `schema.dbml`" is also
open to challenge; each one lists the reasoning it rests on.
