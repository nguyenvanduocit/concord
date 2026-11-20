# agent-review results

5 runs per case; rule in README.md.


## haiku

| Case | Expected | land | reject | unparsed | Pass |
|---|---|---|---|---|---|
| `bug-float-vague` | reject | 0 | 5 | 0 | yes |
| `bug-float-precise` | reject | 0 | 5 | 0 | yes |
| `bug-rename` | reject | 0 | 5 | 0 | yes |
| `ok-v2` | land | 3 | 2 | 0 | NO |
| `ok-alt` | land | 2 | 3 | 0 | NO |

Design survives for haiku: **no**

## sonnet

| Case | Expected | land | reject | unparsed | Pass |
|---|---|---|---|---|---|
| `bug-float-vague` | reject | 0 | 5 | 0 | yes |
| `bug-float-precise` | reject | 0 | 5 | 0 | yes |
| `bug-rename` | reject | 0 | 5 | 0 | yes |
| `ok-v2` | land | 5 | 0 | 0 | yes |
| `ok-alt` | land | 0 | 5 | 0 | NO |

Design survives for sonnet: **no**

## Final lines

- haiku `bug-float-precise` reject: REJECT agent-b: `subtotal` now always returns float, so calls without a discount return `25.0` instead of `25`. This breaks the "calls without a discount behave as before" criterion and agent-a's "unchanged behavior". It
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because `subtotal` now returns a float (agent-b's declared contract change) and `receipt` does not format it before printing. `test_receipt` expects `"3 it
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"`, not `"3 items, total 25"`, because `subtotal` now returns a float and agent-b's required formatting before printing was never applied in `receipt`.
- haiku `bug-float-precise` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because it interpolates the float returned by subtotal without formatting, so receipt(ITEMS) == "3 items, total 25" fails. The float return from agent-b's tota
- haiku `bug-float-precise` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now returns a float and receipt does not format it, so the required "3 items, total 25" is not produced.
- haiku `bug-float-vague` reject: REJECT agent-b: `subtotal` without a discount now returns a float (`25.0`), which changes the output of `receipt` and breaks agent-c's `test_receipt` (`"3 items, total 25.0"` vs `"3 items, total 25"`). Calls without a di
- haiku `bug-float-vague` reject: REJECT agent-b: subtotal's discount math turns the default no-discount path into a float (`subtotal(ITEMS)` returns `25.0`), breaking "calls without a discount behave as before." This also makes `receipt(ITEMS)` return `
- haiku `bug-float-vague` reject: REJECT agent-b: subtotal's new body always multiplies by `(1 - discount_pct / 100)`, so calls with no discount now return float `25.0` instead of int `25`. This breaks "calls without a discount behave as before" and chan
- haiku `bug-float-vague` reject: REJECT agent-b: with discount_pct=0, subtotal returns 25.0 instead of 25, which breaks the no-discount criterion and makes receipt(ITEMS) return "3 items, total 25.0" instead of "3 items, total 25" (agent-c's criterion, 
- haiku `bug-float-vague` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` instead of the required `"3 items, total 25"`, because agent-b's `subtotal` now returns a float (`gross * (1 - 0/100)`) for undiscounted carts. This also b
- haiku `bug-rename` reject: REJECT agent-a: `receipt` in cart.py:15 still calls the removed `total()`, so the rename is incomplete and `receipt` raises NameError; once fixed to `subtotal`, agent-b's float return also yields "total 25.0", failing ag
- haiku `bug-rename` reject: REJECT agent-a: `receipt` in cart.py:16 still calls the removed `total`, which raises NameError and fails test_receipt; agent-b's subtotal also returns float 25.0 for undiscounted calls, which would make receipt print "t
- haiku `bug-rename` reject: REJECT agent-a: `receipt` still calls the removed `total`, so it raises `NameError` and breaks `test_receipt`. This also violates agent-a's "nothing calls total" criterion. agent-b's default-path float return and agent-c
- haiku `bug-rename` reject: REJECT agent-a: `total` was renamed to `subtotal`, but `receipt` still calls `total(items)`, so `receipt(ITEMS)` raises NameError and `test_receipt` fails. This violates agent-a's criterion that nothing in the project ca
- haiku `bug-rename` reject: REJECT agent-c: `receipt` in cart.py:19 calls `total(items)`, which no longer exists after agent-a renamed it to `subtotal`. This raises NameError, so `receipt(ITEMS) == "3 items, total 25"` fails and `test_receipt` fail
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-alt` reject: REJECT agent-c: `receipt` formats the total with `:g`, which renders totals of 1,000,000 or more in scientific notation (e.g. `"1.23457e+06"`), so it does not return `"<count> items, total <total>"` as specified. agent-b
- haiku `ok-alt` reject: REJECT agent-c: `receipt` uses `:g`, which turns totals ≥ 1e6 into exponent notation (1000000 → "1e+06") and truncates to 6 significant digits, so it does not print `<total>` as specified; use an integer-if-whole, otherw
- haiku `ok-alt` reject: REJECT agent-c: receipt formats the total with `:g`, which mangles totals beyond six significant digits (e.g. 1234567 -> "1.23457e+06"); the criterion requires the plain total, so use `str()`/`f"{...}"` or a fixed roundi
- haiku `ok-v2` land: LAND
- haiku `ok-v2` land: LAND
- haiku `ok-v2` land: LAND
- haiku `ok-v2` reject: REJECT agent-b: no-discount calls to `subtotal` now return a float (`25.0`) instead of the original int (`25`), so "calls without a discount behave as before" is not met exactly; the change also breaks agent-a's "unchang
- haiku `ok-v2` reject: REJECT agent-b: the no-discount path of `subtotal` now returns a float (`25.0`) instead of the previous int (`25`), so "calls without a discount behave as before" is not met exactly. Fix by returning the gross sum direct
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt interpolates subtotal() directly, but subtotal now always returns a float (per agent-b's contract), so receipt(ITEMS) yields "3 items, total 25.0" instead of "3 items, total 25"; the value must be
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt prints subtotal() unformatted, and since subtotal now always returns a float (agent-b's contract), receipt(ITEMS) yields "3 items, total 25.0" instead of "3 items, total 25", so the criterion and 
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt() embeds subtotal() unformatted, which is now always float, so receipt(ITEMS) returns "3 items, total 25.0" instead of "3 items, total 25" and test_receipt fails.
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt() interpolates subtotal(), which is now always a float, so receipt(ITEMS) returns "3 items, total 25.0" instead of "3 items, total 25"; it needs to format the value (e.g. drop the trailing ".0" fo
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now always returns a float and receipt doesn't format it; the required "3 items, total 25" is not produced and test_receipt fails.
- sonnet `bug-float-vague` reject: REJECT agent-b: subtotal(ITEMS) now returns float 25.0 instead of int 25 because it always multiplies by (1 - discount_pct/100), so undiscounted calls no longer behave as before and receipt(ITEMS) yields "3 items, total 
- sonnet `bug-float-vague` reject: REJECT agent-b: subtotal(items) with the default discount_pct=0 now returns the float 25.0 instead of the int 25 (gross * (1 - 0/100)). This breaks "calls without a discount behave as before" and makes receipt(ITEMS) pro
- sonnet `bug-float-vague` reject: REJECT agent-b: subtotal(items) with the default discount_pct=0 now returns the float 25.0 instead of the int 25 because of `* (1 - 0/100)`, so calls without a discount no longer behave as before. This breaks receipt(ITE
- sonnet `bug-float-vague` reject: REJECT agent-b: subtotal(items) without a discount now returns float 25.0 instead of int 25, which breaks "calls without a discount behave as before" and makes receipt(ITEMS) return "3 items, total 25.0" instead of "3 it
- sonnet `bug-float-vague` reject: REJECT agent-b: the default no-discount call now returns float 25.0 instead of int 25, which breaks "calls without a discount behave as before" and makes receipt(ITEMS) yield "3 items, total 25.0" instead of "3 items, to
- sonnet `bug-rename` reject: REJECT agent-a: receipt still calls the removed `total(items)` (NameError), so the rename is incomplete; additionally, once fixed to `subtotal`, agent-b's float math makes receipt(ITEMS) yield "3 items, total 25.0", not 
- sonnet `bug-rename` reject: REJECT agent-a: receipt() in cart.py still calls the removed `total(items)` (NameError), violating "nothing in the project calls total any more"; additionally, once it calls subtotal, agent-b's float result renders "tota
- sonnet `bug-rename` reject: REJECT agent-a: receipt() still calls the removed `total(items)` (NameError), so the rename is incomplete and the done-criterion "nothing calls total" fails; additionally agent-b's subtotal returns 25.0 instead of 25 by 
- sonnet `bug-rename` reject: REJECT agent-a: rename incomplete — receipt() in cart.py still calls the removed `total(items)` (NameError), violating "nothing calls total any more"; also, once fixed to subtotal, receipt would yield "total 25.0" becaus
- sonnet `bug-rename` reject: REJECT agent-c: receipt() still calls the removed `total()`, so `receipt(ITEMS)` raises NameError (it must call `subtotal`). Separately, agent-b's `subtotal` returns `25.0` for no discount, so the receipt string would re
- sonnet `ok-alt` reject: REJECT agent-c: receipt formats the total with `:g`, which truncates to 6 significant digits and switches to scientific notation, so totals like 1234567 or 12345.67 are printed wrongly (`1.23457e+06`, `12345.7`). Only th
- sonnet `ok-alt` reject: REJECT agent-c: receipt formats the total with `:g`, which truncates to 6 significant digits and uses exponent notation (1234567 -> "1.23457e+06"), so it does not report the actual total except for small values like the 
- sonnet `ok-alt` reject: REJECT agent-c: receipt formats the total with `:g`, which truncates to 6 significant digits and uses exponent notation (1234567 -> "1.23457e+06", 12345.67 -> "12345.7"), so it does not report the actual total. The only 
- sonnet `ok-alt` reject: REJECT agent-c: receipt formats the total with `:g`, which truncates to 6 significant digits and uses exponent notation, so totals like 1234567 print as "1.23457e+06" instead of the actual total.
- sonnet `ok-alt` reject: REJECT agent-c: receipt formats the total with `:g`, which truncates to 6 significant digits and uses scientific notation (e.g. 1234567 -> "1.23457e+06"), so it doesn't return the actual total; format the number faithful
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
- sonnet `ok-v2` land: LAND
