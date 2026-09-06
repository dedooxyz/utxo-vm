#[derive(Debug, Clone)]
pub struct GasMeter {
    pub limit: u64,
    pub consumed: u64,
}

impl GasMeter {
    pub const BASE_TX_COST: u64 = 21_000;
    pub const HOST_CALL_COST: u64 = 500;
    pub const MEMORY_GROW_COST_PER_PAGE: u64 = 1_000;
    pub const EVENT_EMIT_COST_PER_BYTE: u64 = 10;
    pub const STATE_WRITE_COST_PER_BYTE: u64 = 50;

    pub fn new(limit: u64) -> Self {
        Self { limit, consumed: 0 }
    }

    pub fn consume(&mut self, amount: u64) -> Result<(), String> {
        if self.consumed.saturating_add(amount) > self.limit {
            Err(format!(
                "Out of Gas: limit {} exceeded (requested {}, already consumed {})",
                self.limit, amount, self.consumed
            ))
        } else {
            self.consumed += amount;
            Ok(())
        }
    }

    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.consumed)
    }
}
