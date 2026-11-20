import cart

ITEMS = [{"price": 10, "qty": 2}, {"price": 5, "qty": 1}]


def test_item_count():
    assert cart.item_count(ITEMS) == 3
