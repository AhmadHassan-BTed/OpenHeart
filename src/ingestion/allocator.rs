use std::sync::atomic::{AtomicU32, Ordering};

/// Thread-safe monotonic token ID allocator.
pub struct TokenIdAllocator {
    counter: AtomicU32,
}

impl TokenIdAllocator {
    pub fn new() -> Self {
        Self {
            counter: AtomicU32::new(0),
        }
    }

    pub fn current(&self) -> u32 {
        self.counter.load(Ordering::SeqCst)
    }

    pub fn next_id(&self) -> u32 {
        self.counter.fetch_add(1, Ordering::SeqCst)
    }
}

impl Default for TokenIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocator_monotonicity() {
        let alloc = TokenIdAllocator::new();
        assert_eq!(alloc.current(), 0);
        assert_eq!(alloc.next_id(), 0);
        assert_eq!(alloc.next_id(), 1);
        assert_eq!(alloc.next_id(), 2);
        assert_eq!(alloc.current(), 3);
    }
}
