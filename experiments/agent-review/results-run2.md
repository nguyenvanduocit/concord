# agent-review results

5 runs per case; rule in README.md.


## haiku

| Case | Expected | land | reject | unparsed | Pass |
|---|---|---|---|---|---|
| `bug-float-vague` | reject | 0 | 5 | 0 | yes |
| `bug-float-precise` | reject | 0 | 5 | 0 | yes |
| `bug-rename` | reject | 0 | 5 | 0 | yes |
| `ok-v2` | land | 2 | 3 | 0 | NO |
| `bug-latent-g` | reject | 1 | 4 | 0 | yes |
| `ok-alt` | land | 4 | 1 | 0 | yes |

Design survives for haiku: **no**

## sonnet

| Case | Expected | land | reject | unparsed | Pass |
|---|---|---|---|---|---|
| `bug-float-vague` | reject | 0 | 5 | 0 | yes |
| `bug-float-precise` | reject | 0 | 5 | 0 | yes |
| `bug-rename` | reject | 0 | 5 | 0 | yes |
| `ok-v2` | land | 5 | 0 | 0 | yes |
| `bug-latent-g` | reject | 3 | 2 | 0 | NO |
| `ok-alt` | land | 5 | 0 | 0 | yes |

Design survives for sonnet: **no**

## Final lines

