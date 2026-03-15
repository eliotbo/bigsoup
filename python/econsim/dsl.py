"""Strategy DSL for econsim.

Builds a Python expression tree that compiles to a C float expression,
which is substituted into the CUDA kernel template.

Available named inputs:
  Param:    aggression, mean_reversion, trend_follow, noise_scale,
            ema_alpha, fair_value_lr, position_limit, risk_aversion, curvature, midpoint
  State:    fair_value_estimate, ema, prev_mid
  BboField: mid, spread
  Noise():  pre-computed LCG noise (scaled by noise_scale * spread)
  Position(): current agent position (f32)
  FairValue(): exogenous fundamental value

Example (verbose):
    mr   = Param("mean_reversion") * (State("fair_value_estimate") - BboField("mid"))
    tf   = Param("trend_follow")   * (BboField("mid") - State("ema"))
    expr = signal(mr + tf + Noise())
    c_str = compile(expr)  # pass to PySimulation.set_strategy()

Example (ergonomic):
    from econsim.dsl import p, s, bbo, noise, pos, exp, abs, signal, compile

    raw = p.mean_reversion * (s.fair_value_estimate - bbo.mid) \
        + p.trend_follow * (bbo.mid - s.ema) \
        + noise \
        + (-p.risk_aversion * pos)
    desirability = 1.0 / (1.0 + exp(p.curvature * (abs(pos) - p.midpoint)))
    c_str = compile(signal(raw * desirability))
"""


def _wrap(x):
    """Wrap plain numbers as Const nodes."""
    if isinstance(x, Expr):
        return x
    if isinstance(x, (int, float)):
        return Const(x)
    raise TypeError(f"Cannot convert {type(x).__name__} to Expr")


class Expr:
    """Base class for all DSL expression nodes."""

    def __add__(self, other):
        return Add(self, _wrap(other))

    def __radd__(self, other):
        return Add(_wrap(other), self)

    def __sub__(self, other):
        return Sub(self, _wrap(other))

    def __rsub__(self, other):
        return Sub(_wrap(other), self)

    def __mul__(self, other):
        return Mul(self, _wrap(other))

    def __rmul__(self, other):
        return Mul(_wrap(other), self)

    def __truediv__(self, other):
        return Div(self, _wrap(other))

    def __rtruediv__(self, other):
        return Div(_wrap(other), self)

    def __neg__(self):
        return Neg(self)

    def __abs__(self):
        return Abs(self)

    def to_c(self):
        raise NotImplementedError


# --- Leaf nodes ---

class Const(Expr):
    """Literal float constant."""

    def __init__(self, value):
        self.value = float(value)

    def to_c(self):
        s = repr(self.value)
        return s + "f"


class Param(Expr):
    """Reference a strategy parameter by name (K=10 layout)."""

    VALID = frozenset({
        "aggression", "mean_reversion", "trend_follow", "noise_scale",
        "ema_alpha", "fair_value_lr", "position_limit", "risk_aversion",
        "curvature", "midpoint",
    })

    def __init__(self, name):
        if name not in self.VALID:
            raise ValueError(f"Unknown param: {name!r}. Valid: {sorted(self.VALID)}")
        self.name = name

    def to_c(self):
        return self.name


class State(Expr):
    """Reference an internal state variable (M=4 layout)."""

    VALID = frozenset({"fair_value_estimate", "ema", "prev_mid"})

    def __init__(self, name):
        if name not in self.VALID:
            raise ValueError(f"Unknown state: {name!r}. Valid: {sorted(self.VALID)}")
        self.name = name

    def to_c(self):
        return self.name


class BboField(Expr):
    """Reference a BBO-derived field."""

    VALID = frozenset({"mid", "spread"})

    def __init__(self, name):
        if name not in self.VALID:
            raise ValueError(f"Unknown BBO field: {name!r}. Valid: {sorted(self.VALID)}")
        self.name = name

    def to_c(self):
        return self.name


class Noise(Expr):
    """Pre-computed LCG noise (already scaled by noise_scale * spread)."""

    def to_c(self):
        return "noise"


class Position(Expr):
    """Current agent position (f32)."""

    def to_c(self):
        return "pos"


class FairValue(Expr):
    """Exogenous fundamental value."""

    def to_c(self):
        return "fair_value"


# --- Binary operators ---

class Add(Expr):
    def __init__(self, left, right):
        self.left = left
        self.right = right

    def to_c(self):
        return f"({self.left.to_c()} + {self.right.to_c()})"


class Sub(Expr):
    def __init__(self, left, right):
        self.left = left
        self.right = right

    def to_c(self):
        return f"({self.left.to_c()} - {self.right.to_c()})"


class Mul(Expr):
    def __init__(self, left, right):
        self.left = left
        self.right = right

    def to_c(self):
        return f"({self.left.to_c()} * {self.right.to_c()})"


class Div(Expr):
    def __init__(self, left, right):
        self.left = left
        self.right = right

    def to_c(self):
        return f"({self.left.to_c()} / {self.right.to_c()})"


# --- Unary operators ---

class Neg(Expr):
    def __init__(self, expr):
        self.expr = expr

    def to_c(self):
        return f"(-({self.expr.to_c()}))"


class Abs(Expr):
    def __init__(self, expr):
        self.expr = expr

    def to_c(self):
        return f"fabsf({self.expr.to_c()})"


class Exp(Expr):
    """Exponential: expf(expr) in CUDA C."""

    def __init__(self, expr):
        self.expr = _wrap(expr)

    def to_c(self):
        return f"expf({self.expr.to_c()})"


# --- Clamp ---

class Clamp(Expr):
    """Clamp expression between lo and hi: fminf(fmaxf(expr, lo), hi)."""

    def __init__(self, expr, lo, hi):
        self.expr = _wrap(expr)
        self.lo = _wrap(lo)
        self.hi = _wrap(hi)

    def to_c(self):
        return f"fminf(fmaxf({self.expr.to_c()}, {self.lo.to_c()}), {self.hi.to_c()})"


# --- Signal builder and compiler ---

class Signal(Expr):
    """Marker wrapping a complete signal expression."""

    def __init__(self, expr):
        self.expr = _wrap(expr)

    def to_c(self):
        return self.expr.to_c()


def signal(expr):
    """Wrap an expression as a complete signal definition."""
    return Signal(expr)


def compile(expr):
    """Compile an expression tree to a C float expression string.

    The returned string is suitable for passing to PySimulation.set_strategy().
    """
    return expr.to_c()


# --- Ergonomic frontend sugar ---

class _Namespace:
    """Attribute-access namespace that returns DSL nodes.

    Example: p.mean_reversion -> Param("mean_reversion")
    """

    def __init__(self, cls):
        self._cls = cls

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        return self._cls(name)

    def __repr__(self):
        return f"_Namespace({self._cls.__name__})"


p = _Namespace(Param)
s = _Namespace(State)
bbo = _Namespace(BboField)

noise = Noise()
pos = Position()
fair_value = FairValue()

exp = Exp


def abs(expr):
    """DSL abs — returns Abs node. Python's builtin abs() also works on Expr."""
    return Abs(_wrap(expr))
