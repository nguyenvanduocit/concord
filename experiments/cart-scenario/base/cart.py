"""Shopping cart."""


def total(items):
    """Sum of price * qty for every item."""
    return sum(i["price"] * i["qty"] for i in items)


def item_count(items):
    return sum(i["qty"] for i in items)
