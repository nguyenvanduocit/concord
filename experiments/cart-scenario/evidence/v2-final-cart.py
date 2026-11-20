"""Shopping cart."""


def subtotal(items, discount_pct=0):
    """Sum of price * qty over items, less discount_pct percent (0-100)."""
    gross = sum(i["price"] * i["qty"] for i in items)
    return gross * (1 - discount_pct / 100)


def item_count(items):
    return sum(i["qty"] for i in items)


def receipt(items):
    """One-line summary: item count and cart total."""
    amount = subtotal(items)
    shown = int(amount) if amount == int(amount) else amount
    return f"{item_count(items)} items, total {shown}"
