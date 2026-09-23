"""How `print` shows a number, and how numbers cross the JSON protocol."""

import math


def describe(x):
    return f"result = {x:.2f}\n"


def decode(v):
    # JSON has no infinity or NaN, so those travel as strings ("inf", "-inf",
    # "NaN"); float() accepts both forms.
    return float(v)


def encode(x):
    return x if math.isfinite(x) else str(x)