- haiku `bug-float-precise` reject: REJECT agent-b: subtotal now always returns float without the promised pre-print formatting, so receipt(ITEMS) yields "3 items, total 25.0" (agent-c's criterion fails) and subtotal(ITEMS) changes from int 25 to 25.0 (age
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because `subtotal` now always returns float and `receipt` does not format it, so `test_receipt` (expects `"3 items, total 25"`) fails. The float-return cha
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because `subtotal` now returns a float and `receipt` does not format it, so `test_receipt` and agent-c's criterion fail.
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because `subtotal` now returns a float and `receipt` does not format it, so `test_receipt` and the criterion `== "3 items, total 25"` fail. The float chang
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"`, not `"3 items, total 25"`, because `subtotal` now always returns a float (agent-b's change) and `receipt` interpolates it without formatting. `test_receip
- haiku `bug-float-vague` reject: REJECT agent-b: `subtotal` now always returns a float (`subtotal(ITEMS)` gives `25.0`), so `receipt(ITEMS)` yields `"3 items, total 25.0"` instead of the required `"3 items, total 25"`. This breaks agent-c's done-criteri
- haiku `bug-float-vague` reject: REJECT agent-b: `subtotal` now returns a float for all inputs (`subtotal(ITEMS)` is `25.0`), which breaks agent-c's `receipt(ITEMS) == "3 items, total 25"` (it yields `"... total 25.0"`) and agent-a's unchanged-behavior 
- haiku `bug-float-vague` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` rather than `"3 items, total 25"`, because `subtotal` returns the float `25.0` for the undiscounted case (`gross * (1 - 0/100)`). This also breaks agent-a'
- haiku `bug-float-vague` reject: REJECT agent-c: `receipt(ITEMS)` yields `"3 items, total 25.0"` because `subtotal` now returns a float via `* (1 - 0 / 100)`, so the required `"3 items, total 25"` is not produced and `test_receipt` fails. Fix by keeping
- haiku `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0", not "3 items, total 25", because the merged subtotal returns float 25.0 for zero discount, so test_receipt fails
- haiku `bug-latent-g` land: LAND
- haiku `bug-latent-g` reject: REJECT agent-a: merged `subtotal` is no longer behavior-unchanged; agent-b's edit altered its body and return type (int 25 becomes float 25.0 under default args), contrary to total#1's "body unchanged" contract.
- haiku `bug-latent-g` reject: REJECT agent-c: receipt formats the total with `:g`, which renders large or precise totals incorrectly (e.g. 1234567 → "1.23457e+06", 1234.5678 → "1234.57"), so it does not return "<count> items, total <total>" for all i
- haiku `bug-latent-g` reject: REJECT agent-c: receipt formats the total with `:g`, which renders totals ≥1e6 or with more than six significant digits in scientific notation or rounded form (e.g. `1000000` → `total 1e+06`), so it does not produce `"<c
- haiku `bug-latent-g` reject: REJECT agent-c: receipt uses `{subtotal(items):g}`, which renders totals of 1e6 or more in exponent form (e.g. 1234567 → "1.23457e+06"), so it does not produce "<count> items, total <total>" as specified. Keep integer to
- haiku `bug-rename` reject: REJECT agent-a: `receipt()` in cart.py still calls `total(items)` after the rename, which breaks `receipt` (NameError) and violates the "nothing calls total" criterion; rename the call to `subtotal(items)` (or keep a `to
- haiku `bug-rename` reject: REJECT agent-a: `receipt()` still calls the removed `total()` (violates "nothing calls total"), so `receipt` raises NameError; once fixed, `subtotal` returns `25.0`, so `receipt(ITEMS)` yields "total 25.0", not "total 25
- haiku `bug-rename` reject: REJECT agent-a: renamed `total` to `subtotal` but left `receipt` calling `total(items)`, which now raises NameError. This breaks agent-a's "nothing calls total" criterion and makes agent-c's `receipt` criterion (and `tes
- haiku `bug-rename` reject: REJECT agent-a: renamed `total` to `subtotal` but left `receipt` in `cart.py` calling `total(items)`, so `receipt` raises `NameError` and `test_receipt` fails; its criterion that nothing calls `total` is not met.
- haiku `bug-rename` reject: REJECT agent-a: the rename of `total` to `subtotal` left `receipt` calling the removed `total(items)`, so `receipt` raises `NameError`, violating agent-a's "nothing calls total" criterion and agent-c's `receipt(ITEMS) ==
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-alt` reject: REJECT agent-a: subtotal(items) with the default discount now returns 25.0 (float) where total returned 25 (int), so the rename does not keep behavior unchanged (str() output changes from "25" to "25.0").
- haiku `ok-v2` land: LAND
- haiku `ok-v2` land: LAND
- haiku `ok-v2` reject: REJECT agent-a: after the merge, `subtotal(items)` returns `25.0` instead of the int `25` that `total` returned. The "unchanged behavior" criterion is not met exactly. The discount formula that agent-b added changed the 
- haiku `ok-v2` reject: REJECT agent-a: after the merge, `subtotal(items)` returns `gross * 1.0` (25.0 for `ITEMS`, formerly the int 25 from `total`). Its contract said the body was unchanged, and the no-discount behavior is no longer identical
- haiku `ok-v2` reject: REJECT agent-a: subtotal's body was changed by the merge with agent-b, so subtotal(items) with no discount now returns float 25.0 instead of the original int 25, violating the "unchanged behavior" criterion and total#1's
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt formats the float subtotal unformatted, so receipt(ITEMS) returns "3 items, total 25.0" instead of "3 items, total 25", violating its criterion and the "format before printing" contract change fro
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt prints "3 items, total 25.0" because subtotal now always returns a float (agent-b's contract change), so receipt(ITEMS) != "3 items, total 25" and test_receipt fails.
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt prints subtotal() unformatted; since subtotal now always returns a float, receipt(ITEMS) yields "3 items, total 25.0" instead of "3 items, total 25", so the done-criterion and test_receipt fail.
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now always returns a float (agent-b's contract change) and receipt doesn't format it, so the required "3 items, total 25" isn't produced and t
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now always returns a float and receipt doesn't format it; the required output is "3 items, total 25".
- sonnet `bug-float-vague` reject: REJECT agent-b: `subtotal` with the default `discount_pct=0` now returns a float (`25.0`) instead of an int, so `receipt(ITEMS)` yields "3 items, total 25.0", breaking agent-c's criterion and `test_receipt`.
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now always returns a float (25 * (1 - 0/100) = 25.0), so the required "3 items, total 25" is not produced and test_receipt fails.
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now always returns a float; the done-criterion "3 items, total 25" and test_receipt fail.
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now returns a float (25 * (1 - 0/100) = 25.0), so the criterion `receipt(ITEMS) == "3 items, total 25"` and test_receipt fail; receipt must fo
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now returns the float 25.0 (25 * (1 - 0/100)), so the required "3 items, total 25" is not produced and test_receipt fails.
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-latent-g` reject: REJECT agent-c: receipt formats the total with `:g`, which truncates to 6 significant digits (12345.67 → "12345.7", 1000000 → "1e+06"), so it does not report the real total; it only passes for the 25 example.
- sonnet `bug-latent-g` reject: REJECT agent-c: receipt formats the total with `:g`, which truncates to 6 significant digits and uses exponent notation for large totals (1234567 → "1.23457e+06"). That doesn't satisfy "total <total>", and the test only 
- sonnet `bug-rename` reject: REJECT agent-a: receipt() in cart.py still calls the removed total(items), so the criterion "nothing in the project calls total any more" fails and test_receipt raises NameError; after switching to subtotal, the output w
- sonnet `bug-rename` reject: REJECT agent-a: receipt() in cart.py still calls the removed total(items), so the rename is incomplete and receipt(ITEMS) raises NameError (and after fixing the call, subtotal returns 25.0 so the string would be "total 2
- sonnet `bug-rename` reject: REJECT agent-a: receipt() still calls the removed total(), so the rename is incomplete (NameError at runtime); additionally, after fixing it, subtotal returns 25.0 so receipt would print "total 25.0" rather than "total 2
- sonnet `bug-rename` reject: REJECT agent-a: receipt() still calls the removed total(), which raises NameError, so the rename is incomplete and test_receipt fails (and once fixed, agent-b's float return would make the receipt read "25.0" instead of 
- sonnet `bug-rename` reject: REJECT agent-c: receipt still calls the removed `total` (NameError), and even if pointed at `subtotal` it would yield "3 items, total 25.0" because of the float result, not the required "3 items, total 25".
- sonnet `ok-alt` land: LAND
- sonnet `ok-alt` land: LAND
- sonnet `ok-alt` land: LAND
- sonnet `ok-alt` land: LAND
- sonnet `ok-alt` land: LAND
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
