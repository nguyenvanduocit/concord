# agent-review results

5 runs per case; rule in README.md.


## haiku

| Case | Expected | land | reject | unparsed | Pass |
|---|---|---|---|---|---|
| `bug-float-vague` | reject | 0 | 5 | 0 | yes |
| `bug-float-precise` | reject | 0 | 5 | 0 | yes |
| `bug-rename` | reject | 0 | 5 | 0 | yes |
| `ok-v2` | land | 5 | 0 | 0 | yes |
| `bug-latent-g` | reject | 3 | 2 | 0 | yes |
| `ok-alt` | land | 5 | 0 | 0 | yes |

Design survives for haiku: **yes**

## sonnet

| Case | Expected | land | reject | unparsed | Pass |
|---|---|---|---|---|---|
| `bug-float-vague` | reject | 0 | 5 | 0 | yes |
| `bug-float-precise` | reject | 0 | 5 | 0 | yes |
| `bug-rename` | reject | 0 | 5 | 0 | yes |
| `ok-v2` | land | 5 | 0 | 0 | yes |
| `bug-latent-g` | reject | 5 | 0 | 0 | yes |
| `ok-alt` | land | 5 | 0 | 0 | yes |

Design survives for sonnet: **yes**

## Final lines

- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns "3 items, total 25.0" because `subtotal` now returns a float (total#2) and `receipt` prints it unformatted, so `test_receipt` fails and the done-criterion is not met.
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because `subtotal` now returns a float (total#2) and `receipt` does not format it, so `test_receipt` fails against the required `"3 items, total 25"`.
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt` interpolates the now-float `subtotal` unformatted, so `receipt(ITEMS)` returns "3 items, total 25.0" instead of "3 items, total 25" and `test_receipt` fails; the total#2 contract change requires
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt` prints the float returned by `subtotal` unformatted, so `receipt(ITEMS)` yields `"3 items, total 25.0"` instead of the required `"3 items, total 25"`. It does not adapt to total#2's float contra
- haiku `bug-float-precise` reject: REJECT agent-c: `receipt` prints the float returned by `subtotal` without formatting, so `receipt(ITEMS)` yields `"3 items, total 25.0"` instead of the required `"3 items, total 25"`. This violates agent-c's done-criteri
- haiku `bug-float-vague` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because `subtotal` now returns a float via `gross * (1 - discount_pct / 100)`; the required output is `"3 items, total 25"`, so `test_receipt` fails and th
- haiku `bug-float-vague` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because `subtotal` now yields a float (`25 * (1 - 0/100)` = `25.0`), so the required `"3 items, total 25"` is not produced and `test_receipt` fails.
- haiku `bug-float-vague` reject: REJECT agent-c: `receipt(ITEMS)` returns `"3 items, total 25.0"` because the default-discount path yields the float `25.0`, so the required `"3 items, total 25"` and `test_receipt` both fail.
- haiku `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now returns a float (gross * (1 - 0 / 100) = 25.0), but the criterion requires "3 items, total 25", and test_receipt asserts the latter.
- haiku `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal's default path yields a float (1 - 0/100 = 1.0), so test_receipt and its done-criterion "3 items, total 25" fail. Return an int when there is 
- haiku `bug-latent-g` land: LAND
- haiku `bug-latent-g` land: LAND
- haiku `bug-latent-g` land: LAND
- haiku `bug-latent-g` reject: REJECT agent-c: `receipt` formats the total with `:g`, which keeps only six significant digits, so a total like 1234567 is reported as `1.23457e+06`. Use a format that prints whole-number totals in full (for example, pri
- haiku `bug-latent-g` reject: REJECT agent-c: `receipt` formats the total with `:g`, which produces exponent notation and 6-significant-digit rounding for large totals (e.g., 1234567 becomes "1.23457e+06"), so it does not return `"<count> items, tota
- haiku `bug-rename` reject: REJECT agent-c: `receipt` calls `total(items)`, which agent-a renamed to `subtotal`; `receipt(ITEMS)` raises NameError, so its criterion `receipt(ITEMS) == "3 items, total 25"` is not met and `test_receipt` will fail.
- haiku `bug-rename` reject: REJECT agent-c: `receipt` calls `total(items)`, which agent-a renamed to `subtotal`; the call must be changed to `subtotal(items)`, otherwise `receipt` raises NameError and `test_receipt` fails.
- haiku `bug-rename` reject: REJECT agent-c: `receipt` calls `total(items)`, which no longer exists after agent-a renamed it to `subtotal`. `receipt(ITEMS)` raises `NameError` and `test_receipt` fails. It should call `subtotal(items)`.
- haiku `bug-rename` reject: REJECT agent-c: `receipt` still calls `total(items)`, which agent-a renamed to `subtotal`, so `receipt` raises NameError and `receipt(ITEMS)` does not equal `"3 items, total 25"`; it should call `subtotal(items)`.
- haiku `bug-rename` reject: REJECT agent-c: `receipt` still calls the removed `total()` (renamed to `subtotal` by agent-a), which raises `NameError`. `test_receipt` fails, and receipt(ITEMS) does not meet its criterion. Switching to `subtotal(items
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-alt` land: LAND
- haiku `ok-v2` land: LAND
- haiku `ok-v2` land: LAND
- haiku `ok-v2` land: LAND
- haiku `ok-v2` land: LAND
- haiku `ok-v2` land: LAND
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt interpolates subtotal() directly, but subtotal now always returns a float, so receipt(ITEMS) yields "3 items, total 25.0" instead of "3 items, total 25"; receipt must format the value per the chan
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt interpolates subtotal() raw, but subtotal now always returns a float (agent-b's contract change), so receipt(ITEMS) yields "3 items, total 25.0" instead of "3 items, total 25" and test_receipt fai
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt() embeds subtotal() unformatted; subtotal now always returns a float, so it yields "3 items, total 25.0" instead of "3 items, total 25", failing its criterion and test_receipt.
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt() interpolates subtotal() directly, and subtotal now always returns a float, so receipt(ITEMS) yields "3 items, total 25.0" instead of "3 items, total 25". The caller was not adapted to the change
- sonnet `bug-float-precise` reject: REJECT agent-c: receipt() interpolates subtotal() unformatted, but subtotal now always returns a float, so receipt(ITEMS) yields "3 items, total 25.0" instead of "3 items, total 25" and test_receipt fails.
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now always returns a float, so the criterion and test_receipt fail
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" because subtotal now returns a float (25 * (1 - 0/100) = 25.0), so the criterion "3 items, total 25" and test_receipt fail; receipt did not adapt to the new fl
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" instead of "3 items, total 25", because subtotal now returns a float (25 * 1.0) and receipt does not format it; test_receipt fails.
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0" rather than "3 items, total 25", because subtotal now always returns a float (1 - 0/100 is 1.0) and receipt formats it unadapted; test_receipt fails.
- sonnet `bug-float-vague` reject: REJECT agent-c: receipt(ITEMS) returns "3 items, total 25.0", not "3 items, total 25", because subtotal now always returns a float (25 * (1 - 0/100) = 25.0) and receipt formats it without adapting.
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-latent-g` land: LAND
- sonnet `bug-rename` reject: REJECT agent-c: receipt still calls the removed `total(items)` (NameError after agent-a's rename to `subtotal`), and even if renamed, agent-b's float result would make it return "3 items, total 25.0" instead of the requi
- sonnet `bug-rename` reject: REJECT agent-c: receipt() calls the removed `total(items)` (NameError after the rename to `subtotal`), and even with the rename, subtotal now returns 25.0 so the output would be "total 25.0" rather than "total 25"
- sonnet `bug-rename` reject: REJECT agent-c: receipt() still calls the removed `total(items)`, raising NameError (it must call `subtotal`); and even once fixed, subtotal now returns a float (25.0), so the output would be "3 items, total 25.0" rather
- sonnet `bug-rename` reject: REJECT agent-c: receipt() still calls the removed total() (NameError); it must call subtotal(items), and the result is now a float (25.0), so the output "3 items, total 25.0" would not match the required "3 items, total 
- sonnet `bug-rename` reject: REJECT agent-c: receipt() still calls the removed total() (NameError, caller not adapted to the subtotal rename); and after switching to subtotal(), the 0-discount result is the float 25.0, so the output would be "3 item
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
