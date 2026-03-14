/// Price tick spacing for horizontal grid lines
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceTickSpacing {
    pub step: f32,
    pub decimals: u32,
}

impl PriceTickSpacing {
    pub fn new(step: f32, decimals: u32) -> Self {
        Self { step, decimals }
    }

    /// Generate tick values for the given price range
    pub fn generate_ticks(&self, min_price: f32, max_price: f32) -> Vec<f32> {
        if !min_price.is_finite() || !max_price.is_finite() || max_price <= min_price {
            return Vec::new();
        }

        if self.step <= 0.0 || !self.step.is_finite() {
            return Vec::new();
        }

        let mut ticks = Vec::new();

        // Align first tick to a multiple of step
        let first_tick = (min_price / self.step).ceil() * self.step;

        let mut current = first_tick;
        let mut guard = 0;
        const MAX_TICKS: usize = 100;

        while current <= max_price && guard < MAX_TICKS {
            ticks.push(current);
            current += self.step;
            guard += 1;
        }

        ticks
    }

    /// Format a price value using the appropriate decimal places
    pub fn format_price(&self, price: f32) -> String {
        format!("{:.decimals$}", price, decimals = self.decimals as usize)
    }
}

/// Select appropriate price spacing for the given price span
pub fn select_price_spacing(
    span: f32,
    prev: Option<PriceTickSpacing>,
    min_ticks: usize,
    max_ticks: usize,
) -> PriceTickSpacing {
    // Hysteresis: if previous spacing still valid, keep it
    if let Some(prev_spacing) = prev {
        if prev_spacing.step > 0.0 {
            let count = (span / prev_spacing.step).round() as usize + 1;
            if count >= min_ticks && count <= max_ticks {
                return prev_spacing;
            }
        }
    }

    let desired = ((min_ticks + max_ticks) / 2) as f32;
    let mut step = nice_number(span / desired.max(1.0));

    loop {
        if step <= f32::EPSILON || !step.is_finite() {
            step = span.max(1.0);
            break;
        }

        let count = (span / step).ceil() as usize + 1;
        if count > max_ticks {
            step *= 2.0;
            continue;
        }

        if count < min_ticks {
            step *= 0.5;
            continue;
        }

        break;
    }

    PriceTickSpacing::new(step, decimal_places(step))
}

/// Round a value to a "nice" number (1, 2, 2.5, 5, 10, etc.)
fn nice_number(value: f32) -> f32 {
    if !value.is_finite() || value == 0.0 {
        return 1.0;
    }

    let exponent = value.abs().log10().floor();
    let fraction = value / 10f32.powf(exponent);

    let nice_fraction = if fraction < 1.5 {
        1.0
    } else if fraction < 3.0 {
        2.0
    } else if fraction < 3.5 {
        2.5
    } else if fraction < 7.0 {
        5.0
    } else {
        10.0
    };

    nice_fraction * 10f32.powf(exponent)
}

/// Determine number of decimal places needed for a given step size
pub fn decimal_places(step: f32) -> u32 {
    if step == 0.0 || !step.is_finite() {
        return 0;
    }

    let mut decimals = 0;
    let mut scaled = step;
    while decimals < 8 {
        let rounded = scaled.round();
        if (scaled - rounded).abs() < 1e-4 {
            return decimals;
        }
        scaled *= 10.0;
        decimals += 1;
    }
    decimals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_reasonable_step() {
        let span = 100.0;
        let spacing = select_price_spacing(span, None, 4, 8);
        assert!(spacing.step > 0.0);
        assert!(spacing.decimals <= 4);
    }

    #[test]
    fn generates_ticks_in_range() {
        let spacing = PriceTickSpacing::new(10.0, 0);
        let ticks = spacing.generate_ticks(15.0, 45.0);
        assert_eq!(ticks, vec![20.0, 30.0, 40.0]);
    }

    #[test]
    fn formats_price_correctly() {
        let spacing = PriceTickSpacing::new(0.25, 2);
        assert_eq!(spacing.format_price(123.456), "123.46");
    }
}
