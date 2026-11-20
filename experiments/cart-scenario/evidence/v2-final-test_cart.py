import cart

ITEMS = [{"price": 10, "qty": 2}, {"price": 5, "qty": 1}]


def test_item_count():
    assert cart.item_count(ITEMS) == 3


def test_subtotal():
    assert cart.subtotal(ITEMS) == 25


def test_subtotal_with_discount():
    assert cart.subtotal(ITEMS, discount_pct=20) == 20


def test_receipt():
    assert cart.receipt(ITEMS) == "3 items, total 25"
